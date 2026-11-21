# -*- coding: utf-8 -*-
"""整理归档：按分类规则复制/移动照片，支持命名模板、冲突处理与回滚。"""
from __future__ import annotations

import datetime as dt
import os
import shutil
import time

from .library import Library
from .util import (human_size, parse_iso, safe_name, sha1_file, unique_path)

SCHEMES = {
    "year": ["{year}"],
    "year_month": ["{year}", "{year}-{month}"],
    "year_month_day": ["{year}", "{year}-{month}", "{year}-{month}-{day}"],
    "place": ["{place}"],
    "place_year": ["{place}", "{year}"],
    "place_year_month": ["{place}", "{year}", "{year}-{month}"],
    "quality": ["{quality}"],
    "dup": ["{dup}"],
    "flat": [],
}

PLACE_UNKNOWN = "未识别地点"
FAVORITE_DIR = "收藏"


def _fmt(template: str, row, extra: dict) -> str:
    d = parse_iso(row["dt"])
    place = (row["place"] or "").strip()
    if not place or place == "未知地点":
        place = PLACE_UNKNOWN
    vals = {
        "year": "%04d" % (row["year"] or 0) if row["year"] else "无年份",
        "month": "%02d" % (row["month"] or 0) if row["month"] else "00",
        "day": "%02d" % (row["day"] or 0) if row["day"] else "00",
        "hour": "%02d" % (row["hour"] or 0),
        "minute": d.strftime("%M") if d else "00",
        "second": d.strftime("%S") if d else "00",
        "date": d.strftime("%Y-%m-%d") if d else "0000-00-00",
        "time": d.strftime("%H-%M-%S") if d else "00-00-00",
        "name": os.path.splitext(row["name"] or "photo")[0],
        "ext": (row["ext"] or ".jpg").lstrip("."),
        "tag": extra.get("tag") or "未分类",
        "event": extra.get("event") or "未分组",
        "quality": extra.get("quality") or "未评级",
        "dup": extra.get("dup") or "独立",
        "camera": safe_name(row["camera_model"] or "未知相机"),
        "place": safe_name(place or PLACE_UNKNOWN, 60),
        "undated": extra.get("undated") or "",
    }
    try:
        out = template.format(**vals)
    except Exception:
        out = template
    return safe_name(out, 120)


def _primary_tag(lib: Library, pid: int, kind_pref=("event", "user", "auto")):
    rows = lib.q("SELECT t.name, t.kind FROM tags t JOIN photo_tags pt ON pt.tag_id=t.id "
                 "WHERE pt.photo_id=?", (pid,))
    by_kind = {}
    for r in rows:
        by_kind.setdefault(r["kind"], []).append(r["name"])
    for k in kind_pref:
        if by_kind.get(k):
            # 事件优先取带年份的，自动标签跳过质量类
            for n in by_kind[k]:
                if k == "auto" and n in ("模糊", "过暗", "过亮", "低分辨率", "重复",
                                         "高光过曝", "大面积死黑", "横幅", "竖幅", "噪点重"):
                    continue
                return n
    return None


QUALITY_BUCKETS = ("优质", "一般", "较差")
BAD_AUTO_TAGS = {"模糊", "过暗", "过亮", "低分辨率", "重复", "高光过曝", "大面积死黑",
                 "横幅", "竖幅", "噪点重", "灰白/单色", "褪色平淡", "年代感强",
                 "棕褐老照片", "时间待确认", "有人物", "高清大图", "疑似扫描件"}


def _event_of(lib: Library, pid: int):
    r = lib.q1("SELECT t.name FROM tags t JOIN photo_tags pt ON pt.tag_id=t.id "
               "WHERE pt.photo_id=? AND t.kind='event' LIMIT 1", (pid,))
    return r["name"] if r else "未分组"


def _tag_of(lib: Library, pid: int):
    rows = lib.q("SELECT t.name, t.kind FROM tags t JOIN photo_tags pt ON pt.tag_id=t.id "
                 "WHERE pt.photo_id=? AND t.kind IN ('user','auto')", (pid,))
    names = [r["name"] for r in rows if r["name"] not in BAD_AUTO_TAGS]
    if not names:
        return "未分类"
    # 语义标签优先于尺寸/技术类
    return names[0]


def _quality_of(row):
    q = row["quality_score"] or 0
    return "优质" if q >= 70 else ("一般" if q >= 40 else "较差")


def _dup_of(row):
    return "重复" if row["dup_group"] else "独立"


def plan_organize(lib: Library, scheme="year_month", template=None, limit=0,
                  only_ids=None, only_where=None, with_id=False, parts=None,
                  favorite_dir=False, favorite_in_regular=True) -> list:
    """生成整理计划：[(photo_id, 相对路径)]。

    scheme: year / year_month / year_month_day / place / place_year /
            place_year_month / quality / dup / flat
    parts: 自定义分组层级（覆盖 scheme），如 ["{year}", "{place}"]
    favorite_dir: 收藏照片是否单独放一个「收藏」文件夹
    favorite_in_regular: 收藏照片是否同时保留在常规分类里
    """
    parts = list(parts) if parts else SCHEMES.get(scheme, SCHEMES["year_month"])
    need_tag = any("{tag}" in x for x in parts)
    need_event = any("{event}" in x for x in parts)
    need_quality = any("{quality}" in x for x in parts)
    need_dup = any("{dup}" in x for x in parts)

    where = "missing=0"
    args = []
    if only_ids:
        where += " AND id IN (%s)" % ",".join("?" * len(only_ids))
        args.extend(only_ids)
    if only_where:
        where += " AND (" + only_where + ")"
    rows = lib.all_photos(where, tuple(args))
    if limit:
        rows = rows[:limit]

    fname_tpl = template or "{date}_{time}_{name}"
    plan = []
    taken = set()

    def _add(pid, rel):
        """把一个相对路径加入计划，并处理同名冲突。"""
        if rel.lower() in taken:
            stem, e = os.path.splitext(rel)
            k = 2
            while ("%s_%d%s" % (stem, k, e)).lower() in taken:
                k += 1
            rel = "%s_%d%s" % (stem, k, e)
        taken.add(rel.lower())
        plan.append((pid, rel))

    for r in rows:
        extra = {}
        if need_tag:
            extra["tag"] = _tag_of(lib, r["id"])
        if need_event:
            extra["event"] = _event_of(lib, r["id"])
        if need_quality:
            extra["quality"] = _quality_of(r)
        if need_dup:
            extra["dup"] = _dup_of(r)
        segs = [_fmt(pt, r, extra) for pt in parts] if parts else []
        base = _fmt(fname_tpl, r, extra)
        ext = (r["ext"] or ".jpg").lstrip(".").lower()
        if not base.lower().endswith("." + ext):
            base = base + "." + ext

        is_fav = bool(r["favorite"]) if "favorite" in r.keys() else False

        if is_fav and favorite_dir:
            if favorite_in_regular:
                # 常规分类一份 + 收藏夹一份，用 (id, path) 里的路径区分
                rel_reg = os.path.join(*segs, base) if segs else base
                _add(r["id"], rel_reg)
                _add(r["id"], os.path.join(FAVORITE_DIR, base))
            else:
                # 只放收藏夹，不进常规分类
                _add(r["id"], os.path.join(FAVORITE_DIR, base))
            continue

        rel = os.path.join(*segs, base) if segs else base
        _add(r["id"], rel)
    return plan


def execute_organize(lib: Library, dest_root: str, plan, mode="copy",
                     on_conflict="rename", progress=None, cancel=None,
                     verify=True) -> dict:
    """按计划执行。mode: copy / move / link。返回统计，可回滚。"""
    dest_root = os.path.abspath(dest_root)
    os.makedirs(dest_root, exist_ok=True)
    batch = "B%d" % int(time.time())
    done = skipped = errors = 0
    moved = 0
    total = len(plan)
    taken = set()
    log_ids = []
    for i, (pid, rel) in enumerate(plan):
        if cancel is not None and cancel.is_set():
            break
        row = lib.photo(pid)
        if not row or not os.path.exists(row["path"]):
            errors += 1
            continue
        target = os.path.join(dest_root, rel)
        try:
            os.makedirs(os.path.dirname(target), exist_ok=True)
        except Exception:
            pass
        if os.path.exists(target):
            if on_conflict == "skip":
                skipped += 1
                continue
            if on_conflict == "overwrite":
                try:
                    os.remove(target)
                except Exception:
                    pass
            else:
                target = unique_path(target, taken)
        else:
            taken.add(target.lower())
        try:
            if mode == "move":
                shutil.move(row["path"], target)
                moved += 1
            elif mode == "link":
                try:
                    os.link(row["path"], target)
                except Exception:
                    shutil.copy2(row["path"], target)
            else:
                shutil.copy2(row["path"], target)
        except Exception:
            errors += 1
            continue
        ok = True
        if verify and mode != "link":
            try:
                ok = sha1_file(target) == (row["sha1"] or sha1_file(target))
            except Exception:
                ok = False
        detail = "batch=%s;mode=%s;sha1ok=%s;size=%s" % (batch, mode, ok, human_size(row["size"]))
        oid = lib.log_op("organize", row["path"], target, detail)
        log_ids.append(oid)
        if mode == "move":
            lib.ex("UPDATE photos SET path=?,folder=?,missing=0 WHERE id=?",
                   (target, os.path.dirname(target), pid))
        done += 1
        if progress and (i % 20 == 0 or i == total - 1):
            progress("organize", i + 1, total, os.path.basename(target))
    return {"done": done, "skipped": skipped, "errors": errors, "batch": batch,
            "moved": moved, "total": total, "log_ids": log_ids, "dest": dest_root}


def rollback(lib: Library, batch: str = None, limit=0) -> dict:
    """撤销某批次（或最近未撤销）的整理操作。"""
    if batch:
        rows = lib.q("SELECT * FROM ops_log WHERE kind='organize' AND undone=0 "
                     "AND detail LIKE ? ORDER BY id DESC", ("%" + batch + "%",))
    else:
        last = lib.q1("SELECT detail FROM ops_log WHERE kind='organize' AND undone=0 "
                      "ORDER BY id DESC LIMIT 1")
        if not last:
            return {"restored": 0, "errors": 0}
        b = "batch=" + last["detail"].split("batch=")[1].split(";")[0]
        rows = lib.q("SELECT * FROM ops_log WHERE kind='organize' AND undone=0 "
                     "AND detail LIKE ? ORDER BY id DESC", ("%" + b + "%",))
    if limit:
        rows = rows[:limit]
    restored = errors = 0
    for r in rows:
        src, dst = r["src"], r["dst"]
        try:
            if os.path.exists(dst):
                os.makedirs(os.path.dirname(src), exist_ok=True)
                if os.path.exists(src):
                    os.remove(dst)
                else:
                    shutil.move(dst, src)
                lib.ex("UPDATE photos SET path=?,folder=? WHERE path=?",
                       (src, os.path.dirname(src), dst))
                lib.ex("UPDATE ops_log SET undone=1 WHERE id=?", (r["id"],))
                restored += 1
        except Exception:
            errors += 1
    return {"restored": restored, "errors": errors}


def rename_in_place(lib: Library, plan_renames, dry_run=True, progress=None) -> dict:
    """原地重命名（同目录内），返回可回滚的日志。"""
    batch = "R%d" % int(time.time())
    done = errors = 0
    for pid, new_path in plan_renames:
        row = lib.photo(pid)
        if not row:
            errors += 1
            continue
        src = row["path"]
        dst = os.path.join(os.path.dirname(src), os.path.basename(new_path))
        if src == dst or not os.path.exists(src):
            continue
        if os.path.exists(dst):
            errors += 1
            continue
        if dry_run:
            done += 1
            continue
        try:
            os.rename(src, dst)
            lib.ex("UPDATE photos SET path=?,name=? WHERE id=?",
                   (dst, os.path.basename(dst), pid))
            lib.log_op("rename", src, dst, "batch=%s" % batch)
            done += 1
        except Exception:
            errors += 1
    return {"renamed": done, "errors": errors, "batch": batch, "dry_run": dry_run}