# -*- coding: utf-8 -*-
"""SQLite 资源库：照片元数据、标签、事件、重复分组、整理日志。"""
from __future__ import annotations

import os
import sqlite3
import threading
import time

from . import config
from .util import iso, parse_iso

SCHEMA = """
PRAGMA journal_mode=WAL;
CREATE TABLE IF NOT EXISTS photos (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT UNIQUE NOT NULL,
    folder TEXT,
    name TEXT,
    ext TEXT,
    size INTEGER,
    mtime REAL,
    ctime REAL,
    sha1 TEXT,
    width INTEGER, height INTEGER, megapixels REAL, aspect REAL,
    dt TEXT,            -- 拍摄时间 ISO
    dt_source TEXT,     -- exif / filename / fs / manual
    dt_precision TEXT,
    year INTEGER, month INTEGER, day INTEGER, hour INTEGER,
    camera_make TEXT, camera_model TEXT, software TEXT,
    lat REAL, lon REAL, place TEXT,
    blur REAL, brightness REAL, contrast REAL, saturation REAL,
    hue REAL, sepia REAL, color REAL, skin REAL, noise REAL, sharpness REAL,
    dark_ratio REAL, white_ratio REAL,
    a_hash TEXT, d_hash TEXT, p_hash TEXT,
    flags TEXT, has_people INTEGER, is_mono INTEGER, is_sepia INTEGER,
    age_score REAL, quality_score REAL,
    thumb TEXT, analyzed_at REAL, missing INTEGER DEFAULT 0,
    dup_group INTEGER, is_best INTEGER DEFAULT 0, note TEXT,
    favorite INTEGER DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_photos_dt ON photos(dt);
CREATE INDEX IF NOT EXISTS idx_photos_folder ON photos(folder);
CREATE INDEX IF NOT EXISTS idx_photos_phash ON photos(p_hash);
CREATE INDEX IF NOT EXISTS idx_photos_sha1 ON photos(sha1);
CREATE INDEX IF NOT EXISTS idx_photos_dup ON photos(dup_group);

CREATE TABLE IF NOT EXISTS roots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT UNIQUE NOT NULL,
    added_at REAL,
    last_scan REAL,
    recursive INTEGER DEFAULT 1
);

CREATE TABLE IF NOT EXISTS tags (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT UNIQUE NOT NULL,
    kind TEXT DEFAULT 'user',   -- user / auto / event / place / time / quality
    color TEXT DEFAULT ''
);

CREATE TABLE IF NOT EXISTS photo_tags (
    photo_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    source TEXT DEFAULT 'user',
    PRIMARY KEY (photo_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_pt_tag ON photo_tags(tag_id);

CREATE TABLE IF NOT EXISTS events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    title TEXT,
    start_dt TEXT,
    end_dt TEXT,
    lat REAL, lon REAL,
    count INTEGER DEFAULT 0,
    cover_id INTEGER
);

CREATE TABLE IF NOT EXISTS dup_groups (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT DEFAULT 'similar',  -- exact / similar
    count INTEGER DEFAULT 0,
    best_id INTEGER,
    keep_bytes INTEGER DEFAULT 0,
    saved_bytes INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS ops_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts REAL,
    kind TEXT,
    src TEXT,
    dst TEXT,
    detail TEXT,
    undone INTEGER DEFAULT 0
);

CREATE TABLE IF NOT EXISTS meta (k TEXT PRIMARY KEY, v TEXT);
"""

PHOTO_COLS = [
    "path", "folder", "name", "ext", "size", "mtime", "ctime", "sha1",
    "width", "height", "megapixels", "aspect", "dt", "dt_source", "dt_precision",
    "year", "month", "day", "hour", "camera_make", "camera_model", "software",
    "lat", "lon", "place", "blur", "brightness", "contrast", "saturation", "hue",
    "sepia", "color", "skin", "noise", "sharpness", "dark_ratio", "white_ratio",
    "a_hash", "d_hash", "p_hash", "flags", "has_people", "is_mono", "is_sepia",
    "age_score", "quality_score", "thumb", "analyzed_at", "missing", "dup_group",
    "is_best", "note", "favorite",
]


class Library:
    def __init__(self, path=None):
        self.path = path or config.db_path()
        os.makedirs(os.path.dirname(self.path) or ".", exist_ok=True)
        self._local = threading.local()
        self.conn = self._connect()
        self.conn.executescript(SCHEMA)
        self.conn.commit()
        self._migrate()

    def _migrate(self):
        """给旧库补新列并补索引，老版本的库没有 favorite 列。"""
        try:
            cols = {r["name"] for r in self.q("PRAGMA table_info(photos)")}
            if "favorite" not in cols:
                self.ex("ALTER TABLE photos ADD COLUMN favorite INTEGER DEFAULT 0")
            if "place" not in cols:
                self.ex("ALTER TABLE photos ADD COLUMN place TEXT")
            # 索引必须在列存在后再建，否则旧库会直接报错
            if "favorite" in {r["name"] for r in self.q("PRAGMA table_info(photos)")}:
                self.ex("CREATE INDEX IF NOT EXISTS idx_photos_fav ON photos(favorite)")
        except Exception:
            pass

    def _connect(self):
        c = sqlite3.connect(self.path, timeout=30, check_same_thread=False)
        c.row_factory = sqlite3.Row
        try:
            c.execute("PRAGMA synchronous=NORMAL")
            c.execute("PRAGMA cache_size=-40000")
            c.execute("PRAGMA temp_store=MEMORY")
        except Exception:
            pass
        return c

    # -------------------------------------------------------------- 基础
    def close(self):
        try:
            self.conn.close()
        except Exception:
            pass

    def q(self, sql, args=()):
        return self.conn.execute(sql, args).fetchall()

    def q1(self, sql, args=()):
        r = self.conn.execute(sql, args).fetchone()
        return r

    def ex(self, sql, args=()):
        cur = self.conn.execute(sql, args)
        self.conn.commit()
        return cur

    def get_meta(self, k, default=None):
        r = self.q1("SELECT v FROM meta WHERE k=?", (k,))
        return r["v"] if r else default

    def set_meta(self, k, v):
        self.ex("INSERT INTO meta(k,v) VALUES(?,?) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
                (k, str(v)))

    # -------------------------------------------------------------- 根目录
    def add_root(self, path, recursive=1):
        path = os.path.abspath(path)
        self.ex("INSERT OR IGNORE INTO roots(path,added_at,recursive) VALUES(?,?,?)",
                (path, time.time(), int(recursive)))
        return path

    def remove_root(self, path):
        self.ex("DELETE FROM roots WHERE path=?", (os.path.abspath(path),))

    def roots(self):
        return [r["path"] for r in self.q("SELECT path FROM roots ORDER BY id")]

    def touch_root(self, path):
        self.ex("UPDATE roots SET last_scan=? WHERE path=?", (time.time(), os.path.abspath(path)))

    # -------------------------------------------------------------- 照片
    def upsert_photo(self, d: dict) -> int:
        """按 path 插入或更新；返回 photo id。"""
        path = d["path"]
        row = self.q1("SELECT id FROM photos WHERE path=?", (path,))
        vals = {k: d.get(k) for k in PHOTO_COLS if k in d}
        if row:
            pid = row["id"]
            if vals:
                sets = ", ".join("%s=?" % k for k in vals)
                self.ex("UPDATE photos SET %s WHERE id=?" % sets, list(vals.values()) + [pid])
            return pid
        cols = ["path"] + [k for k in vals if k != "path"]
        sql = "INSERT INTO photos(%s) VALUES(%s)" % (",".join(cols), ",".join("?" * len(cols)))
        args = [vals["path"]] + [vals[k] for k in cols[1:]]
        cur = self.ex(sql, args)
        return cur.lastrowid

    def photo_by_path(self, path):
        return self.q1("SELECT * FROM photos WHERE path=?", (path,))

    def photo(self, pid):
        return self.q1("SELECT * FROM photos WHERE id=?", (pid,))

    def all_photos(self, where="", args=(), order="dt IS NULL, dt, id"):
        sql = "SELECT * FROM photos"
        if where:
            sql += " WHERE " + where
        sql += " ORDER BY " + order
        return self.q(sql, args)

    def photo_ids(self, where="", args=()):
        sql = "SELECT id FROM photos"
        if where:
            sql += " WHERE " + where
        return [r["id"] for r in self.q(sql, args)]

    def count(self, where="", args=()):
        sql = "SELECT COUNT(*) c FROM photos"
        if where:
            sql += " WHERE " + where
        return self.q1(sql, args)["c"]

    def stats(self) -> dict:
        c = self.conn
        s = {}
        s["total"] = self.q1("SELECT COUNT(*) c FROM photos")["c"]
        s["analyzed"] = self.q1("SELECT COUNT(*) c FROM photos WHERE analyzed_at IS NOT NULL")["c"]
        s["with_dt"] = self.q1("SELECT COUNT(*) c FROM photos WHERE dt IS NOT NULL")["c"]
        s["no_dt"] = s["total"] - s["with_dt"]
        s["dups"] = self.q1("SELECT COUNT(*) c FROM photos WHERE dup_group IS NOT NULL")["c"]
        s["missing"] = self.q1("SELECT COUNT(*) c FROM photos WHERE missing=1")["c"]
        s["bytes"] = self.q1("SELECT COALESCE(SUM(size),0) c FROM photos")["c"]
        s["years"] = [(r["year"], r["c"]) for r in self.q(
            "SELECT year, COUNT(*) c FROM photos WHERE year IS NOT NULL GROUP BY year ORDER BY year")]
        s["tags"] = self.q1("SELECT COUNT(*) c FROM tags")["c"]
        return s

    def mark_missing(self, pid, flag=1):
        self.ex("UPDATE photos SET missing=? WHERE id=?", (int(flag), pid))

    def delete_photo(self, pid):
        self.ex("DELETE FROM photo_tags WHERE photo_id=?", (pid,))
        self.ex("DELETE FROM photos WHERE id=?", (pid,))

    # -------------------------------------------------------------- 标签
    def tag_id(self, name, kind="user", create=True):
        name = (name or "").strip()
        if not name:
            return None
        r = self.q1("SELECT id FROM tags WHERE name=?", (name,))
        if r:
            return r["id"]
        if not create:
            return None
        cur = self.ex("INSERT INTO tags(name,kind) VALUES(?,?)", (name, kind))
        return cur.lastrowid

    def add_tag(self, pid, name, kind="user", source="user"):
        tid = self.tag_id(name, kind)
        if tid is None:
            return
        self.ex("INSERT OR IGNORE INTO photo_tags(photo_id,tag_id,source) VALUES(?,?,?)",
                (pid, tid, source))

    def add_tags_bulk(self, pairs, kind="user", source="user"):
        """pairs: [(photo_id, tag_name)]"""
        cache = {}
        rows = []
        for pid, name in pairs:
            name = (name or "").strip()
            if not name:
                continue
            tid = cache.get(name)
            if tid is None:
                tid = self.tag_id(name, kind)
                cache[name] = tid
            if tid:
                rows.append((pid, tid, source))
        if rows:
            self.conn.executemany(
                "INSERT OR IGNORE INTO photo_tags(photo_id,tag_id,source) VALUES(?,?,?)", rows)
            self.conn.commit()

    def remove_tag(self, pid, name):
        tid = self.tag_id(name, create=False)
        if tid:
            self.ex("DELETE FROM photo_tags WHERE photo_id=? AND tag_id=?", (pid, tid))

    def clear_auto_tags(self):
        self.ex("DELETE FROM photo_tags WHERE source='auto'")
        self.ex("DELETE FROM tags WHERE kind='auto' AND id NOT IN (SELECT tag_id FROM photo_tags)")

    def tags_of(self, pid):
        return [r["name"] for r in self.q(
            "SELECT t.name FROM tags t JOIN photo_tags pt ON pt.tag_id=t.id "
            "WHERE pt.photo_id=? ORDER BY t.name", (pid,))]

    def all_tags(self):
        return [(r["name"], r["kind"], r["c"]) for r in self.q(
            "SELECT t.name, t.kind, COUNT(pt.photo_id) c FROM tags t "
            "LEFT JOIN photo_tags pt ON pt.tag_id=t.id GROUP BY t.id "
            "ORDER BY t.kind, c DESC, t.name")]

    # -------------------------------------------------------------- 日志
    def log_op(self, kind, src, dst="", detail=""):
        cur = self.ex("INSERT INTO ops_log(ts,kind,src,dst,detail) VALUES(?,?,?,?,?)",
                      (time.time(), kind, src, dst, detail))
        return cur.lastrowid

    def recent_ops(self, limit=200):
        return self.q("SELECT * FROM ops_log ORDER BY id DESC LIMIT ?", (limit,))
