# -*- coding: utf-8 -*-
"""照片整理后端 HTTP API。

把已有的 Python 引擎（扫描/分类/去重/修复/整理）暴露成 REST 接口，
Tauri 前端通过 fetch 调用。随 Tauri 打包时作为独立进程启动。

接口约定：
  GET  /api/stats          统计
  GET  /api/photos         照片列表（分页、筛选、排序）
  GET  /api/photo          单张详情（含标签）
  GET  /api/tags           标签（带 group 分类）
  GET  /api/places         地点树：地点 -> 年 -> 月 -> 数量
  GET  /api/duplicates     重复组（每组直接带照片）
  GET  /api/restore/presets 修复预设与可选项
  GET  /thumb?id=          缩略图（落盘缓存）
  GET  /photo?id=          原图
  POST /api/roots          添加目录
  POST /api/scan           全流程整理
  POST /api/scan/start     后台开始扫描，配合 /api/scan/progress
  GET  /api/scan/progress  扫描进度
  POST /api/dups/clean     清理重复副本（默认移到回收目录）
  POST /api/restore        单张修复
  POST /api/restore/apply  修复并返回可预览的图片
  POST /api/organize       整理归档
"""
from __future__ import annotations

import io
import json
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, unquote, urlparse

from PIL import Image

from opo import classify, config, geo, organizer, pipeline, restore, scanner
from opo.imaging import THUMB_MAX, open_image, quality_metrics, save_thumb, thumb_key
from opo.library import Library

PORT = int(os.environ.get("OPO_PORT", "18921"))
_lib = None

# ---------------------------------------------------------------- 缩略图缓存
_THUMB_LOCK = threading.Lock()
_THUMB_MEM = {}
_THUMB_MEM_MAX = 600
# 每张图一把锁：避免同一张图被多个请求同时解码、同时写缓存
_THUMB_KEY_LOCKS = {}
_THUMB_KEY_GUARD = threading.Lock()


def _key_lock(key):
    with _THUMB_KEY_GUARD:
        lk = _THUMB_KEY_LOCKS.get(key)
        if lk is None:
            lk = threading.Lock()
            # 控制字典大小，避免无限增长
            if len(_THUMB_KEY_LOCKS) > 2000:
                _THUMB_KEY_LOCKS.clear()
            _THUMB_KEY_LOCKS[key] = lk
        return lk


def lib():
    global _lib
    if _lib is None:
        _lib = Library()
    return _lib


def rec(row):
    """sqlite Row -> JSON dict。"""
    return {k: row[k] for k in row.keys()} if row is not None else None


def _thumb_cache_path(path: str, size: int) -> str:
    try:
        mtime = os.path.getmtime(path)
    except Exception:
        mtime = 0.0
    # v2 前缀：缩略图算法换过，避免命中旧版本的畸形缓存
    return os.path.join(config.thumb_dir(),
                        "v2_" + thumb_key(path, size, mtime) + ".jpg")


def _read_thumb(path: str, size: int = 480) -> bytes | None:
    """读缩略图：优先磁盘缓存，其次内存缓存，最后现算一张并落盘。

    关键点（之前缩略图变成细条的原因）：
      * 必须先用 exif_transpose 修正方向，否则带 Orientation 的 PNG 会算错长宽；
      * 按"最长边缩到 size"之后再按比例算另一边，不能对超宽/超高的图直接
        thumbnail 成 200x13364 这种畸形尺寸；
      * 输出统一限制在 size x size 内，并做一次边界 clamp。
    """
    cache = _thumb_cache_path(path, size)
    key = (path.lower(), size)

    # 先查内存缓存，避免频繁读盘
    with _THUMB_LOCK:
        hit = _THUMB_MEM.get(key)
    if hit:
        return hit

    def _try_disk():
        """读磁盘缓存；只在文件完整时返回，残缺返回 None（不删除）。"""
        try:
            if not os.path.exists(cache):
                return None
            with open(cache, "rb") as f:
                data = f.read()
            if len(data) > 256 and data[:2] == b"\xff\xd8" and data[-2:] == b"\xff\xd9":
                return data
        except Exception:
            pass
        return None

    data = _try_disk()
    if data:
        with _THUMB_LOCK:
            _THUMB_MEM[key] = data
        return data

    # 同一张图只允许一个线程解码+写盘，其余线程等它完成后直接读
    lk = _key_lock(key)
    with lk:
        data = _try_disk()          # 等锁期间可能已被别的线程写好
        if data:
            with _THUMB_LOCK:
                _THUMB_MEM[key] = data
            return data
        # 这里绝不 os.remove(cache)：删除与 replace 并发会让别的线程读不到文件
        return _build_and_cache_thumb(path, size, key, cache)


def _build_and_cache_thumb(path: str, size: int, key, cache: str) -> bytes | None:
    """真正解码并生成缩略图（调用方必须已持有该 key 的锁）。"""
    try:
        im = Image.open(path)
        im.load()
    except Exception:
        return None

    # 方向修正必须在取尺寸之前
    try:
        from PIL import ImageOps
        im = ImageOps.exif_transpose(im)
    except Exception:
        pass

    w, h = im.size
    if not w or not h:
        return None

    if im.mode in ("RGBA", "LA", "P"):
        try:
            bg = Image.new("RGB", (w, h), (255, 255, 255))
            rgba = im.convert("RGBA")
            bg.paste(rgba, mask=rgba.split()[-1])
            im = bg
        except Exception:
            im = im.convert("RGB")
    elif im.mode != "RGB":
        try:
            im = im.convert("RGB")
        except Exception:
            return None

    # 等比缩放到 size 见方以内（长边对齐）
    ratio = min(float(size) / w, float(size) / h, 1.0)
    nw = max(1, int(round(w * ratio)))
    nh = max(1, int(round(h * ratio)))
    # 极端长图（如 1x5000 的长截图）再夹一次，避免细条
    nw = min(nw, size)
    nh = min(nh, size)
    try:
        t = im.resize((nw, nh), Image.LANCZOS)
    except Exception:
        t = im

    buf = io.BytesIO()
    try:
        t.save(buf, "JPEG", quality=84, optimize=True)
    except Exception:
        return None
    data = buf.getvalue()

    # 写入磁盘缓存必须原子化：
    # 多个请求线程可能同时为同一张图生成缩略图，
    # 直接 open(...,"wb") 会读到写了一半的文件，表现就是"图片崩坏/半截"。
    # 先写唯一临时文件，再 os.replace 原子替换。
    try:
        os.makedirs(os.path.dirname(cache), exist_ok=True)
        tmp = "%s.%d.%d.tmp" % (cache, os.getpid(), threading.get_ident())
        with open(tmp, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        os.replace(tmp, cache)
    except Exception:
        try:
            if os.path.exists(tmp):
                os.remove(tmp)
        except Exception:
            pass

    with _THUMB_LOCK:
        if len(_THUMB_MEM) > _THUMB_MEM_MAX:
            _THUMB_MEM.clear()
        _THUMB_MEM[key] = data
    return data


def _send_bytes(handler, data: bytes, mime="image/jpeg", cacheable=False):
    try:
        handler.send_response(200)
        handler.send_header("Content-Type", mime)
        handler.send_header("Content-Length", str(len(data)))
        if cacheable:
            handler.send_header("Cache-Control", "public, max-age=604800")
        else:
            # 大图必须走短缓存：重置索引后 id 会指向另一张图，
            # 长缓存会让浏览器显示旧图（"点开不是同一张"）。
            handler.send_header("Cache-Control", "no-store, must-revalidate")
            handler.send_header("Pragma", "no-cache")
        handler.send_header("Access-Control-Allow-Origin", "*")
        handler.send_header("Connection", "close")
        handler.end_headers()
        handler.wfile.write(data)
        handler.wfile.flush()
    except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
        pass


def _photo_like(row):
    """给前端补上可定位的 id 字段，避免前端拼路径。"""
    d = rec(row) if row is not None else None
    if d is not None:
        pid = d.get("id")
        if pid is not None:
            d["thumb_url"] = "/thumb?id=%d" % int(pid)
            d["photo_url"] = "/photo?id=%d" % int(pid)
    return d


# ---------------------------------------------------------------- 扫描进度
_PROGRESS = {
    "running": False,
    "phase": "",
    "done": 0,
    "total": 0,
    "detail": "",
    "percent": 0.0,
    "error": "",
    "started": 0.0,
    "finished": 0.0,
}
_PROGRESS_LOCK = threading.Lock()


def _progress_reset():
    with _PROGRESS_LOCK:
        _PROGRESS.update({"running": True, "phase": "准备中", "done": 0,
                          "total": 0, "detail": "", "percent": 0.0,
                          "error": "", "started": time.time(),
                          "finished": 0.0})


def _progress_update(phase, done=0, total=0, detail=""):
    """把各阶段进度加权换算成整体 0-100，避免每个阶段都从 1% 重新开始。

    权重按实际耗时占比给：分析照片最重（截图解码慢），
    去重和打标很轻，扫描目录只是遍历文件名。
    """
    with _PROGRESS_LOCK:
        _PROGRESS["phase"] = phase
        _PROGRESS["done"] = done
        if total:
            _PROGRESS["total"] = total
        _PROGRESS["detail"] = detail
        _PROGRESS["percent"] = _overall_percent(phase, done, total)


# 阶段 -> (起始百分比, 该阶段占的宽度)
_PHASE_SPAN = [
    ("扫描目录", 0, 10),
    ("分析照片", 10, 60),
    ("生成标签", 70, 8),
    ("识别地点", 78, 6),
    ("聚类事件", 84, 4),
    ("查找重复", 88, 11),
    ("完成", 99, 1),
]


def _overall_percent(phase, done, total):
    for prefix, start, width in _PHASE_SPAN:
        if phase.startswith(prefix) or prefix in phase:
            if phase.startswith("完成"):
                return 100.0
            if not total:
                return round(float(start), 1)
            frac = min(1.0, max(0.0, float(done) / float(total)))
            return round(start + width * frac, 1)
    return 0.0


def _progress_finish(error=""):
    with _PROGRESS_LOCK:
        _PROGRESS["running"] = False
        _PROGRESS["error"] = error or ""
        _PROGRESS["finished"] = time.time()
        if not error:
            _PROGRESS["percent"] = 100.0


def _run_scan_async(roots, force_reanalyze=False):
    def work():
        try:
            cfg = config.load_settings()
            _progress_update("扫描目录", 0, 0, "正在枚举文件…")
            # 开关切换后必须重新分析，否则旧照片的 flags/地点不会更新
            res = pipeline.run_full(lib(), cfg, roots=roots,
                                    prog=pipeline.Progress(),
                                    on_progress=_progress_update,
                                    redo=False,
                                    reanalyze_all=force_reanalyze)
            if res.get("error"):
                _progress_finish(res["error"])
                return
            st = lib().stats()
            _progress_update("完成", st.get("total", 1), st.get("total", 1),
                             "共 %d 张照片" % st.get("total", 0))
            _progress_finish()
        except Exception as e:
            _progress_finish("%s: %s" % (type(e).__name__, e))

    t = threading.Thread(target=work, daemon=True)
    t.start()


class Handler(BaseHTTPRequestHandler):
    _logged = 0

    def log_message(self, fmt, *args):
        # 只记录前 30 条，避免日志被缩略图请求刷屏
        if Handler._logged >= 30:
            return
        try:
            Handler._logged += 1
            with open(os.path.join(config.app_dir(), "bridge_access.log"), "a",
                      encoding="utf-8") as f:
                f.write("%s %s\n" % (time.strftime("%H:%M:%S"), fmt % args))
        except Exception:
            pass

    def _send(self, data, code=200):
        body = json.dumps(data, ensure_ascii=False, default=str).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json; charset=utf-8")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Access-Control-Allow-Origin", "*")
        self.end_headers()
        self.wfile.write(body)

    def do_OPTIONS(self):
        """浏览器跨域预检；不处理就会返回 501，前端表现为"后端未就绪"。"""
        self.send_response(204)
        self.send_header("Access-Control-Allow-Origin", "*")
        self.send_header("Access-Control-Allow-Methods", "GET, POST, OPTIONS")
        self.send_header("Access-Control-Allow-Headers", "Content-Type, Cache-Control")
        self.send_header("Access-Control-Max-Age", "86400")
        self.send_header("Content-Length", "0")
        self.end_headers()

    # ------------------------------------------------------------ 图片
    def _serve_image(self, want_original: bool):
        u = urlparse(self.path)
        q = parse_qs(u.query)
        path = unquote(q.get("path", [""])[0])
        if not path and q.get("id"):
            try:
                row = lib().photo(int(q["id"][0]))
                if row:
                    # 缩略图也要从原图生成：库里存的 thumb 只有 320px，
                    # 拿它当源图会在大尺寸下被放大成糊图/长条。
                    path = row["path"] or row["thumb"]
            except Exception:
                path = ""
        if not path or not os.path.exists(path):
            self.send_error(404)
            return
        if want_original:
            ext = os.path.splitext(path)[1].lower()
            mime = {".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png",
                    ".gif": "image/gif", ".webp": "image/webp", ".bmp": "image/bmp",
                    ".tif": "image/tiff", ".tiff": "image/tiff"}.get(
                        ext, "application/octet-stream")
            try:
                with open(path, "rb") as f:
                    # 原图不缓存，避免重置索引后 id 复用显示旧图
                    _send_bytes(self, f.read(), mime, cacheable=False)
            except Exception:
                self.send_error(500)
            return
        try:
            size = int(q.get("size", ["1200"])[0])
        except Exception:
            size = 1200
        # 上限放宽到 4096：大图查看需要足够分辨率，太小会显得"软件里看不清"
        size = max(64, min(size, 4096))
        # 并发下偶发读到"正在被替换"的缓存，重试几次即可拿到完整图
        data = None
        for attempt in range(5):
            data = _read_thumb(path, size)
            if data:
                break
            time.sleep(0.1 * (attempt + 1))
        if not data:
            # 记录一次诊断信息，便于定位是文件问题还是并发问题
            try:
                with open(os.path.join(config.app_dir(), "thumb_error.log"), "a",
                          encoding="utf-8") as f:
                    f.write("%s size=%s exists=%s path=%s\n" % (
                        time.strftime("%H:%M:%S"), size, os.path.exists(path), path))
            except Exception:
                pass
            self.send_error(404)
            return
        _send_bytes(self, data)

    # ------------------------------------------------------------ GET
    def do_GET(self):
        u = urlparse(self.path)
        q = parse_qs(u.query)
        p = u.path

        if p == "/thumb":
            self._serve_image(False)
            return
        if p == "/photo":
            self._serve_image(True)
            return

        try:
            if p == "/api/ping":
                self._send({"ok": True, "app": "photo-organizer", "pid": os.getpid()})
            elif p == "/api/stats":
                st = lib().stats()
                # 「未知地点」不算已识别地点，避免显示成"1 个地点"却全是未知
                st["places"] = lib().q1(
                    "SELECT COUNT(DISTINCT place) c FROM photos "
                    "WHERE missing=0 AND place IS NOT NULL AND place<>'' "
                    "AND place<>?", (geo.UNKNOWN,))["c"]
                st["unknown_place"] = lib().q1(
                    "SELECT COUNT(*) c FROM photos WHERE missing=0 AND "
                    "(place IS NULL OR place='' OR place=?)",
                    (geo.UNKNOWN,))["c"]
                self._send(st)
            elif p == "/api/photos":
                where = ["missing=0"]
                args = []
                flag = q.get("flag")
                if flag and flag[0]:
                    where.append("flags LIKE ?")
                    args.append("%" + flag[0] + "%")
                dup = q.get("dup")
                if dup and dup[0] == "only":
                    where.append("dup_group IS NOT NULL")
                elif dup and dup[0] == "none":
                    where.append("dup_group IS NULL")
                if q.get("nodate") and q["nodate"][0] in ("1", "true", "yes"):
                    where.append("(dt IS NULL OR TRIM(dt)='')")
                if q.get("dup_group"):
                    where.append("dup_group = ?")
                    args.append(int(q["dup_group"][0]))
                if q.get("fav") and q["fav"][0] in ("1", "true", "yes"):
                    where.append("favorite=1")
                if q.get("year"):
                    where.append("year = ?")
                    args.append(int(q["year"][0]))
                if q.get("month"):
                    where.append("month = ?")
                    args.append(int(q["month"][0]))
                if q.get("place"):
                    where.append("place = ?")
                    args.append(q["place"][0])
                if q.get("noplace") and q["noplace"][0] in ("1", "true", "yes"):
                    where.append("(place IS NULL OR place='' OR place=?)")
                    args.append(geo.UNKNOWN)
                if q.get("min_quality"):
                    where.append("quality_score >= ?")
                    args.append(float(q["min_quality"][0]))
                if q.get("max_quality"):
                    where.append("quality_score <= ?")
                    args.append(float(q["max_quality"][0]))
                if q.get("q"):
                    where.append("(name LIKE ? OR path LIKE ? OR place LIKE ?)")
                    kw = "%" + q["q"][0] + "%"
                    args.extend([kw, kw, kw])

                order = "dt IS NULL, dt ASC, id"
                if q.get("sort"):
                    order = {
                        "dt ASC": "dt IS NULL, dt ASC, id",
                        "dt DESC": "dt IS NULL, dt DESC, id DESC",
                        "quality_score DESC": "quality_score DESC, id",
                        "quality_score ASC": "quality_score ASC, id",
                        "age_score DESC": "age_score DESC, id",
                    }.get(q["sort"][0], order)

                w = " AND ".join(where)
                total = lib().count(w, tuple(args))
                try:
                    limit = int(q.get("limit", ["200"])[0])
                except Exception:
                    limit = 200
                try:
                    offset = int(q.get("offset", ["0"])[0])
                except Exception:
                    offset = 0
                limit = max(1, min(limit, 2000))
                offset = max(0, offset)
                # 直接在 SQL 里分页，避免每次都把全部行读进内存
                rows = lib().q("SELECT * FROM photos WHERE %s ORDER BY %s LIMIT ? OFFSET ?"
                               % (w, order), tuple(args) + (limit, offset))
                self._send({"items": [_photo_like(r) for r in rows],
                            "total": total, "offset": offset,
                            "limit": limit,
                            "has_more": offset + len(rows) < total})
            elif p == "/api/photo":
                try:
                    row = lib().photo(int(q.get("id", ["0"])[0]))
                except Exception:
                    row = None
                if row is None:
                    self._send({"error": "not found"}, 404)
                else:
                    d = _photo_like(row)
                    d["tags"] = lib().tags_of(row["id"])
                    self._send(d)
            elif p == "/api/tags":
                items = []
                for n, k, c in lib().all_tags():
                    if k == "auto":
                        if n in ("模糊", "过暗", "过亮", "低分辨率", "褪色平淡",
                                 "泛黄/棕褐", "噪点重", "高光过曝", "大面积死黑",
                                 "灰白/单色"):
                            g = "质量"
                        elif n.startswith("相机:"):
                            g = "相机"
                        else:
                            g = "特征"
                    elif k == "event":
                        g = "事件"
                    elif k == "time":
                        g = "时间"
                    else:
                        g = "自定义"
                    items.append({"name": n, "kind": k, "count": c, "group": g})
                self._send({"items": items})
            elif p == "/api/settings":
                self._send({"settings": config.load_settings()})
            elif p == "/api/timeline":
                self._send({"items": _timeline_tree()})
            elif p == "/api/places":
                tree = _place_tree()
                known = [x for x in tree if x["place"] != geo.UNKNOWN]
                self._send({"items": tree, "known_count": len(known)})
            elif p == "/api/events":
                rows = lib().q("SELECT * FROM events ORDER BY start_dt DESC")
                self._send({"items": [rec(r) for r in rows]})
            elif p == "/api/duplicates":
                self._send({"items": _dup_groups(), "total": lib().q1(
                    "SELECT COUNT(*) c FROM dup_groups")["c"]})
            elif p == "/api/roots":
                rs = lib().roots()
                self._send({"items": rs, "total": len(rs)})
            elif p == "/api/scan/progress":
                with _PROGRESS_LOCK:
                    self._send(dict(_PROGRESS))
            elif p == "/api/restore/presets":
                self._send({"presets": list(restore.PRESETS.keys()),
                            "options": restore.OPTIONS,
                            "defaults": restore.preset_options("标准修复")})
            else:
                self._send({"error": "not found"}, 404)
        except Exception as e:
            self._send({"error": str(e)}, 500)

    # ------------------------------------------------------------ POST
    def do_POST(self):
        u = urlparse(self.path)
        try:
            length = int(self.headers.get("Content-Length", "0"))
        except Exception:
            length = 0
        raw = self.rfile.read(length) if length else b"{}"
        try:
            body = json.loads(raw or b"{}")
        except Exception:
            body = {}

        try:
            if u.path == "/api/settings":
                cur = config.load_settings()
                incoming = body.get("settings") or {}
                changed = False
                for k, v in incoming.items():
                    if k not in config.DEFAULT_SETTINGS:
                        continue
                    old = config.DEFAULT_SETTINGS[k]
                    try:
                        cur[k] = int(v) if isinstance(old, int) else float(v)
                        changed = True
                    except Exception:
                        continue
                if changed:
                    config.save_settings(cur)
                self._send({"ok": changed, "settings": cur})
            elif u.path == "/api/scan":
                roots = body.get("roots") or lib().roots()
                _progress_reset()
                res = pipeline.run_full(lib(), config.load_settings(), roots=roots,
                                        prog=pipeline.Progress(),
                                        on_progress=_progress_update)
                if res.get("error"):
                    _progress_finish(res["error"])
                else:
                    _progress_update("完成", 1, 1, "整理完成")
                    _progress_finish()
                self._send(res)
            elif u.path == "/api/scan/start":
                with _PROGRESS_LOCK:
                    busy = _PROGRESS["running"]
                if busy:
                    self._send({"ok": False, "error": "已有扫描任务在进行中"})
                    return
                roots = body.get("roots") or lib().roots()
                if not roots:
                    self._send({"ok": False, "error": "还没有添加照片目录"})
                    return
                # 每次「开始扫描」都按当前实验室开关重算，
                # 否则开关切换后旧照片不会带上新结果（用户会以为没生效）
                _progress_reset()
                _run_scan_async(roots, force_reanalyze=True)
                self._send({"ok": True})
            elif u.path == "/api/places/backfill":
                _progress_reset()
                # 界面上点「重新识别地点」= 全量重算，旧的经纬度名会被换成城市名
                res = classify.backfill_places(lib(), _progress_update,
                                               redo=bool(body.get("redo", True)))
                _progress_finish()
                self._send({"ok": True, **res})
            elif u.path == "/api/dups/clean":
                self._send(_clean_duplicates(body))
            elif u.path == "/api/photos/delete":
                self._send(_delete_photos(body))
            elif u.path == "/api/reclassify":
                self._send(_reclassify_flags())
            elif u.path == "/api/favorite":
                self._send(_toggle_favorite(body))
            elif u.path == "/api/index/reset":
                self._send(_reset_index(body))
            elif u.path == "/api/restore":
                self._send(_restore_one(body))
            elif u.path == "/api/restore/apply":
                self._send(_restore_apply(body))
            elif u.path == "/api/restore/batch":
                self._send(_restore_batch(body))
            elif u.path == "/api/organize":
                plan = organizer.plan_organize(
                    lib(), scheme=body.get("scheme", "year_month"),
                    template=body.get("template", "{date}_{name}"),
                    parts=body.get("parts"),
                    favorite_dir=bool(body.get("favorite_dir", False)),
                    favorite_in_regular=bool(body.get("favorite_in_regular", True)))
                if body.get("dry_run", True):
                    self._send({"plan": plan[:500], "total": len(plan)})
                else:
                    dest = body.get("dest") or ""
                    if not dest:
                        self._send({"error": "未指定输出目录"})
                        return
                    res = organizer.execute_organize(
                        lib(), dest, plan, mode=body.get("mode", "copy"))
                    self._send({k: v for k, v in res.items() if k != "log_ids"})
            elif u.path == "/api/roots":
                path = (body.get("path") or "").strip().strip('"')
                if not path:
                    self._send({"ok": False, "error": "路径为空"})
                    return
                if not os.path.isdir(path):
                    self._send({"ok": False, "error": "目录不存在：" + path})
                    return
                lib().add_root(path)
                self._send({"ok": True, "path": path, "roots": lib().roots()})
            elif u.path == "/api/roots/remove":
                path = (body.get("path") or "").strip()
                if not path:
                    self._send({"ok": False, "error": "路径为空"})
                    return
                lib().remove_root(path)
                self._send({"ok": True, "roots": lib().roots()})
            else:
                self._send({"error": "not found"}, 404)
        except Exception as e:
            import traceback
            self._send({"error": str(e), "trace": traceback.format_exc()[-800:]}, 500)


# ---------------------------------------------------------------- 收藏
def _toggle_favorite(body):
    """收藏/取消收藏。body: {id, favorite: true/false}"""
    pid = int(body.get("id") or 0)
    L = lib()
    row = L.photo(pid)
    if row is None:
        return {"ok": False, "error": "没有这张照片"}
    want = body.get("favorite")
    if want is None:
        want = not bool(row["favorite"])
    L.ex("UPDATE photos SET favorite=? WHERE id=?", (1 if want else 0, pid))
    return {"ok": True, "id": pid, "favorite": bool(want)}


def _reset_index(body):
    """清除扫描索引：删掉所有照片记录与缩略图缓存，保留根目录列表。

    用于"照片库乱掉了 / 想从头重新识别"的场景，
    与照片修复的"清除记录"完全无关。
    """
    keep_roots = body.get("keep_roots", True)
    L = lib()
    try:
        before = L.q1("SELECT COUNT(*) c FROM photos")["c"]
        L.ex("DELETE FROM photo_tags")
        L.ex("DELETE FROM events")
        L.ex("DELETE FROM dup_groups")
        L.ex("DELETE FROM photos")
        try:
            L.ex("DELETE FROM sqlite_sequence WHERE name IN ('photos','events','dup_groups')")
        except Exception:
            pass
        if not keep_roots:
            L.ex("DELETE FROM roots")
        # 清空缩略图缓存，避免旧键残留
        import shutil
        try:
            shutil.rmtree(config.thumb_dir(), ignore_errors=True)
        except Exception:
            pass
        try:
            os.makedirs(config.thumb_dir(), exist_ok=True)
        except Exception:
            pass
        with _THUMB_LOCK:
            _THUMB_MEM.clear()
        return {"ok": True, "removed": before,
                "roots_kept": L.roots() if keep_roots else []}
    except Exception as e:
        return {"ok": False, "error": str(e)}


# ---------------------------------------------------------------- 重新判定
def _reclassify_flags():
    """用当前判定标准重算所有照片的质量标记，不需要重新解码图片。

    只读库里已有的指标（blur/noise/contrast 等）重新套一遍 classify_flags，
    所以既快又能让"改了标准"立刻生效。
    """
    from opo import imaging

    cfg = config.load_settings()
    L = lib()
    rows = L.q("SELECT id,path,blur,brightness,contrast,saturation,sepia,color,"
               "noise,dark_ratio,white_ratio,width,height,megapixels,sharpness "
               "FROM photos WHERE missing=0")
    updates = []
    for r in rows:
        m = {k: r[k] for k in (
            "blur", "brightness", "contrast", "saturation", "sepia", "color",
            "noise", "dark_ratio", "white_ratio")}
        m["megapixels"] = r["megapixels"] or (
            ((r["width"] or 0) * (r["height"] or 0)) / 1_000_000.0)
        m["sharpness"] = r["sharpness"] or 0
        try:
            im = None
            # 截图判定需要看像素，但只读缩略图就够，成本很低
            if r["path"] and os.path.exists(r["path"]):
                im = Image.open(r["path"])
                im.thumbnail((160, 160))
            screen_like = imaging.is_screen_like(im, m) if im is not None else False
        except Exception:
            screen_like = False
        flags = imaging.classify_flags(m, cfg, screen_like=screen_like)
        # 开关关闭时清空标记，避免旧库残留
        if not cfg.get("lab_quality", False):
            flags = []
        updates.append(("|".join(flags), r["id"]))
    if updates:
        L.conn.executemany("UPDATE photos SET flags=? WHERE id=?", updates)
        L.conn.commit()
    with_flag = sum(1 for f, _ in updates if f)
    return {"ok": True, "total": len(updates), "flagged": with_flag,
            "clean": len(updates) - with_flag}


# ---------------------------------------------------------------- 时间轴
def _timeline_tree():
    """年份 -> 月份 -> 数量。给时间轴一次拿全，避免前端首屏空白。"""
    rows = lib().q(
        "SELECT year, month, COUNT(*) c FROM photos "
        "WHERE missing=0 AND year IS NOT NULL "
        "GROUP BY year, month ORDER BY year DESC, month ASC")
    tree = {}
    for r in rows:
        y = int(r["year"])
        node = tree.setdefault(y, {"year": y, "count": 0, "months": []})
        node["count"] += r["c"]
        if r["month"]:
            node["months"].append({"month": int(r["month"]), "count": r["c"]})
    return sorted(tree.values(), key=lambda x: -x["year"])


# ---------------------------------------------------------------- 地点树
def _place_tree():
    rows = lib().q(
        "SELECT COALESCE(NULLIF(place,''),?) AS p, year, month, COUNT(*) c "
        "FROM photos WHERE missing=0 GROUP BY p, year, month "
        "ORDER BY c DESC", (geo.UNKNOWN,))
    tree = {}
    for r in rows:
        place = r["p"]
        parent, leaf = geo.place_parts(place)
        node = tree.setdefault(place, {"place": place, "parent": parent,
                                       "leaf": leaf, "count": 0, "years": {}})
        node["count"] += r["c"]
        if r["year"]:
            y = node["years"].setdefault(int(r["year"]), {"year": int(r["year"]),
                                                         "count": 0, "months": {}})
            y["count"] += r["c"]
            if r["month"]:
                y["months"][int(r["month"])] = y["months"].get(int(r["month"]), 0) + r["c"]
    out = []
    for place, node in sorted(tree.items(),
                              key=lambda kv: (kv[0] == geo.UNKNOWN, -kv[1]["count"])):
        years = []
        for y, yn in sorted(node["years"].items(), reverse=True):
            years.append({"year": y, "count": yn["count"],
                          "months": [{"month": m, "count": c}
                                     for m, c in sorted(yn["months"].items())]})
        out.append({"place": place, "parent": node["parent"],
                    "leaf": node["leaf"], "count": node["count"], "years": years})
    return out


# ---------------------------------------------------------------- 重复组
def _dup_groups():
    groups = lib().q("SELECT * FROM dup_groups ORDER BY saved_bytes DESC")
    if not groups:
        return []
    ids = [g["id"] for g in groups]
    marks = ",".join("?" * len(ids))
    photos = lib().q(
        "SELECT * FROM photos WHERE dup_group IN (%s) AND missing=0 "
        "ORDER BY dup_group, is_best DESC, quality_score DESC" % marks,
        tuple(ids))
    by_group = {}
    for r in photos:
        by_group.setdefault(r["dup_group"], []).append(_photo_like(r))
    out = []
    for g in groups:
        d = rec(g)
        d["photos"] = by_group.get(g["id"], [])
        if not d["photos"]:
            continue
        out.append(d)
    return out


def _clean_duplicates(body):
    keep_all = bool(body.get("keep_all"))
    group_ids = body.get("groups") or []
    where = "dup_group IS NOT NULL AND is_best=0 AND missing=0"
    args = []
    if not keep_all:
        if not group_ids:
            return {"ok": False, "error": "未选择要清理的重复组"}
        where += " AND dup_group IN (%s)" % ",".join("?" * len(group_ids))
        args.extend(group_ids)

    dest = (body.get("dest") or "").strip()
    permanent = bool(body.get("delete"))
    recycle = dest or os.path.join(config.app_dir(), "recycle")
    L = lib()
    rows = L.q("SELECT * FROM photos WHERE " + where, tuple(args))
    moved = removed = errors = 0
    freed = 0
    for r in rows:
        src = r["path"]
        if not src or not os.path.exists(src):
            L.ex("UPDATE photos SET missing=1 WHERE id=?", (r["id"],))
            errors += 1
            continue
        try:
            size = os.path.getsize(src)
            if permanent:
                os.remove(src)
                L.ex("DELETE FROM photo_tags WHERE photo_id=?", (r["id"],))
                L.ex("DELETE FROM photos WHERE id=?", (r["id"],))
                removed += 1
            else:
                os.makedirs(recycle, exist_ok=True)
                base = os.path.basename(src)
                dst = os.path.join(recycle, base)
                stem, ext = os.path.splitext(dst)
                k = 1
                while os.path.exists(dst):
                    dst = "%s_%d%s" % (stem, k, ext)
                    k += 1
                import shutil
                shutil.move(src, dst)
                L.ex("UPDATE photos SET path=?,folder=?,missing=1 WHERE id=?",
                     (dst, os.path.dirname(dst), r["id"]))
                L.log_op("dedup_move", src, dst, "清理重复副本")
                moved += 1
            freed += size
        except Exception:
            errors += 1
    L.ex("DELETE FROM photo_tags WHERE source='dup'")
    _refresh_dup_groups(L)
    return {"ok": True, "moved": moved, "removed": removed, "errors": errors,
            "freed": freed, "dest": recycle, "permanent": permanent}


def _delete_photos(body):
    """按 id 逐张删除（移到回收目录）。用于重复照片页的自选删除。"""
    ids = body.get("ids") or []
    if not ids:
        return {"ok": False, "error": "没有选中照片"}
    dest = (body.get("dest") or "").strip()
    recycle = dest or os.path.join(config.app_dir(), "recycle")
    permanent = bool(body.get("permanent"))
    L = lib()
    moved = removed = errors = 0
    freed = 0
    for pid in ids:
        try:
            row = L.photo(int(pid))
        except Exception:
            row = None
        if row is None:
            errors += 1
            continue
        src = row["path"]
        if not src or not os.path.exists(src):
            L.ex("UPDATE photos SET missing=1 WHERE id=?", (row["id"],))
            errors += 1
            continue
        try:
            size = os.path.getsize(src)
            if permanent:
                os.remove(src)
                L.ex("DELETE FROM photo_tags WHERE photo_id=?", (row["id"],))
                L.ex("DELETE FROM photos WHERE id=?", (row["id"],))
                removed += 1
            else:
                os.makedirs(recycle, exist_ok=True)
                base = os.path.basename(src)
                dst = os.path.join(recycle, base)
                stem, ext = os.path.splitext(dst)
                k = 1
                while os.path.exists(dst):
                    dst = "%s_%d%s" % (stem, k, ext)
                    k += 1
                import shutil
                shutil.move(src, dst)
                L.ex("UPDATE photos SET path=?,folder=?,missing=1 WHERE id=?",
                     (dst, os.path.dirname(dst), row["id"]))
                L.log_op("delete_move", src, dst, "重复照片删除")
                moved += 1
            freed += size
        except Exception:
            errors += 1
    _refresh_dup_groups(L)
    return {"ok": True, "moved": moved, "removed": removed, "errors": errors,
            "freed": freed, "dest": recycle}


def _refresh_dup_groups(L):
    """清理后重算每组张数与可省空间；只剩 1 张的组直接移除。"""
    for g in L.q("SELECT id FROM dup_groups"):
        rows = L.q("SELECT id, size FROM photos WHERE dup_group=? AND missing=0",
                   (g["id"],))
        if len(rows) < 2:
            L.ex("UPDATE photos SET dup_group=NULL, is_best=0 WHERE dup_group=?", (g["id"],))
            L.ex("DELETE FROM dup_groups WHERE id=?", (g["id"],))
            continue
        sizes = sorted([r["size"] or 0 for r in rows], reverse=True)
        saved = max(0, sum(sizes) - sizes[0])
        L.ex("UPDATE dup_groups SET count=?, saved_bytes=? WHERE id=?",
             (len(rows), saved, g["id"]))


# ---------------------------------------------------------------- 修复
def _restore_opts(body):
    preset = body.get("preset") or "标准修复"
    opts = dict(restore.PRESETS.get(preset, {}))
    custom = body.get("options")
    if isinstance(custom, dict):
        opts.update({k: v for k, v in custom.items() if k in restore.OPTIONS})
    return preset, opts


def _load_for_restore(row):
    im = open_image(row["path"])
    if im is None:
        return None, None
    m = quality_metrics(im)
    return im, m


def _restore_one(body):
    pid = int(body.get("id") or 0)
    row = lib().photo(pid)
    if row is None:
        return {"error": "没有这张照片"}
    im, m = _load_for_restore(row)
    if im is None:
        return {"error": "这张图无法解码"}
    _, opts = _restore_opts(body)
    out = restore.auto_enhance(im, m, opts)
    out_path = body.get("out") or os.path.join(
        config.app_dir(), "restored_%d.jpg" % pid)
    out.save(out_path, "JPEG", quality=95)
    after = quality_metrics(out)
    return {"id": pid, "path": out_path,
            "path_url": "/photo?path=" + _quote(out_path),
            "before": m, "after": after,
            "gain": _restore_gain(m, after)}


def _restore_apply(body):
    """修复后直接返回 JPEG 预览，前端无需再取一次文件。"""
    pid = int(body.get("id") or 0)
    row = lib().photo(pid)
    if row is None:
        return {"error": "没有这张照片"}
    im, m = _load_for_restore(row)
    if im is None:
        return {"error": "这张图无法解码"}
    _, opts = _restore_opts(body)
    screen_like = restore._is_screen_or_document(im, m)
    out = restore.auto_enhance(im, m, opts)
    buf = io.BytesIO()
    prev = out.copy()
    prev.thumbnail((1400, 1400), Image.LANCZOS)
    prev.save(buf, "JPEG", quality=90)
    import base64
    b64 = base64.b64encode(buf.getvalue()).decode("ascii")
    after = quality_metrics(out)
    return {"id": pid, "image": "data:image/jpeg;base64," + b64,
            "before": m, "after": after, "gain": _restore_gain(m, after),
            "screen_like": screen_like}


def _restore_batch(body):
    ids = body.get("ids") or []
    dest = (body.get("dest") or "").strip()
    if not ids:
        return {"ok": False, "error": "没有选中照片"}
    if not dest:
        return {"ok": False, "error": "请先选择输出文件夹"}
    os.makedirs(dest, exist_ok=True)
    _, opts = _restore_opts(body)
    done = errors = 0
    for pid in ids:
        try:
            row = lib().photo(int(pid))
            if row is None:
                errors += 1
                continue
            im, m = _load_for_restore(row)
            if im is None:
                errors += 1
                continue
            out = restore.auto_enhance(im, m, opts)
            name = os.path.splitext(row["name"] or ("photo_%d" % pid))[0]
            out.save(os.path.join(dest, name + "_修复.jpg"), "JPEG", quality=95)
            done += 1
        except Exception:
            errors += 1
    return {"ok": True, "done": done, "errors": errors, "dest": dest}


def _restore_gain(before, after):
    def pct(a, b):
        try:
            return round((float(b) / max(float(a), 1e-6) - 1.0) * 100.0, 1)
        except Exception:
            return 0.0
    return {"contrast": pct(before.get("contrast"), after.get("contrast")),
            "detail": pct(before.get("blur"), after.get("blur")),
            "sepia": round((1.0 - float(after.get("sepia", 0) or 0) /
                            max(float(before.get("sepia", 0) or 0), 1e-6)) * 100.0, 1)}


def _quote(s):
    from urllib.parse import quote
    return quote(s, safe="")


# ---------------------------------------------------------------- 启动
def _already_running(port: int) -> bool:
    """端口上已经有本程序的后端在跑时直接退出，避免两个实例互相抢端口。"""
    import socket
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.settimeout(0.6)
    try:
        if s.connect_ex(("127.0.0.1", port)) != 0:
            return False
    finally:
        s.close()
    import urllib.request
    try:
        with urllib.request.urlopen("http://127.0.0.1:%d/api/ping" % port,
                                    timeout=1.5) as r:
            return r.status == 200
    except Exception:
        return False


class _Server(ThreadingHTTPServer):
    allow_reuse_address = True
    daemon_threads = True
    # 每个请求一条连接：图片请求量大且长短不一，
    # 复用连接反而容易在 WebView 里出现"图片加载一半失败"
    protocol_version = "HTTP/1.0"

    def handle_error(self, request, client_address):
        import sys
        et = sys.exc_info()[0]
        if et in (ConnectionResetError, BrokenPipeError, ConnectionAbortedError):
            return
        super().handle_error(request, client_address)


def _watch_parent():
    """看门狗：主进程（Tauri）退出后自动结束自己。

    设置 OPO_NO_WATCH=1 可关闭（仅用于开发/自动化测试）。
    """
    if os.name != "nt" or os.environ.get("OPO_NO_WATCH") == "1":
        return
    try:
        import ctypes
        from ctypes import wintypes

        k32 = ctypes.windll.kernel32
        k32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        k32.OpenProcess.restype = wintypes.HANDLE
        k32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        k32.WaitForSingleObject.restype = wintypes.DWORD

        parent = os.getppid()
        h = k32.OpenProcess(0x1000 | 0x00100000, False, parent)
        if not h:
            return
    except Exception:
        return

    def loop():
        while True:
            try:
                if k32.WaitForSingleObject(h, 1000) == 0:
                    os._exit(0)
            except Exception:
                os._exit(0)

    threading.Thread(target=loop, daemon=True).start()


def main():
    if _already_running(PORT):
        return 0
    _watch_parent()
    try:
        srv = _Server(("127.0.0.1", PORT), Handler)
    except OSError:
        return 1
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        srv.server_close()
    return 0


if __name__ == "__main__":
    main()
