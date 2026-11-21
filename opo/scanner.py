# -*- coding: utf-8 -*-
"""扫描 + 分析流水线。支持增量、多线程、进度回调。"""
from __future__ import annotations

import os
import time
import traceback

from . import config, imaging
from .library import Library
from .util import (IMG_EXTS, RAW_EXTS, is_image, iso, human_size, parse_datetime,
                   parse_datetime_from_path, safe_name)

SKIP_DIRS = {"$recycle.bin", "system volume information", ".git", "node_modules",
             "__pycache__", ".thumbnails", "thumbs", "@eadir"}


def walk_images(root: str, recursive=True, exclude=(), on_file=None, cancel=None):
    """遍历目录，产出图片路径。"""
    root = os.path.abspath(root)
    ex = {os.path.abspath(p).lower() for p in exclude if p}
    if not os.path.isdir(root):
        return
    if not recursive:
        try:
            names = sorted(os.listdir(root))
        except Exception:
            return
        for n in names:
            p = os.path.join(root, n)
            if os.path.isfile(p) and is_image(p):
                if on_file:
                    on_file(p)
                yield p
        return
    for dirpath, dirnames, filenames in os.walk(root, topdown=True):
        if cancel is not None and cancel.is_set():
            return
        low = dirpath.lower()
        dirnames[:] = [d for d in dirnames
                       if d.lower() not in SKIP_DIRS
                       and not d.startswith(".")
                       and os.path.join(dirpath, d).lower() not in ex]
        # 跳过本工具的整理输出目录
        if os.path.basename(low) in ("oldphoto_organized", "opo-organized"):
            dirnames[:] = []
            continue
        for n in filenames:
            p = os.path.join(dirpath, n)
            if is_image(p):
                if on_file:
                    on_file(p)
                yield p


def scan_roots(lib: Library, cfg: dict, roots=None, progress=None, cancel=None,
               force=False) -> dict:
    """把磁盘上的新文件写进库（只登记，不做重分析）。"""
    roots = roots if roots is not None else lib.roots()
    added = skipped = errors = 0
    seen = set()
    for root in roots:
        lib.add_root(root)
        for path in walk_images(root, recursive=True, cancel=cancel):
            if cancel is not None and cancel.is_set():
                break
            seen.add(path)
            try:
                st = os.stat(path)
            except Exception:
                errors += 1
                continue
            row = lib.photo_by_path(path)
            if row and not force:
                if abs((row["mtime"] or 0) - st.st_mtime) < 1e-6:
                    skipped += 1
                    continue
            lib.upsert_photo({
                "path": path,
                "folder": os.path.dirname(path),
                "name": os.path.basename(path),
                "ext": os.path.splitext(path)[1].lower(),
                "size": st.st_size,
                "mtime": st.st_mtime,
                "ctime": st.st_ctime,
            })
            added += 1
            if progress and added % 25 == 0:
                progress("scan", added, 0, os.path.basename(path))
    # 标记消失的文件
    if seen:
        rows = lib.q("SELECT id,path FROM photos WHERE missing=0")
        for r in rows:
            if r["path"] not in seen and not os.path.exists(r["path"]):
                lib.mark_missing(r["id"], 1)
    return {"added": added, "skipped": skipped, "errors": errors,
            "roots": len(roots), "seen": len(seen)}


def _analyze_one(args):
    """子进程/线程工作单元。"""
    (pid, path, cfg) = args
    from .imaging import thumb_key
    tdir = config.thumb_dir()
    try:
        mtime = os.path.getmtime(path)
    except Exception:
        return (pid, None, "stat failed")
    th = os.path.join(tdir, thumb_key(path, cfg["thumb_size"], mtime) + ".jpg")
    try:
        res = imaging.analyze_photo(path, cfg, th, cfg["thumb_size"], cfg["thumb_quality"])
    except Exception:
        return (pid, None, traceback.format_exc()[-300:])
    if res is None:
        return (pid, None, "decode failed")
    res["thumb"] = th if res.get("thumb_ok") else None
    return (pid, res, None)


def analyze_pending(lib: Library, cfg: dict, limit=0, progress=None, cancel=None,
                    workers=None, redo=False) -> dict:
    """分析还没分析过的照片：EXIF、质量、哈希、缩略图。"""
    where = "analyzed_at IS NULL OR thumb IS NULL" if not redo else "1=1"
    rows = lib.q("SELECT id,path,mtime,size FROM photos WHERE %s ORDER BY id%s"
                 % (where, " LIMIT %d" % limit if limit else ""))
    total = len(rows)
    if not total:
        return {"total": 0, "done": 0, "failed": 0}
    import threading
    q = _queue()
    workers = workers or cfg.get("analyze_workers") or min(8, (os.cpu_count() or 4))
    workers = max(1, int(workers))
    tasks = [(r["id"], r["path"], cfg) for r in rows]
    done = failed = 0
    lock = threading.Lock()

    def consume(results):
        nonlocal done, failed
        for pid, res, err in results:
            if res is None:
                failed += 1
                lib.set_meta("last_error", "%s: %s" % (pid, err))
                continue
            update_from_analysis(lib, pid, res, cfg)
            done += 1
            if progress and (done % 20 == 0 or done == total):
                progress("analyze", done, total, os.path.basename(""))

    if workers == 1:
        out = []
        for t in tasks:
            if cancel is not None and cancel.is_set():
                break
            out.append(_analyze_one(t))
        consume(out)
    else:
        # chunked 以减小内存
        CH = 64
        for i in range(0, len(tasks), CH):
            if cancel is not None and cancel.is_set():
                break
            chunk = tasks[i:i + CH]
            out = q.map(_analyze_one, chunk, chunksize=4)
            consume(out)
    return {"total": total, "done": done, "failed": failed}


_MP_POOL = None


def _queue():
    """复用进程池；失败时退化为线程池。"""
    global _MP_POOL
    if _MP_POOL is not None:
        return _MP_POOL
    import multiprocessing as mp
    import concurrent.futures as cf

    def _noop(x):
        return x

    class _Adapters:
        def __init__(self):
            self.pool = None
            self.tp = None
            n = min(8, (os.cpu_count() or 4))
            self.tp = cf.ThreadPoolExecutor(max_workers=n)
            # 进程池自检：Windows 的 spawn 会重新导入主模块，
            # 若入口脚本没有 __main__ 保护就会在子进程里反复崩溃。
            # 先跑一个极小任务确认可用，不行就永久退化为线程池。
            try:
                pool = mp.Pool(processes=n)
                if list(pool.map(_noop, [0, 1, 2])) == [0, 1, 2]:
                    self.pool = pool
                else:
                    pool.terminate()
                    self.pool = None
            except Exception:
                self.pool = None

        def map(self, fn, items, chunksize=1):
            if self.pool is not None:
                try:
                    return list(self.pool.map(fn, items, chunksize))
                except Exception:
                    # 子进程出错：本次用线程重算，并永久放弃进程池
                    try:
                        self.pool.terminate()
                    except Exception:
                        pass
                    self.pool = None
            return list(self.tp.map(fn, items))

    _MP_POOL = _Adapters()
    return _MP_POOL


def reset_pool():
    global _MP_POOL
    try:
        if _MP_POOL and getattr(_MP_POOL, "pool", None):
            _MP_POOL.pool.terminate()
    except Exception:
        pass
    _MP_POOL = None


def update_from_analysis(lib: Library, pid: int, res: dict, cfg: dict | None = None):
    row = lib.photo(pid)
    if not row:
        return
    path = row["path"]
    # 时间：EXIF > 文件名/目录 > 文件修改时间
    ex = _exif_for(path)
    dt = ex.get("datetime")
    src = "exif"
    prec = "second"
    if not dt:
        dt, prec = parse_datetime(os.path.basename(path))
        src = "filename"
    if not dt:
        dt, prec = parse_datetime_from_path(os.path.dirname(path))
        src = "folder"
    if not dt:
        try:
            dt = _from_ts(row["mtime"])
            src = "fs"
            prec = "day"
        except Exception:
            dt = None
    upd = {
        "sha1": _sha1_cached(lib, path),
        "width": res["width"], "height": res["height"],
        "megapixels": res["megapixels"], "aspect": res["aspect"],
        "blur": res["blur"], "brightness": res["brightness"],
        "contrast": res["contrast"], "saturation": res["saturation"],
        "hue": res["hue"], "sepia": res["sepia"], "color": res["color"],
        "skin": res["skin"], "noise": res["noise"], "sharpness": res["sharpness"],
        "dark_ratio": res["dark_ratio"], "white_ratio": res["white_ratio"],
        "a_hash": res["a_hash"], "d_hash": res["d_hash"], "p_hash": res["p_hash"],
        "flags": "|".join(res["flags"]),
        "has_people": 1 if res["has_people"] else 0,
        "is_mono": 1 if res["is_mono"] else 0,
        "is_sepia": 1 if res["is_sepia"] else 0,
        "age_score": res["age_score"],
        "quality_score": _quality_score(res),
        "thumb": res.get("thumb"),
        "analyzed_at": time.time(),
        "missing": 0,
    }
    if dt:
        upd.update({
            "dt": iso(dt), "dt_source": src, "dt_precision": prec,
            "year": dt.year, "month": dt.month, "day": dt.day, "hour": dt.hour,
        })
    if ex.get("make"):
        upd["camera_make"] = ex["make"]
    if ex.get("model"):
        upd["camera_model"] = ex["model"]
    if ex.get("software"):
        upd["software"] = ex["software"]
    if ex.get("lat") is not None:
        upd["lat"], upd["lon"] = ex["lat"], ex["lon"]
    # 地点识别已是常驻功能（不再受实验室开关限制）
    from . import geo
    upd["place"] = geo.resolve_place(
        ex.get("lat"), ex.get("lon"),
        "%s %s" % (os.path.dirname(path), os.path.basename(path)))
    lib.upsert_photo({"path": path, **upd})


_EXIF_CACHE = {}


def _exif_for(path):
    from .exif_reader import read_exif
    try:
        return read_exif(path)
    except Exception:
        return {}


def _from_ts(ts):
    import datetime as dt
    return dt.datetime.fromtimestamp(ts)


def _sha1_cached(lib: Library, path):
    from .util import sha1_file
    try:
        return sha1_file(path)
    except Exception:
        return None


def _quality_score(res: dict) -> float:
    """0-100 的综合可用性评分：越清晰、曝光越正常越高。"""
    import math
    # 用已归一化的锐度（imaging._sharp_score 的结果）而不是原始方差
    sharp = min(1.0, max(0.0, res.get("sharpness", 0.0)) / 100.0)
    expo = 1.0 - min(1.0, abs(res["brightness"] - 0.50) / 0.50)
    contrast = min(1.0, res["contrast"] / 0.26)
    res_s = min(1.0, math.log(max(1.0, res["megapixels"] * 4.0), 2) / 5.0)
    noise = max(0.0, 1.0 - res["noise"] / 18.0)
    return round(100.0 * (0.42 * sharp + 0.20 * expo + 0.16 * contrast
                          + 0.12 * res_s + 0.10 * noise), 1)
