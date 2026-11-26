//! SQLite 资源库：照片元数据、标签、事件、重复分组、整理日志。
//!
//! Schema 与 Python 版 `opo/library.py` 逐字一致，
//! 目的是让现有用户的 `library.db` 能被直接打开，无需重新扫描。

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension, Row};

use crate::config;
use crate::util::unique_path;
/// 建表语句。列名、类型、索引名都必须与原版保持完全一致。
pub const SCHEMA: &str = r#"
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
    lat REAL, lon REAL, place TEXT, place_source TEXT,
    blur REAL, brightness REAL, contrast REAL, saturation REAL,
    hue REAL, sepia REAL, color REAL, skin REAL, noise REAL, sharpness REAL,
    dark_ratio REAL, white_ratio REAL,
    a_hash TEXT, d_hash TEXT, p_hash TEXT,
    flags TEXT, has_people INTEGER, is_mono INTEGER, is_sepia INTEGER,
    age_score REAL, quality_score REAL,
    thumb TEXT, analyzed_at REAL, missing INTEGER DEFAULT 0,
    dup_group INTEGER, is_best INTEGER DEFAULT 0, note TEXT,
    favorite INTEGER DEFAULT 0,
    -- photo / video。视频只做抽帧与时长，不做像素质量分析
    kind TEXT DEFAULT 'photo',
    duration REAL
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

CREATE TABLE IF NOT EXISTS event_members (
    event_id INTEGER NOT NULL,
    photo_id INTEGER NOT NULL UNIQUE,
    PRIMARY KEY (event_id, photo_id)
);
CREATE INDEX IF NOT EXISTS idx_em_photo ON event_members(photo_id);

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

"#;

/// 可写入 photos 表的列（不含自增主键）。
pub const PHOTO_COLS: &[&str] = &[
    "path",
    "folder",
    "name",
    "ext",
    "size",
    "mtime",
    "ctime",
    "sha1",
    "width",
    "height",
    "megapixels",
    "aspect",
    "dt",
    "dt_source",
    "dt_precision",
    "year",
    "month",
    "day",
    "hour",
    "camera_make",
    "camera_model",
    "software",
    "lat",
    "lon",
    "place",
    "place_source",
    "blur",
    "brightness",
    "contrast",
    "saturation",
    "hue",
    "sepia",
    "color",
    "skin",
    "noise",
    "sharpness",
    "dark_ratio",
    "white_ratio",
    "a_hash",
    "d_hash",
    "p_hash",
    "flags",
    "has_people",
    "is_mono",
    "is_sepia",
    "age_score",
    "quality_score",
    "thumb",
    "analyzed_at",
    "missing",
    "dup_group",
    "is_best",
    "note",
    "favorite",
    "kind",
    "duration",
];

/// 照片行的通用读取：按列名取值，缺失列返回空，便于兼容旧库。
#[derive(Debug, Clone, Default)]
pub struct PhotoRow {
    pub id: i64,
    pub path: String,
    pub folder: Option<String>,
    pub name: Option<String>,
    pub ext: Option<String>,
    pub size: Option<i64>,
    pub mtime: Option<f64>,
    pub ctime: Option<f64>,
    pub sha1: Option<String>,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub megapixels: Option<f64>,
    pub aspect: Option<f64>,
    pub dt: Option<String>,
    pub dt_source: Option<String>,
    pub dt_precision: Option<String>,
    pub year: Option<i64>,
    pub month: Option<i64>,
    pub day: Option<i64>,
    pub hour: Option<i64>,
    pub camera_make: Option<String>,
    pub camera_model: Option<String>,
    pub software: Option<String>,
    pub lat: Option<f64>,
    pub lon: Option<f64>,
    pub place: Option<String>,
    pub place_source: Option<String>,
    pub blur: Option<f64>,
    pub brightness: Option<f64>,
    pub contrast: Option<f64>,
    pub saturation: Option<f64>,
    pub hue: Option<f64>,
    pub sepia: Option<f64>,
    pub color: Option<f64>,
    pub skin: Option<f64>,
    pub noise: Option<f64>,
    pub sharpness: Option<f64>,
    pub dark_ratio: Option<f64>,
    pub white_ratio: Option<f64>,
    pub a_hash: Option<String>,
    pub d_hash: Option<String>,
    pub p_hash: Option<String>,
    pub flags: Option<String>,
    pub has_people: Option<i64>,
    pub is_mono: Option<i64>,
    pub is_sepia: Option<i64>,
    pub age_score: Option<f64>,
    pub quality_score: Option<f64>,
    pub thumb: Option<String>,
    pub analyzed_at: Option<f64>,
    pub missing: Option<i64>,
    pub dup_group: Option<i64>,
    pub is_best: Option<i64>,
    pub note: Option<String>,
    pub favorite: Option<i64>,
    /// `photo` 或 `video`
    pub kind: Option<String>,
    /// 视频时长（秒），照片为 NULL
    pub duration: Option<f64>,
}

impl PhotoRow {
    fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get("id")?,
            path: row.get::<_, Option<String>>("path")?.unwrap_or_default(),
            folder: row.get("folder").ok(),
            name: row.get("name").ok(),
            ext: row.get("ext").ok(),
            size: row.get("size").ok(),
            mtime: row.get("mtime").ok(),
            ctime: row.get("ctime").ok(),
            sha1: row.get("sha1").ok(),
            width: row.get("width").ok(),
            height: row.get("height").ok(),
            megapixels: row.get("megapixels").ok(),
            aspect: row.get("aspect").ok(),
            dt: row.get("dt").ok(),
            dt_source: row.get("dt_source").ok(),
            dt_precision: row.get("dt_precision").ok(),
            year: row.get("year").ok(),
            month: row.get("month").ok(),
            day: row.get("day").ok(),
            hour: row.get("hour").ok(),
            camera_make: row.get("camera_make").ok(),
            camera_model: row.get("camera_model").ok(),
            software: row.get("software").ok(),
            lat: row.get("lat").ok(),
            lon: row.get("lon").ok(),
            place: row.get("place").ok(),
            place_source: row.get("place_source").ok(),
            blur: row.get("blur").ok(),
            brightness: row.get("brightness").ok(),
            contrast: row.get("contrast").ok(),
            saturation: row.get("saturation").ok(),
            hue: row.get("hue").ok(),
            sepia: row.get("sepia").ok(),
            color: row.get("color").ok(),
            skin: row.get("skin").ok(),
            noise: row.get("noise").ok(),
            sharpness: row.get("sharpness").ok(),
            dark_ratio: row.get("dark_ratio").ok(),
            white_ratio: row.get("white_ratio").ok(),
            a_hash: row.get("a_hash").ok(),
            d_hash: row.get("d_hash").ok(),
            p_hash: row.get("p_hash").ok(),
            flags: row.get("flags").ok(),
            has_people: row.get("has_people").ok(),
            is_mono: row.get("is_mono").ok(),
            is_sepia: row.get("is_sepia").ok(),
            age_score: row.get("age_score").ok(),
            quality_score: row.get("quality_score").ok(),
            thumb: row.get("thumb").ok(),
            analyzed_at: row.get("analyzed_at").ok(),
            missing: row.get("missing").ok(),
            dup_group: row.get("dup_group").ok(),
            is_best: row.get("is_best").ok(),
            note: row.get("note").ok(),
            favorite: row.get("favorite").ok(),
            kind: row.get("kind").ok(),
            duration: row.get("duration").ok(),
        })
    }

    pub fn is_favorite(&self) -> bool {
        self.favorite.unwrap_or(0) != 0
    }

    pub fn is_video(&self) -> bool {
        self.kind.as_deref() == Some("video")
    }

    /// 视频时长的人类可读形式，如 `1:23`
    pub fn duration_text(&self) -> Option<String> {
        let d = self.duration?;
        if d <= 0.0 || !d.is_finite() {
            return None;
        }
        let total = d.round() as i64;
        let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
        Some(if h > 0 {
            format!("{h}:{m:02}:{s:02}")
        } else {
            format!("{m}:{s:02}")
        })
    }

    pub fn is_missing(&self) -> bool {
        self.missing.unwrap_or(0) != 0
    }

    /// 文件名（去掉目录），优先用库里的 name 列。
    pub fn file_name(&self) -> String {
        self.name
            .clone()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| {
                Path::new(&self.path)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("photo")
                    .to_string()
            })
    }
}

/// 统计信息，字段名与 Python 版 `Library.stats()` 对齐。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Stats {
    pub total: i64,
    pub analyzed: i64,
    pub with_dt: i64,
    pub no_dt: i64,
    pub dups: i64,
    pub missing: i64,
    pub bytes: i64,
    /// (年份, 数量)，按年份升序
    pub years: Vec<(i64, i64)>,
    pub tags: i64,
}

pub struct Library {
    pub path: PathBuf,
    conn: Connection,
}

impl Library {
    /// 打开默认库（`%LOCALAPPDATA%\OldPhotoOrganizer\library.db`）。
    pub fn open_default() -> anyhow::Result<Self> {
        Self::open(&config::db_path())
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                config::ensure_dir(parent.to_path_buf());
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch("PRAGMA cache_size=-40000;")?;
        conn.execute_batch("PRAGMA temp_store=MEMORY;")?;
        conn.execute_batch(SCHEMA)?;
        let mut lib = Self {
            path: path.to_path_buf(),
            conn,
        };
        lib.migrate();
        // 修掉 v1.1 早期版本用 canonicalize 留下的 `\\?\` 前缀
        let (r, p) = lib.strip_unc_prefixes();
        if r > 0 || p > 0 {
            let _ = lib.set_meta(
                "unc_prefix_stripped",
                &format!("roots={r},photos={p}"),
            );
        }
        Ok(lib)
    }

    /// 内存库，仅用于测试。
    pub fn open_memory() -> anyhow::Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        let mut lib = Self {
            path: PathBuf::from(":memory:"),
            conn,
        };
        lib.migrate();
        Ok(lib)
    }

    /// 给旧库补新列并补索引。旧版本没有 favorite 列。
    fn migrate(&mut self) {
        let cols = self.photo_columns();
        if !cols.contains(&"favorite".to_string()) {
            let _ = self
                .conn
                .execute("ALTER TABLE photos ADD COLUMN favorite INTEGER DEFAULT 0", []);
        }
        if !cols.contains(&"place".to_string()) {
            let _ = self
                .conn
                .execute("ALTER TABLE photos ADD COLUMN place TEXT", []);
        }
        // 视频支持：老库没有 kind / duration 列
        if !cols.contains(&"kind".to_string()) {
            let _ = self.conn.execute(
                "ALTER TABLE photos ADD COLUMN kind TEXT DEFAULT 'photo'",
                [],
            );
        }
        if !cols.contains(&"duration".to_string()) {
            let _ = self
                .conn
                .execute("ALTER TABLE photos ADD COLUMN duration REAL", []);
        }
        if !cols.contains(&"place_source".to_string()) {
            let _ = self.conn.execute(
                "ALTER TABLE photos ADD COLUMN place_source TEXT",
                [],
            );
        }
        // 索引必须在列存在之后再建
        let _ = self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_photos_fav ON photos(favorite)",
            [],
        );
        let _ = self.conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_photos_kind ON photos(kind)",
            [],
        );
    }

    pub fn photo_columns(&self) -> Vec<String> {
        let mut stmt = match self.conn.prepare("PRAGMA table_info(photos)") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| r.get::<_, String>(1));
        match rows {
            Ok(it) => it.flatten().collect(),
            Err(_) => Vec::new(),
        }
    }

    // -------------------------------------------------------------- 基础

    pub fn conn(&self) -> &Connection {
        &self.conn
    }


    /// 取单行（任意查询，返回第一列）
    pub fn query_count(&self, sql: &str, args: &[&dyn rusqlite::ToSql]) -> i64 {
        self.conn
            .query_row(sql, args, |r| r.get::<_, i64>(0))
            .unwrap_or(0)
    }

    pub fn get_meta(&self, k: &str) -> Option<String> {
        self.conn
            .query_row("SELECT v FROM meta WHERE k=?", params![k], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
    }

    pub fn set_meta(&self, k: &str, v: &str) -> rusqlite::Result<()> {
        self.conn.execute(
            "INSERT INTO meta(k,v) VALUES(?,?) ON CONFLICT(k) DO UPDATE SET v=excluded.v",
            params![k, v],
        )?;
        Ok(())
    }

    // -------------------------------------------------------------- 根目录

    pub fn add_root(&self, path: &Path) -> String {
        let abs = absolutize(path);
        let now = now_ts();
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO roots(path,added_at,recursive) VALUES(?,?,?)",
            params![abs, now, 1],
        );
        abs
    }

    pub fn remove_root(&self, path: &Path) {
        let abs = absolutize(path);
        let _ = self
            .conn
            .execute("DELETE FROM roots WHERE path=?", params![abs]);
    }

    pub fn roots(&self) -> Vec<String> {
        let Ok(mut stmt) = self.conn.prepare("SELECT path FROM roots ORDER BY id") else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
            return Vec::new();
        };
        it.flatten().collect()
    }


    // -------------------------------------------------------------- 照片

    pub fn photo_by_path(&self, path: &str) -> Option<PhotoRow> {
        self.conn
            .query_row("SELECT * FROM photos WHERE path=?", params![path], |r| {
                PhotoRow::from_row(r)
            })
            .optional()
            .ok()
            .flatten()
    }

    pub fn photo(&self, id: i64) -> Option<PhotoRow> {
        self.conn
            .query_row("SELECT * FROM photos WHERE id=?", params![id], |r| {
                PhotoRow::from_row(r)
            })
            .optional()
            .ok()
            .flatten()
    }

    pub fn all_photos(&self, where_clause: &str, order: &str) -> Vec<PhotoRow> {
        let mut sql = String::from("SELECT * FROM photos");
        if !where_clause.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(where_clause);
        }
        sql.push_str(" ORDER BY ");
        sql.push_str(order);
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| PhotoRow::from_row(r)) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    pub fn photo_ids(&self, where_clause: &str) -> Vec<i64> {
        let mut sql = String::from("SELECT id FROM photos");
        if !where_clause.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(where_clause);
        }
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| r.get::<_, i64>(0)) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    pub fn count(&self, where_clause: &str) -> i64 {
        let mut sql = String::from("SELECT COUNT(*) c FROM photos");
        if !where_clause.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(where_clause);
        }
        self.query_count(&sql, &[])
    }

    /// 按 path 插入或更新；返回 photo id。只写入传入的字段。
    pub fn upsert_photo(&mut self, d: &PhotoUpdate) -> rusqlite::Result<i64> {
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM photos WHERE path=?", params![d.path], |r| {
                r.get(0)
            })
            .optional()?;

        let sets = d.non_empty_sets();
        if let Some(pid) = existing {
            if !sets.is_empty() {
                let assign: Vec<String> =
                    sets.iter().map(|(k, _)| format!("{k}=?")).collect();
                let sql = format!("UPDATE photos SET {} WHERE id=?", assign.join(", "));
                let mut vals: Vec<rusqlite::types::Value> =
                    sets.iter().map(|(_, v)| v.clone()).collect();
                vals.push(rusqlite::types::Value::Integer(pid));
                self.conn.execute(&sql, rusqlite::params_from_iter(vals))?;
            }
            return Ok(pid);
        }

        // 插入：path 一定在内，其余按需
        let mut cols: Vec<String> = Vec::new();
        let mut vals: Vec<rusqlite::types::Value> = Vec::new();
        for (k, v) in &sets {
            cols.push((*k).to_string());
            vals.push(v.clone());
        }
        let placeholders: Vec<&str> = cols.iter().map(|_| "?").collect();
        let sql = format!(
            "INSERT INTO photos({}) VALUES({})",
            cols.join(","),
            placeholders.join(",")
        );
        self.conn.execute(&sql, rusqlite::params_from_iter(vals))?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn mark_missing(&self, id: i64, flag: bool) {
        let _ = self.conn.execute(
            "UPDATE photos SET missing=? WHERE id=?",
            params![if flag { 1 } else { 0 }, id],
        );
    }

    pub fn delete_photo(&self, id: i64) {
        let _ = self
            .conn
            .execute("DELETE FROM event_members WHERE photo_id=?", params![id]);
        let _ = self
            .conn
            .execute("DELETE FROM photo_tags WHERE photo_id=?", params![id]);
        let _ = self
            .conn
            .execute("DELETE FROM photos WHERE id=?", params![id]);
    }

    pub fn stats(&self) -> Stats {
        let total = self.query_count("SELECT COUNT(*) c FROM photos", &[]);
        let analyzed = self.query_count(
            "SELECT COUNT(*) c FROM photos WHERE analyzed_at IS NOT NULL",
            &[],
        );
        let with_dt = self.query_count("SELECT COUNT(*) c FROM photos WHERE dt IS NOT NULL", &[]);
        let dups = self.query_count(
            "SELECT COUNT(*) c FROM photos WHERE dup_group IS NOT NULL",
            &[],
        );
        let missing = self.query_count("SELECT COUNT(*) c FROM photos WHERE missing=1", &[]);
        let bytes = self.query_count("SELECT COALESCE(SUM(size),0) c FROM photos", &[]);
        let tags = self.query_count("SELECT COUNT(*) c FROM tags", &[]);

        let mut years = Vec::new();
        if let Ok(mut stmt) = self.conn.prepare(
            "SELECT year, COUNT(*) c FROM photos WHERE year IS NOT NULL GROUP BY year ORDER BY year",
        ) {
            if let Ok(it) = stmt.query_map([], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?))
            }) {
                years = it.flatten().collect();
            }
        }

        Stats {
            total,
            analyzed,
            with_dt,
            no_dt: total - with_dt,
            dups,
            missing,
            bytes,
            years,
            tags,
        }
    }

    // -------------------------------------------------------------- 标签

    pub fn tag_id(&mut self, name: &str, kind: &str, create: bool) -> rusqlite::Result<Option<i64>> {
        let name = name.trim();
        if name.is_empty() {
            return Ok(None);
        }
        let existing: Option<i64> = self
            .conn
            .query_row("SELECT id FROM tags WHERE name=?", params![name], |r| {
                r.get(0)
            })
            .optional()?;
        if let Some(id) = existing {
            return Ok(Some(id));
        }
        if !create {
            return Ok(None);
        }
        self.conn
            .execute("INSERT INTO tags(name,kind) VALUES(?,?)", params![name, kind])?;
        Ok(Some(self.conn.last_insert_rowid()))
    }

    pub fn add_tag(&mut self, pid: i64, name: &str, kind: &str, source: &str) {
        let Ok(Some(tid)) = self.tag_id(name, kind, true) else {
            return;
        };
        let _ = self.conn.execute(
            "INSERT OR IGNORE INTO photo_tags(photo_id,tag_id,source) VALUES(?,?,?)",
            params![pid, tid, source],
        );
    }

    /// 批量加标签。pairs 为 (photo_id, tag_name)。
    pub fn add_tags_bulk(&mut self, pairs: &[(i64, String)], kind: &str, source: &str) {
        if pairs.is_empty() {
            return;
        }
        let tx = match self.conn.transaction() {
            Ok(t) => t,
            Err(_) => return,
        };
        let mut cache: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for (pid, name) in pairs {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            let tid = if let Some(id) = cache.get(name) {
                *id
            } else {
                let id = match tx
                    .query_row("SELECT id FROM tags WHERE name=?", params![name], |r| {
                        r.get::<_, i64>(0)
                    })
                    .optional()
                {
                    Ok(Some(id)) => id,
                    _ => {
                        if tx
                            .execute("INSERT INTO tags(name,kind) VALUES(?,?)", params![name, kind])
                            .is_err()
                        {
                            continue;
                        }
                        tx.last_insert_rowid()
                    }
                };
                cache.insert(name.to_string(), id);
                id
            };
            let _ = tx.execute(
                "INSERT OR IGNORE INTO photo_tags(photo_id,tag_id,source) VALUES(?,?,?)",
                params![pid, tid, source],
            );
        }
        let _ = tx.commit();
    }

    pub fn remove_tag(&self, pid: i64, name: &str) {
        let _ = self.conn.execute(
            "DELETE FROM photo_tags WHERE photo_id=? AND tag_id IN (SELECT id FROM tags WHERE name=?)",
            params![pid, name],
        );
    }

    pub fn clear_auto_tags(&self) {
        let _ = self
            .conn
            .execute("DELETE FROM photo_tags WHERE source='auto'", []);
        let _ = self.conn.execute(
            "DELETE FROM tags WHERE kind='auto' AND id NOT IN (SELECT tag_id FROM photo_tags)",
            [],
        );
    }

    pub fn tags_of(&self, pid: i64) -> Vec<String> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT t.name FROM tags t JOIN photo_tags pt ON pt.tag_id=t.id \
             WHERE pt.photo_id=? ORDER BY t.name",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(params![pid], |r| r.get::<_, String>(0)) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// (name, kind, count)
    pub fn all_tags(&self) -> Vec<(String, String, i64)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT t.name, t.kind, COUNT(pt.photo_id) c FROM tags t \
             LEFT JOIN photo_tags pt ON pt.tag_id=t.id GROUP BY t.id \
             ORDER BY t.kind, c DESC, t.name",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, i64>(2)?,
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    // -------------------------------------------------------------- 事件

    /// 清空事件表及相关标签（重建聚类前调用）。
    pub fn clear_events(&self) {
        let _ = self.conn.execute("DELETE FROM event_members", []);
        let _ = self.conn.execute("DELETE FROM events", []);
        let _ = self.conn.execute(
            "DELETE FROM photo_tags WHERE source IN ('event','time')",
            [],
        );
        let _ = self.conn.execute(
            "DELETE FROM tags WHERE kind IN ('event','time') \
             AND id NOT IN (SELECT tag_id FROM photo_tags)",
            [],
        );
    }

    /// 插入一个事件，返回 id。
    pub fn add_event(
        &self,
        title: &str,
        start_dt: Option<&str>,
        end_dt: Option<&str>,
        lat: Option<f64>,
        lon: Option<f64>,
        count: i64,
        cover_id: i64,
    ) -> i64 {
        let _ = self.conn.execute(
            "INSERT INTO events(title,start_dt,end_dt,lat,lon,count,cover_id) \
             VALUES(?,?,?,?,?,?,?)",
            params![title, start_dt, end_dt, lat, lon, count, cover_id],
        );
        self.conn.last_insert_rowid()
    }

    pub fn add_event_members(&self, event_id: i64, photo_ids: &[i64]) {
        let Ok(tx) = self.conn.unchecked_transaction() else {
            return;
        };
        for pid in photo_ids {
            let _ = tx.execute(
                "INSERT OR REPLACE INTO event_members(event_id,photo_id) VALUES(?,?)",
                params![event_id, pid],
            );
        }
        let _ = tx.commit();
    }

    // -------------------------------------------------------------- 相册

    // -------------------------------------------------------------- 重复组

    /// 清空重复分组与相关标记（重算去重前调用）。
    pub fn reset_duplicates(&self) {
        let _ = self.conn.execute("UPDATE photos SET dup_group=NULL, is_best=0", []);
        let _ = self.conn.execute("DELETE FROM dup_groups", []);
        let _ = self
            .conn
            .execute("DELETE FROM photo_tags WHERE source='dup'", []);
    }

    /// 插入一个重复组，返回 id。
    pub fn add_dup_group(
        &self,
        kind: &str,
        count: i64,
        best_id: i64,
        keep_bytes: i64,
        saved_bytes: i64,
    ) -> i64 {
        let _ = self.conn.execute(
            "INSERT INTO dup_groups(kind,count,best_id,keep_bytes,saved_bytes) \
             VALUES(?,?,?,?,?)",
            params![kind, count, best_id, keep_bytes, saved_bytes],
        );
        self.conn.last_insert_rowid()
    }

    /// 批量设置照片的重复组归属。参数为 `(gid, is_best, photo_id)`。
    pub fn set_dup_members(&mut self, rows: &[(i64, i64, i64)]) {
        if rows.is_empty() {
            return;
        }
        let Ok(tx) = self.conn.transaction() else {
            return;
        };
        for (gid, best, pid) in rows {
            let _ = tx.execute(
                "UPDATE photos SET dup_group=?, is_best=? WHERE id=?",
                params![gid, best, pid],
            );
        }
        let _ = tx.commit();
    }

    // -------------------------------------------------------------- 地点

    /// 批量更新地点。参数为 `(place, photo_id)`。
    pub fn set_places(&mut self, rows: &[(String, i64)]) {
        if rows.is_empty() {
            return;
        }
        let Ok(tx) = self.conn.transaction() else {
            return;
        };
        for (place, pid) in rows {
            let _ = tx.execute(
                "UPDATE photos SET place=? WHERE id=?",
                params![place, pid],
            );
        }
        let _ = tx.commit();
    }

    /// (name, kind) —— 与 `tags_of` 不同，这里带类型，供归档时挑语义标签。
    pub fn photo_tags_with_kind(&self, pid: i64) -> Vec<(String, String)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT t.name, t.kind FROM tags t JOIN photo_tags pt ON pt.tag_id=t.id \
             WHERE pt.photo_id=? ORDER BY t.name",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(params![pid], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }



    /// 把一张照片的 path 从 `from` 改成 `to`（回滚用）。
    pub fn repoint_photo_path(&self, from: &str, to: &str) {
        let folder = Path::new(to)
            .parent()
            .map(|p| p.to_string_lossy().to_string());
        let _ = self.conn.execute(
            "UPDATE photos SET path=?, folder=? WHERE path=?",
            params![to, folder, from],
        );
    }

    /// 单独改一张照片的 name 列（改名后同步，path 用 `repoint_photo_path`）。
    pub fn set_photo_name(&self, id: i64, name: &str) {
        let _ = self
            .conn
            .execute("UPDATE photos SET name=? WHERE id=?", params![name, id]);
    }

    /// 重生成视频封面后写回缩略图路径。
    pub fn set_video_thumb(&self, id: i64, thumb: &str) {
        let _ = self.conn.execute(
            "UPDATE photos SET thumb=? WHERE id=?",
            params![thumb, id],
        );
    }

    /// 手动修正日期。`source` 为 None 表示清空手动值，恢复自动识别。
    pub fn set_manual_date(
        &self,
        id: i64,
        date: Option<&str>,
        precision: Option<&str>,
    ) -> rusqlite::Result<()> {
        let Some(d) = date.and_then(crate::util::parse_iso) else {
            return self
                .conn
                .execute("UPDATE photos SET dt_source=NULL WHERE id=?", params![id])
                .map(|_| ());
        };
        let prec = crate::util::Precision::from_str_opt(precision.unwrap_or("day"))
            .unwrap_or(crate::util::Precision::Day);
        self.conn.execute(
            "UPDATE photos SET dt=?, dt_source='manual', dt_precision=?, \
             year=?, month=?, day=?, hour=? WHERE id=?",
            params![
                crate::util::iso_dt(d),
                prec.as_str(),
                d.format("%Y").to_string().parse::<i64>().unwrap_or(0),
                d.format("%m").to_string().parse::<i64>().unwrap_or(0),
                d.format("%d").to_string().parse::<i64>().unwrap_or(0),
                d.format("%H").to_string().parse::<i64>().unwrap_or(0),
                id,
            ],
        )?;
        Ok(())
    }

    /// 手动修正地点。`place` 为 None/空表示清空手动值，恢复自动识别。
    pub fn set_manual_place(&self, id: i64, place: Option<&str>) -> rusqlite::Result<()> {
        let cleaned = place.map(|s| s.trim()).filter(|s| !s.is_empty());
        let Some(p) = cleaned else {
            return self
                .conn
                .execute("UPDATE photos SET place_source=NULL WHERE id=?", params![id])
                .map(|_| ());
        };
        self.conn.execute(
            "UPDATE photos SET place=?, place_source='manual' WHERE id=?",
            params![p, id],
        )?;
        Ok(())
    }

    /// 手动设置备注。空串表示清除。
    pub fn set_note(&self, id: i64, note: Option<&str>) -> rusqlite::Result<()> {
        let cleaned = note.map(|s| s.trim()).filter(|s| !s.is_empty());
        match cleaned {
            Some(s) => self.conn.execute(
                "UPDATE photos SET note=? WHERE id=?",
                params![s, id],
            ),
            None => self.conn.execute(
                "UPDATE photos SET note=NULL WHERE id=?",
                params![id],
            ),
        }?;
        Ok(())
    }

    /// 标记某条操作日志为已撤销。
    pub fn mark_op_undone(&self, id: i64) {
        let _ = self
            .conn
            .execute("UPDATE ops_log SET undone=1 WHERE id=?", params![id]);
    }

    /// 最近的事件（标题、张数、开始时间），供报告使用。
    pub fn recent_events(&self, limit: i64) -> Vec<(String, i64, String)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT title, count, start_dt FROM events ORDER BY start_dt DESC LIMIT ?",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(params![limit], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// 可省空间最多的重复组（id、类型、张数、可省字节）。
    pub fn top_dup_groups(&self, limit: i64) -> Vec<(i64, String, i64, i64)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT id, kind, count, saved_bytes FROM dup_groups \
             ORDER BY saved_bytes DESC LIMIT ?",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(params![limit], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                r.get::<_, Option<i64>>(3)?.unwrap_or(0),
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// 带参数的计数（供 Tauri 命令层拼动态 WHERE 用）。
    pub fn count_params(&self, where_clause: &str, args: &[String]) -> i64 {
        let sql = format!("SELECT COUNT(*) c FROM photos WHERE {where_clause}");
        let refs: Vec<&dyn rusqlite::ToSql> =
            args.iter().map(|s| s as &dyn rusqlite::ToSql).collect();
        self.conn
            .query_row(&sql, refs.as_slice(), |r| r.get::<_, i64>(0))
            .unwrap_or(0)
    }

    /// 分页查询照片（带参数）。
    pub fn query_photos_paged(
        &self,
        where_clause: &str,
        order: &str,
        args: &[String],
        limit: i64,
        offset: i64,
    ) -> Vec<PhotoRow> {
        let sql = format!(
            "SELECT * FROM photos WHERE {where_clause} ORDER BY {order} LIMIT ? OFFSET ?"
        );
        let mut vals: Vec<rusqlite::types::Value> = args
            .iter()
            .map(|s| rusqlite::types::Value::Text(s.clone()))
            .collect();
        vals.push(rusqlite::types::Value::Integer(limit));
        vals.push(rusqlite::types::Value::Integer(offset));
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(rusqlite::params_from_iter(vals), |r| {
            PhotoRow::from_row(r)
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// (year, month, count)，供时间轴
    pub fn year_month_counts(&self) -> Vec<(i64, Option<i64>, i64)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT year, month, COUNT(*) c FROM photos \
             WHERE missing=0 AND year IS NOT NULL \
             GROUP BY year, month ORDER BY year DESC, month ASC",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, i64>(2)?,
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// (place, year, month, count)，供地点页
    pub fn place_year_month_counts(&self, unknown: &str) -> Vec<(String, Option<i64>, Option<i64>, i64)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT COALESCE(NULLIF(place,''),?) AS p, year, month, COUNT(*) c \
             FROM photos WHERE missing=0 GROUP BY p, year, month ORDER BY c DESC",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(params![unknown], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, i64>(3)?,
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// 全部事件（按开始时间倒序）
    pub fn all_events(&self) -> Vec<(i64, String, Option<String>, Option<String>, i64, Option<i64>)> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT * FROM events ORDER BY start_dt DESC")
        else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
                r.get::<_, Option<i64>>(5)?.unwrap_or(0),
                r.get::<_, Option<i64>>(6)?,
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// (id, kind, count, best_id, saved_bytes)
    pub fn dup_groups(&self) -> Vec<(i64, String, i64, Option<i64>, i64)> {
        let Ok(mut stmt) = self
            .conn
            .prepare("SELECT * FROM dup_groups ORDER BY saved_bytes DESC")
        else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                r.get::<_, Option<i64>>(3)?,
                r.get::<_, Option<i64>>(4)?.unwrap_or(0),
            ))
        }) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// 某重复组里的照片，最优的排最前。
    pub fn photos_in_dup_group(&self, gid: i64) -> Vec<PhotoRow> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT * FROM photos WHERE dup_group=? AND missing=0 \
             ORDER BY is_best DESC, quality_score DESC",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map(params![gid], |r| PhotoRow::from_row(r)) else {
            return Vec::new();
        };
        it.flatten().collect()
    }

    /// 设置收藏状态。
    pub fn set_favorite(&self, id: i64, favorite: bool) {
        let _ = self.conn.execute(
            "UPDATE photos SET favorite=? WHERE id=?",
            params![if favorite { 1 } else { 0 }, id],
        );
    }

    /// 清空索引（保留或清空根目录列表），并清空缩略图缓存目录。
    pub fn reset_index(&self, keep_roots: bool) {
        let _ = self.conn.execute("DELETE FROM photo_tags", []);
        let _ = self.conn.execute("DELETE FROM event_members", []);
        let _ = self.conn.execute("DELETE FROM events", []);
        let _ = self.conn.execute("DELETE FROM dup_groups", []);
        let _ = self.conn.execute("DELETE FROM photos", []);
        let _ = self.conn.execute(
            "DELETE FROM sqlite_sequence WHERE name IN ('photos','events','dup_groups')",
            [],
        );
        if !keep_roots {
            let _ = self.conn.execute("DELETE FROM roots", []);
        }
        let dir = config::thumb_dir();
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
    }

    /// 用库里已有的指标重新套一遍判定标准（不重新解码图片）。
    ///
    /// 返回 `(总数, 被标记数)`。
    pub fn reclassify_flags(&self, cfg: &crate::config::Settings) -> (i64, i64) {
        let rows = self.all_photos("missing=0", "id");
        let mut updates: Vec<(String, i64)> = Vec::with_capacity(rows.len());
        let mut flagged = 0i64;
        for r in &rows {
            let m = crate::imaging::Metrics {
                width: r.width.unwrap_or(0) as u32,
                height: r.height.unwrap_or(0) as u32,
                megapixels: r.megapixels.unwrap_or(0.0),
                aspect: r.aspect.unwrap_or(0.0),
                blur: r.blur.unwrap_or(0.0),
                brightness: r.brightness.unwrap_or(0.0),
                contrast: r.contrast.unwrap_or(0.0),
                dark_ratio: r.dark_ratio.unwrap_or(0.0),
                white_ratio: r.white_ratio.unwrap_or(0.0),
                saturation: r.saturation.unwrap_or(0.0),
                hue: r.hue.unwrap_or(0.0),
                sepia: r.sepia.unwrap_or(0.0),
                color: r.color.unwrap_or(0.0),
                skin: r.skin.unwrap_or(0.0),
                noise: r.noise.unwrap_or(0.0),
                sharpness: r.sharpness.unwrap_or(0.0),
                lap_var: r.blur.unwrap_or(0.0),
            };
            // 开关关闭时清空标记，避免旧库残留
            let flags = if cfg.bool("lab_quality") {
                // 截图判定需要看像素，但只读缩略图就够
                let screen_like = std::path::Path::new(&r.path)
                    .exists()
                    .then(|| crate::imaging::open_image(std::path::Path::new(&r.path)))
                    .flatten()
                    .map(|img| crate::imaging::is_screen_like(&img, Some(&m)))
                    .unwrap_or(false);
                crate::imaging::classify_flags(&m, cfg, screen_like)
            } else {
                Vec::new()
            };
            if !flags.is_empty() {
                flagged += 1;
            }
            updates.push((flags.join("|"), r.id));
        }
        if !updates.is_empty() {
            if let Ok(tx) = self.conn.unchecked_transaction() {
                for (f, id) in &updates {
                    let _ = tx.execute("UPDATE photos SET flags=? WHERE id=?", params![f, id]);
                }
                let _ = tx.commit();
            }
        }
        (rows.len() as i64, flagged)
    }

    /// 清理重复副本：把非最优的移动到回收目录（或直接删除）。
    pub fn clean_duplicate_copies(
        &mut self,
        group_ids: &[i64],
        keep_all: bool,
        recycle: &Path,
        permanent: bool,
    ) -> CleanStats {
        let mut where_clause = "dup_group IS NOT NULL AND is_best=0 AND missing=0".to_string();
        if !keep_all {
            if group_ids.is_empty() {
                return CleanStats::default();
            }
            let list = group_ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            where_clause.push_str(&format!(" AND dup_group IN ({list})"));
        }
        let rows = self.all_photos(&where_clause, "id");
        let mut stats = CleanStats::default();
        for r in rows {
            let src = Path::new(&r.path);
            if !src.exists() {
                self.mark_missing(r.id, true);
                stats.errors += 1;
                continue;
            }
            let size = std::fs::metadata(src).map(|m| m.len() as i64).unwrap_or(0);
            if permanent {
                if std::fs::remove_file(src).is_ok() {
                    self.delete_photo(r.id);
                    stats.removed += 1;
                    stats.freed += size;
                } else {
                    stats.errors += 1;
                }
            } else {
                let _ = std::fs::create_dir_all(recycle);
                if let Some(dst) = move_with_unique_name(src, recycle) {
                    let dst_s = dst.to_string_lossy().to_string();
                    self.repoint_photo_path(&r.path, &dst_s);
                    self.mark_missing(r.id, true);
                    self.log_op("dedup_move", &r.path, &dst_s, "清理重复副本");
                    stats.moved += 1;
                    stats.freed += size;
                } else {
                    stats.errors += 1;
                }
            }
        }
        self.refresh_dup_groups();
        stats
    }

    /// 给定待删 id，返回其中会被"整组删光"拦下的照片。
    ///
    /// 规则：同一 dup_group 里，删除后仍在库中的照片至少留 1 张；
    /// 不属于重复组的照片不受影响。
    pub fn blocked_by_dup_guard(&self, ids: &[i64]) -> Vec<i64> {
        use std::collections::{HashMap, HashSet};
        let deleting: HashSet<i64> = ids.iter().copied().collect();
        let mut by_group: HashMap<i64, Vec<i64>> = HashMap::new();
        for id in ids {
            let Some(r) = self.photo(*id) else { continue };
            let Some(gid) = r.dup_group else { continue };
            by_group.entry(gid).or_default().push(*id);
        }
        let mut blocked = Vec::new();
        for (gid, picked) in by_group {
            let survivors: Vec<i64> = self
                .photo_ids(&format!("dup_group={gid} AND missing=0"))
                .into_iter()
                .filter(|id| !deleting.contains(id))
                .collect();
            if !survivors.is_empty() {
                continue;
            }
            // 整组都会被删：保下"最佳"那张，没有标记就保列表里第一张
            let best = picked.iter().copied().find(|id| {
                self.photo(*id)
                    .map(|r| r.is_best.unwrap_or(0) != 0)
                    .unwrap_or(false)
            });
            if let Some(id) = best.or_else(|| picked.first().copied()) {
                blocked.push(id);
            }
        }
        blocked
    }

    /// 按给定 id 顺序决定某组保留谁。`pick` 返回保留的照片 id。
    pub fn set_dup_keeper(&mut self, gid: i64, keep_id: i64) -> bool {
        let in_group = self
            .photo(keep_id)
            .map(|r| r.dup_group == Some(gid) && !r.is_missing())
            .unwrap_or(false);
        if !in_group {
            return false;
        }
        let Ok(tx) = self.conn.transaction() else {
            return false;
        };
        let _ = tx.execute(
            "UPDATE photos SET is_best=0 WHERE dup_group=?",
            params![gid],
        );
        let _ = tx.execute(
            "UPDATE photos SET is_best=1 WHERE id=?",
            params![keep_id],
        );
        let keep_bytes: i64 = tx
            .query_row(
                "SELECT COALESCE(size,0) FROM photos WHERE id=?",
                params![keep_id],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let sum: i64 = tx
            .query_row(
                "SELECT COALESCE(SUM(size),0) FROM photos WHERE dup_group=? AND missing=0",
                params![gid],
                |r| r.get(0),
            )
            .unwrap_or(0);
        let _ = tx.execute(
            "UPDATE dup_groups SET best_id=?, keep_bytes=?, saved_bytes=? WHERE id=?",
            params![keep_id, keep_bytes, (sum - keep_bytes).max(0), gid],
        );
        tx.commit().is_ok()
    }

    /// 按给定 id 删除照片（移到回收目录或直接删除）。
    ///
    /// `allow_whole_group` 为 false 时，同一重复组会被保下 1 张，
    /// 防止用户手滑把整组照片删光；为 true 表示用户已明确确认，照删不误。
    pub fn delete_photos_to(
        &mut self,
        ids: &[i64],
        recycle: &Path,
        permanent: bool,
        allow_whole_group: bool,
    ) -> CleanStats {
        let blocked = if allow_whole_group {
            Vec::new()
        } else {
            self.blocked_by_dup_guard(ids)
        };
        let mut stats = CleanStats::default();
        for id in ids {
            if blocked.contains(id) {
                stats.protected += 1;
                continue;
            }
            let Some(r) = self.photo(*id) else {
                stats.errors += 1;
                continue;
            };
            let src = Path::new(&r.path);
            if !src.exists() {
                self.mark_missing(r.id, true);
                stats.errors += 1;
                continue;
            }
            let size = std::fs::metadata(src).map(|m| m.len() as i64).unwrap_or(0);
            if permanent {
                if std::fs::remove_file(src).is_ok() {
                    self.delete_photo(r.id);
                    stats.removed += 1;
                    stats.freed += size;
                } else {
                    stats.errors += 1;
                }
            } else {
                let _ = std::fs::create_dir_all(recycle);
                if let Some(dst) = move_with_unique_name(src, recycle) {
                    let dst_s = dst.to_string_lossy().to_string();
                    self.repoint_photo_path(&r.path, &dst_s);
                    self.mark_missing(r.id, true);
                    self.log_op("delete_move", &r.path, &dst_s, "重复照片删除");
                    stats.moved += 1;
                    stats.freed += size;
                } else {
                    stats.errors += 1;
                }
            }
        }
        self.refresh_dup_groups();
        stats
    }

    /// 清理后重算每组张数与可省空间；只剩 1 张的组直接移除。
    fn refresh_dup_groups(&self) {
        let groups: Vec<i64> = {
            let Ok(mut stmt) = self.conn.prepare("SELECT id FROM dup_groups") else {
                return;
            };
            let Ok(it) = stmt.query_map([], |r| r.get::<_, i64>(0)) else {
                return;
            };
            it.flatten().collect()
        };
        for gid in groups {
            let mut sizes: Vec<i64> = {
                let Ok(mut stmt) = self
                    .conn
                    .prepare("SELECT size FROM photos WHERE dup_group=? AND missing=0")
                else {
                    continue;
                };
                let Ok(it) = stmt.query_map(params![gid], |r| r.get::<_, Option<i64>>(0)) else {
                    continue;
                };
                it.flatten().flatten().collect()
            };
            if sizes.len() < 2 {
                let _ = self.conn.execute(
                    "UPDATE photos SET dup_group=NULL, is_best=0 WHERE dup_group=?",
                    params![gid],
                );
                let _ = self
                    .conn
                    .execute("DELETE FROM dup_groups WHERE id=?", params![gid]);
                continue;
            }
            sizes.sort_by(|a, b| b.cmp(a));
            let saved: i64 = sizes[1..].iter().sum();
            let _ = self.conn.execute(
                "UPDATE dup_groups SET count=?, saved_bytes=? WHERE id=?",
                params![sizes.len() as i64, saved, gid],
            );
        }
    }

    /// 修掉历史遗留的 `\\?\` 扩展长度前缀。
    ///
    /// v1.1 早期版本用 `std::fs::canonicalize` 存路径，在 Windows 上会带上
    /// `\\?\C:\...` 前缀：显示难看，而且同一张图会因为前缀不同被当成两条记录
    /// （path 是 UNIQUE 键）。这里把 roots 与 photos 里的前缀剥掉。
    ///
    /// 返回 `(修正的 roots 数, 修正的 photos 数)`。
    pub fn strip_unc_prefixes(&mut self) -> (usize, usize) {
        let fix = |s: &str| -> Option<String> {
            // \\?\C:\...   -> C:\...
            // \\?\UNC\srv\share -> \\srv\share
            if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
                return Some(format!(r"\\{rest}"));
            }
            if let Some(rest) = s.strip_prefix(r"\\?\") {
                return Some(rest.to_string());
            }
            None
        };

        let mut roots_fixed = 0usize;
        let roots: Vec<String> = {
            let Ok(mut stmt) = self.conn.prepare("SELECT path FROM roots") else {
                return (0, 0);
            };
            let Ok(it) = stmt.query_map([], |r| r.get::<_, String>(0)) else {
                return (0, 0);
            };
            it.flatten().collect()
        };
        for old in roots {
            if let Some(new) = fix(&old) {
                // 目标可能已存在（重复添加过），那就直接删掉旧的
                let exists: Option<i64> = self
                    .conn
                    .query_row("SELECT id FROM roots WHERE path=?", params![new], |r| {
                        r.get(0)
                    })
                    .optional()
                    .ok()
                    .flatten();
                if exists.is_some() {
                    let _ = self.conn.execute("DELETE FROM roots WHERE path=?", params![old]);
                } else {
                    let _ = self
                        .conn
                        .execute("UPDATE roots SET path=? WHERE path=?", params![new, old]);
                }
                roots_fixed += 1;
            }
        }

        let mut photos_fixed = 0usize;
        let rows: Vec<(i64, String)> = {
            let Ok(mut stmt) = self
                .conn
                .prepare(r"SELECT id, path FROM photos WHERE path LIKE '\\?\%'")
            else {
                return (roots_fixed, 0);
            };
            let Ok(it) = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
            else {
                return (roots_fixed, 0);
            };
            it.flatten().collect()
        };
        for (id, old) in rows {
            let Some(new) = fix(&old) else { continue };
            let folder = Path::new(&new)
                .parent()
                .map(|p| p.to_string_lossy().to_string());
            // 目标 path 已存在时直接删掉这条重复记录
            let clash: Option<i64> = self
                .conn
                .query_row("SELECT id FROM photos WHERE path=?", params![new], |r| r.get(0))
                .optional()
                .ok()
                .flatten();
            if let Some(other) = clash {
                if other != id {
                    let _ = self
                        .conn
                        .execute("DELETE FROM photo_tags WHERE photo_id=?", params![id]);
                    let _ = self
                        .conn
                        .execute("DELETE FROM photos WHERE id=?", params![id]);
                    photos_fixed += 1;
                    continue;
                }
            }
            let _ = self.conn.execute(
                "UPDATE photos SET path=?, folder=?, thumb=? WHERE id=?",
                params![
                    new,
                    folder,
                    self.photo(id)
                        .and_then(|r| r.thumb)
                        .map(|t| fix(&t).unwrap_or(t)),
                    id
                ],
            );
            photos_fixed += 1;
        }
        (roots_fixed, photos_fixed)
    }

    /// 回收站条目：被移到回收目录、尚未取回的文件。
    ///
    /// 数据来源是 `ops_log` 里 `dedup_move` / `delete_move` 且 `undone=0` 的记录。
    /// 返回 `(log_id, 原路径, 回收目录路径, 时间戳)`，按时间倒序。
    ///
    /// 只列出文件仍在回收目录里的条目——文件被外部删掉的记录不展示，
    /// 否则用户点「取回」会得到「文件不存在」的失败提示。
    pub fn recycle_entries(&self) -> Vec<(i64, String, String, f64)> {
        let Ok(mut stmt) = self.conn.prepare(
            "SELECT id, src, dst, ts FROM ops_log \
             WHERE kind IN ('dedup_move','delete_move') AND undone=0 \
             ORDER BY id DESC",
        ) else {
            return Vec::new();
        };
        let Ok(it) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                r.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
            ))
        }) else {
            return Vec::new();
        };
        it.flatten()
            // 文件已不在了就不列出来
            .filter(|(_, _, dst, _)| Path::new(dst).exists())
            .collect()
    }

    /// 清理文件已不在磁盘上的回收记录（幽灵记录）。
    /// 返回被清掉的条数。这些记录留着只会让「全部取回」莫名其妙地失败。
    pub fn purge_ghost_recycle_entries(&self) -> usize {
        let rows: Vec<(i64, String)> = {
            let Ok(mut stmt) = self.conn.prepare(
                "SELECT id, dst FROM ops_log \
                 WHERE kind IN ('dedup_move','delete_move') AND undone=0",
            ) else {
                return 0;
            };
            let Ok(it) = stmt.query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                ))
            }) else {
                return 0;
            };
            it.flatten().collect()
        };
        let mut cleaned = 0usize;
        for (id, dst) in rows {
            if !Path::new(&dst).exists() {
                let _ = self
                    .conn
                    .execute("UPDATE ops_log SET undone=1 WHERE id=?", params![id]);
                cleaned += 1;
            }
        }
        cleaned
    }

    /// 从回收站取回一个文件：移回原路径并恢复索引。
    ///
    /// 返回 `(成功?, 最终路径, 说明)`。
    pub fn restore_from_recycle(&mut self, log_id: i64) -> (bool, String, String) {
        // 找到这条记录
        let row: Option<(String, String)> = self
            .conn
            .query_row(
                "SELECT src, dst FROM ops_log WHERE id=? AND undone=0",
                params![log_id],
                |r| {
                    Ok((
                        r.get::<_, Option<String>>(0)?.unwrap_or_default(),
                        r.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    ))
                },
            )
            .optional()
            .ok()
            .flatten();

        let Some((src, dst)) = row else {
            return (false, String::new(), "找不到这条回收记录".into());
        };
        let src_p = PathBuf::from(&src);
        let dst_p = PathBuf::from(&dst);
        if !dst_p.is_file() {
            return (false, String::new(), "回收目录里的文件已不存在".into());
        }

        // 原路径若已被占用，加序号，绝不覆盖
        let mut taken = std::collections::HashSet::new();
        let target = if src_p.exists() {
            unique_path(&src_p, &mut taken)
        } else {
            src_p.clone()
        };
        if let Some(parent) = target.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let moved = std::fs::rename(&dst_p, &target).is_ok()
            || (std::fs::copy(&dst_p, &target).is_ok() && std::fs::remove_file(&dst_p).is_ok());
        if !moved {
            return (false, String::new(), "移回文件失败".into());
        }

        // 恢复索引：路径改回来、清掉 missing
        let folder = target
            .parent()
            .map(|p| p.to_string_lossy().to_string());
        let _ = self.conn.execute(
            "UPDATE photos SET path=?, folder=?, missing=0 WHERE path=? OR path=?",
            params![
                target.to_string_lossy().to_string(),
                folder,
                dst,
                src
            ],
        );
        let _ = self
            .conn
            .execute("UPDATE ops_log SET undone=1 WHERE id=?", params![log_id]);

        (true, target.to_string_lossy().to_string(), "已取回".into())
    }

    /// 永久删除回收站里的一个文件（不可恢复）。
    pub fn purge_recycle(&mut self, log_id: i64) -> (bool, String) {
        let dst: Option<String> = self
            .conn
            .query_row(
                "SELECT dst FROM ops_log WHERE id=? AND undone=0",
                params![log_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .optional()
            .ok()
            .flatten()
            .flatten();
        let Some(dst) = dst else {
            return (false, "找不到这条回收记录".into());
        };
        let p = PathBuf::from(&dst);
        if p.is_file() && std::fs::remove_file(&p).is_err() {
            return (false, "删除文件失败".into());
        }
        let _ = self
            .conn
            .execute("UPDATE ops_log SET undone=1 WHERE id=?", params![log_id]);
        // 记录也彻底清掉，避免回收站里一直挂着
        let _ = self
            .conn
            .execute("DELETE FROM photos WHERE path=?", params![dst]);
        (true, "已永久删除".into())
    }

    // -------------------------------------------------------------- 日志

    pub fn log_op(&mut self, kind: &str, src: &str, dst: &str, detail: &str) -> i64 {
        let _ = self.conn.execute(
            "INSERT INTO ops_log(ts,kind,src,dst,detail) VALUES(?,?,?,?,?)",
            params![now_ts(), kind, src, dst, detail],
        );
        self.conn.last_insert_rowid()
    }

    /// 归档操作日志，含源路径、目标路径和是否撤销。
    pub fn organize_log_rows(
        &self,
        batch: Option<&str>,
        only_pending: bool,
    ) -> Vec<(i64, String, String, String, i64)> {
        let mut sql = "SELECT id,src,dst,detail,undone FROM ops_log WHERE kind='organize'".to_string();
        if only_pending {
            sql.push_str(" AND undone=0");
        }
        sql.push_str(" ORDER BY id DESC LIMIT 10000");
        let Ok(mut stmt) = self.conn.prepare(&sql) else {
            return Vec::new();
        };
        let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, i64>(4)?,
            ))
        }) else {
            return Vec::new();
        };
        rows.flatten()
            .filter(|row| {
                batch.is_none_or(|b| {
                    let batch_part = format!("batch={b}");
                    row.3.split(';').any(|part| part == batch_part)
                })
            })
            .collect()
    }

}

/// 一次 upsert 要写入的字段。只填需要的字段，未填的列不会被改动。
#[derive(Debug, Clone, Default)]
pub struct PhotoUpdate {
    pub path: String,
    fields: Vec<(&'static str, rusqlite::types::Value)>,
}

impl PhotoUpdate {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            fields: Vec::new(),
        }
    }

    fn push(&mut self, key: &'static str, v: rusqlite::types::Value) -> &mut Self {
        self.fields.push((key, v));
        self
    }

    pub fn text(&mut self, key: &'static str, v: Option<&str>) -> &mut Self {
        debug_assert!(PHOTO_COLS.contains(&key));
        match v {
            Some(s) => self.push(key, rusqlite::types::Value::Text(s.to_string())),
            None => self.push(key, rusqlite::types::Value::Null),
        }
    }

    pub fn int(&mut self, key: &'static str, v: Option<i64>) -> &mut Self {
        debug_assert!(PHOTO_COLS.contains(&key));
        match v {
            Some(i) => self.push(key, rusqlite::types::Value::Integer(i)),
            None => self.push(key, rusqlite::types::Value::Null),
        }
    }

    pub fn real(&mut self, key: &'static str, v: Option<f64>) -> &mut Self {
        debug_assert!(PHOTO_COLS.contains(&key));
        match v {
            Some(f) => self.push(key, rusqlite::types::Value::Real(f)),
            None => self.push(key, rusqlite::types::Value::Null),
        }
    }

    /// 收集所有待写字段；`path` 始终第一个。
    fn non_empty_sets(&self) -> Vec<(&'static str, rusqlite::types::Value)> {
        let mut out = vec![(
            "path",
            rusqlite::types::Value::Text(self.path.clone()),
        )];
        for (k, v) in &self.fields {
            if *k == "path" {
                continue;
            }
            out.push((*k, v.clone()));
        }
        out
    }
}

/// 清理/删除操作的统计
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct CleanStats {
    pub moved: usize,
    pub removed: usize,
    pub errors: usize,
    pub freed: i64,
    /// 因「整组至少保留一张」被拦下的照片数
    pub protected: usize,
}

/// 把文件移到目标目录，重名时自动加序号。
fn move_with_unique_name(src: &Path, dest_dir: &Path) -> Option<PathBuf> {
    let base = src.file_name()?.to_string_lossy().to_string();
    let stem = Path::new(&base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file")
        .to_string();
    let ext = Path::new(&base)
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| format!(".{s}"))
        .unwrap_or_default();
    let mut dst = dest_dir.join(&base);
    let mut k = 1;
    while dst.exists() {
        dst = dest_dir.join(format!("{stem}_{k}{ext}"));
        k += 1;
        if k > 10_000 {
            return None;
        }
    }
    // 先试 rename，跨卷时退回复制 + 删除
    if std::fs::rename(src, &dst).is_ok() {
        return Some(dst);
    }
    if std::fs::copy(src, &dst).is_ok() {
        let _ = std::fs::remove_file(src);
        return Some(dst);
    }
    None
}

/// 把路径转成绝对路径并归一化。
///
/// 刻意**不用** `std::fs::canonicalize`：它在 Windows 上会返回
/// `\\?\C:\Users\...` 这种扩展长度前缀，直接显示给用户就是乱码一样的观感，
/// 而且存进库后与其他路径比较也会不一致。
/// 这里只做纯字符串层面的归一化，与 Python 版 `os.path.abspath()` 行为对齐。
fn absolutize(p: &Path) -> String {
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|c| c.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    normalize_path(&abs).to_string_lossy().to_string()
}

/// 消掉 `.` 与多余的 `..`，统一分隔符，但不解析符号链接、不加 `\\?\` 前缀。
fn normalize_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                // 到根了就不能再往上
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 当前 Unix 时间（秒，含小数）
pub fn now_ts() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lib() -> Library {
        Library::open_memory().unwrap()
    }

    #[test]
    fn recycle_restore_moves_file_back() {
        let dir = std::env::temp_dir().join("po_recycle_restore");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let recycle = dir.join("recycle");
        std::fs::create_dir_all(&recycle).unwrap();

        let orig = dir.join("a.jpg");
        std::fs::write(&orig, b"photo-bytes").unwrap();

        let mut l = lib();
        let mut u = PhotoUpdate::new(orig.to_string_lossy().to_string());
        u.text("name", Some("a.jpg")).int("size", Some(11));
        let pid = l.upsert_photo(&u).unwrap();
        // 走真实的删除到回收站
        let s = l.delete_photos_to(&[pid], &recycle, false, false);
        assert_eq!(s.moved, 1, "{s:?}");
        assert!(!orig.exists(), "原文件应被移走");
        assert_eq!(l.photo(pid).unwrap().missing, Some(1));

        let entries = l.recycle_entries();
        assert_eq!(entries.len(), 1);
        let log_id = entries[0].0;

        let (ok, path, msg) = l.restore_from_recycle(log_id);
        assert!(ok, "取回失败：{msg}");
        assert!(Path::new(&path).exists(), "文件应回到原位置");
        assert_eq!(l.photo(pid).unwrap().missing, Some(0));
        assert!(l.recycle_entries().is_empty(), "取回后不该再列出");
    }

    #[test]
    fn recycle_restore_all_when_original_exists() {
        // 原路径已被占用时，取回应改名而不是覆盖
        let dir = std::env::temp_dir().join("po_recycle_conflict");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let recycle = dir.join("recycle");
        std::fs::create_dir_all(&recycle).unwrap();

        let orig = dir.join("b.jpg");
        std::fs::write(&orig, b"v1").unwrap();
        let mut l = lib();
        let mut u = PhotoUpdate::new(orig.to_string_lossy().to_string());
        u.text("name", Some("b.jpg"));
        let pid = l.upsert_photo(&u).unwrap();
        l.delete_photos_to(&[pid], &recycle, false, false);

        // 用户在回收期间又在原位置放了个同名文件
        std::fs::write(&orig, b"new-file").unwrap();

        let log_id = l.recycle_entries()[0].0;
        let (ok, path, _) = l.restore_from_recycle(log_id);
        assert!(ok);
        assert_ne!(path, orig.to_string_lossy().to_string(), "应改名避让");
        assert_eq!(std::fs::read(&orig).unwrap(), b"new-file".to_vec(), "不能覆盖");
    }

    #[test]
    fn schema_creates_all_tables() {
        let l = lib();
        let mut stmt = l
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table'")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .flatten()
            .collect();
        for t in ["photos", "roots", "tags", "photo_tags", "events", "dup_groups", "ops_log", "meta"] {
            assert!(names.contains(&t.to_string()), "missing table {t}: {names:?}");
        }
    }

    #[test]
    fn photo_has_favorite_column() {
        let l = lib();
        let cols = l.photo_columns();
        assert!(cols.contains(&"favorite".to_string()));
        assert!(cols.contains(&"place".to_string()));
        assert!(cols.contains(&"p_hash".to_string()));
    }

    #[test]
    fn insert_and_fetch_photo() {
        let mut l = lib();
        let mut u = PhotoUpdate::new(r"D:\photos\a.jpg");
        u.text("name", Some("a.jpg"))
            .text("folder", Some(r"D:\photos"))
            .int("size", Some(1234))
            .real("mtime", Some(1000.5));
        let pid = l.upsert_photo(&u).unwrap();
        assert!(pid > 0);

        let row = l.photo(pid).unwrap();
        assert_eq!(row.path, r"D:\photos\a.jpg");
        assert_eq!(row.name.as_deref(), Some("a.jpg"));
        assert_eq!(row.size, Some(1234));
        assert!(!row.is_favorite());
    }


    #[test]
    fn upsert_updates_only_given_fields() {
        let mut l = lib();
        let mut u = PhotoUpdate::new("x.jpg");
        u.text("name", Some("x.jpg")).int("size", Some(10));
        let pid = l.upsert_photo(&u).unwrap();

        // 第二次只改 width，size 不该被清空
        let mut u2 = PhotoUpdate::new("x.jpg");
        u2.int("width", Some(800));
        let pid2 = l.upsert_photo(&u2).unwrap();
        assert_eq!(pid, pid2);

        let row = l.photo(pid).unwrap();
        assert_eq!(row.width, Some(800));
        assert_eq!(row.size, Some(10), "未指定的列不应被覆盖");
    }

    #[test]
    fn upsert_is_case_sensitive_on_path() {
        let mut l = lib();
        l.upsert_photo(&mut PhotoUpdate::new("A.jpg")).unwrap();
        l.upsert_photo(&mut PhotoUpdate::new("a.jpg")).unwrap();
        assert_eq!(l.count(""), 2);
    }

    #[test]
    fn stats_counts_correctly() {
        let mut l = lib();
        for (i, name) in ["a.jpg", "b.jpg", "c.jpg"].iter().enumerate() {
            let mut u = PhotoUpdate::new(*name);
            u.int("size", Some(100)).int("year", Some(1990 + i as i64));
            let pid = l.upsert_photo(&u).unwrap();
            if i < 2 {
                let mut u2 = PhotoUpdate::new(*name);
                u2.text("dt", Some("1990-01-01 00:00:00"))
                    .real("analyzed_at", Some(1.0));
                l.upsert_photo(&u2).unwrap();
                let _ = pid;
            }
        }
        let s = l.stats();
        assert_eq!(s.total, 3);
        assert_eq!(s.analyzed, 2);
        assert_eq!(s.with_dt, 2);
        assert_eq!(s.no_dt, 1);
        assert_eq!(s.bytes, 300);
        assert_eq!(s.years, vec![(1990, 1), (1991, 1), (1992, 1)]);
    }

    #[test]
    fn absolutize_never_adds_unc_prefix() {
        // Windows 上 `\\?\C:\...` 这种扩展前缀显示给用户像乱码，必须避免
        let got = absolutize(Path::new(r"C:\Users\乐\Pictures"));
        assert!(!got.starts_with(r"\\?\"), "不该带 \\\\?\\ 前缀: {got}");
        assert!(got.starts_with(r"C:\"), "{got}");
        assert!(got.ends_with("Pictures"), "{got}");
        // 中文不能被破坏
        assert!(got.contains('乐'), "{got}");
    }

    #[test]
    fn absolutize_is_idempotent() {
        let once = absolutize(Path::new(r"C:\a\b\c"));
        let twice = absolutize(Path::new(&once));
        assert_eq!(once, twice);
    }

    #[test]
    fn normalize_path_collapses_dots() {
        assert_eq!(
            normalize_path(Path::new(r"C:\a\.\b\..\c")).to_string_lossy(),
            r"C:\a\c"
        );
        assert_eq!(normalize_path(Path::new(r"C:\a\b")).to_string_lossy(), r"C:\a\b");
    }

    #[test]
    fn reset_index_keeps_roots_when_asked() {
        let mut l = lib();
        l.add_root(Path::new(r"D:\keep\me"));
        l.upsert_photo(&mut PhotoUpdate::new(r"D:\a\1.jpg")).unwrap();
        l.upsert_photo(&mut PhotoUpdate::new(r"D:\a\2.jpg")).unwrap();
        assert_eq!(l.count(""), 2);

        l.reset_index(true);
        assert_eq!(l.count(""), 0, "照片记录应清空");
        assert_eq!(l.roots().len(), 1, "目录应保留");
    }

    #[test]
    fn reset_index_clears_roots_when_not_kept() {
        let mut l = lib();
        l.add_root(Path::new(r"D:\gone\dir"));
        l.upsert_photo(&mut PhotoUpdate::new(r"D:\a\1.jpg")).unwrap();

        l.reset_index(false);
        assert_eq!(l.count(""), 0, "照片记录应清空");
        assert!(l.roots().is_empty(), "目录也应清空");
    }

    #[test]
    fn remove_root_only_drops_that_dir() {
        let l = lib();
        l.add_root(Path::new(r"D:\a"));
        l.add_root(Path::new(r"D:\b"));
        l.add_root(Path::new(r"D:\c"));
        assert_eq!(l.roots().len(), 3);

        l.remove_root(Path::new(r"D:\b"));
        let roots = l.roots();
        assert_eq!(roots.len(), 2);
        assert!(!roots.iter().any(|r| r.ends_with(r"\b")), "{roots:?}");
        // 其他目录不受影响
        assert!(roots.iter().any(|r| r.ends_with(r"\a")));
        assert!(roots.iter().any(|r| r.ends_with(r"\c")));
    }

    #[test]
    fn remove_root_does_not_touch_photos() {
        // 移除目录只影响 roots 表，照片记录还在（只是下次扫描不再从这个目录找）
        let mut l = lib();
        l.add_root(Path::new(r"D:\keep\photos"));
        l.upsert_photo(&mut PhotoUpdate::new(r"D:\keep\photos\a.jpg"))
            .unwrap();
        assert_eq!(l.count(""), 1);

        l.remove_root(Path::new(r"D:\keep\photos"));
        assert!(l.roots().is_empty());
        assert_eq!(l.count(""), 1, "移除目录不该删照片记录");
    }

    #[test]
    fn roots_roundtrip_without_unc_prefix() {
        let l = lib();
        let p = l.add_root(Path::new(r"C:\Users\乐\Pictures\Screenshots"));
        assert!(!p.starts_with(r"\\?\"), "入库路径不该带 \\\\?\\ 前缀: {p}");
        let roots = l.roots();
        assert_eq!(roots.len(), 1);
        assert!(!roots[0].starts_with(r"\\?\"), "读回来的也不该带: {}", roots[0]);
    }

    #[test]
    fn strip_unc_prefixes_cleans_roots_and_photos() {
        let mut l = lib();
        // 模拟被 canonicalize 污染的数据
        l.conn
            .execute(
                "INSERT INTO roots(path,added_at,recursive) VALUES(?,?,1)",
                params![r"\\?\C:\Users\乐\Pictures\Screenshots", 1.0],
            )
            .unwrap();
        let mut u = PhotoUpdate::new(r"\\?\C:\Users\乐\Pictures\a.jpg");
        u.text("folder", Some(r"\\?\C:\Users\乐\Pictures"));
        let pid = l.upsert_photo(&u).unwrap();

        let (r, p) = l.strip_unc_prefixes();
        assert_eq!(r, 1, "应修好 1 个 root");
        assert_eq!(p, 1, "应修好 1 张照片");

        let roots = l.roots();
        assert_eq!(roots, vec![r"C:\Users\乐\Pictures\Screenshots".to_string()]);

        let row = l.photo(pid).unwrap();
        assert_eq!(row.path, r"C:\Users\乐\Pictures\a.jpg");
        assert!(!row.path.starts_with(r"\\?\"));
    }

    #[test]
    fn strip_unc_prefixes_is_noop_on_clean_db() {
        let mut l = lib();
        l.add_root(Path::new(r"C:\clean\path"));
        let (r, p) = l.strip_unc_prefixes();
        assert_eq!((r, p), (0, 0), "干净库不该被动到");
    }

    #[test]
    fn strip_unc_prefixes_merges_duplicate_paths() {
        let mut l = lib();
        // 同一张图，一份带前缀一份不带：修完应只剩一条
        l.upsert_photo(&mut PhotoUpdate::new(r"C:\a\b.jpg")).unwrap();
        l.conn
            .execute(
                "INSERT INTO photos(path,name) VALUES(?,?)",
                params![r"\\?\C:\a\b.jpg", "b.jpg"],
            )
            .unwrap();
        assert_eq!(l.count(""), 2);

        let (_, p) = l.strip_unc_prefixes();
        assert_eq!(p, 1);
        assert_eq!(l.count(""), 1, "重复记录应被合并");
        assert!(l.photo_by_path(r"C:\a\b.jpg").is_some());
    }

    #[test]
    fn strip_unc_handles_unc_share_paths() {
        let mut l = lib();
        let mut u = PhotoUpdate::new(r"\\?\UNC\server\share\a.jpg");
        l.upsert_photo(&mut u).unwrap();
        l.strip_unc_prefixes();
        assert!(l.photo_by_path(r"\\server\share\a.jpg").is_some());
    }

    #[test]
    fn roots_add_list_remove() {
        let l = lib();
        let p = l.add_root(Path::new(r"D:\photos"));
        assert!(!p.is_empty());
        assert_eq!(l.roots().len(), 1);
        // 重复添加不产生第二条
        l.add_root(Path::new(r"D:\photos"));
        assert_eq!(l.roots().len(), 1);
        l.remove_root(Path::new(r"D:\photos"));
        assert!(l.roots().is_empty());
    }

    #[test]
    fn tags_bulk_and_query() {
        let mut l = lib();
        let p1 = l.upsert_photo(&mut PhotoUpdate::new("a.jpg")).unwrap();
        let p2 = l.upsert_photo(&mut PhotoUpdate::new("b.jpg")).unwrap();
        l.add_tags_bulk(
            &[(p1, "全家福".into()), (p1, "重复".into()), (p2, "全家福".into())],
            "auto",
            "auto",
        );
        let mut tags = l.tags_of(p1);
        tags.sort();
        assert_eq!(tags, vec!["全家福".to_string(), "重复".to_string()]);

        let all = l.all_tags();
        let family = all.iter().find(|(n, _, _)| n == "全家福").unwrap();
        assert_eq!(family.2, 2);
    }

    #[test]
    fn clear_auto_tags_removes_orphans() {
        let mut l = lib();
        let p = l.upsert_photo(&mut PhotoUpdate::new("a.jpg")).unwrap();
        l.add_tags_bulk(&[(p, "模糊".into())], "auto", "auto");
        l.add_tags_bulk(&[(p, "事件:1990".into())], "event", "event");
        l.clear_auto_tags();
        let tags = l.tags_of(p);
        assert_eq!(tags, vec!["事件:1990".to_string()]);
    }

    #[test]
    fn meta_roundtrip() {
        let l = lib();
        assert_eq!(l.get_meta("last_error"), None);
        l.set_meta("last_error", "boom").unwrap();
        assert_eq!(l.get_meta("last_error").as_deref(), Some("boom"));
    }

    #[test]
    fn ops_log_appends() {
        let mut l = lib();
        l.log_op("organize", "a.jpg", "b.jpg", "batch=B1");
        let (kind, src, dst, detail): (String, String, String, String) = l
            .conn
            .query_row(
                "SELECT kind,src,dst,detail FROM ops_log ORDER BY id DESC LIMIT 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .unwrap();
        assert_eq!(kind, "organize");
        assert_eq!(src, "a.jpg");
        assert_eq!(dst, "b.jpg");
        assert!(detail.contains("batch=B1"));
    }

    #[test]
    fn migrate_adds_missing_favorite_column() {
        // 模拟旧库：photos 表没有 favorite 列
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE photos (id INTEGER PRIMARY KEY AUTOINCREMENT, path TEXT UNIQUE NOT NULL);
             CREATE TABLE tags (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT UNIQUE NOT NULL,
                kind TEXT DEFAULT 'user', color TEXT DEFAULT '');",
        )
        .unwrap();
        let mut l = Library {
            path: PathBuf::from(":memory:"),
            conn,
        };
        l.migrate();
        let cols = l.photo_columns();
        assert!(cols.contains(&"favorite".to_string()));
        assert!(cols.contains(&"place".to_string()));
    }

    #[test]
    fn all_photos_filter_and_order() {
        let mut l = lib();
        for (n, y) in [("a.jpg", 1995), ("b.jpg", 1985), ("c.jpg", 2005)] {
            let mut u = PhotoUpdate::new(n);
            u.int("year", Some(y));
            l.upsert_photo(&u).unwrap();
        }
        let rows = l.all_photos("year > 1990", "year ASC");
        let years: Vec<i64> = rows.iter().filter_map(|r| r.year).collect();
        assert_eq!(years, vec![1995, 2005]);
    }
}
