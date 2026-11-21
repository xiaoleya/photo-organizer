# -*- coding: utf-8 -*-
"""通用工具：哈希、文件名安全化、日期解析、格式化。"""
from __future__ import annotations

import hashlib
import math
import os
import re

IMG_EXTS = {
    ".jpg", ".jpeg", ".jpe", ".jfif", ".png", ".bmp", ".gif",
    ".tif", ".tiff", ".webp", ".heic", ".heif", ".avif",
}
RAW_EXTS = {".cr2", ".cr3", ".nef", ".arw", ".dng", ".raf", ".orf", ".rw2"}

_ILLEGAL = re.compile(r'[\\/:*?"<>|\r\n\t]')
_WS = re.compile(r"\s+")

# 1826 年尼切福尔·涅普斯第一张永久照片的年份，作为合理下界
MIN_YEAR = 1826


def sha1_file(path: str, chunk: int = 1 << 20) -> str:
    h = hashlib.sha1()
    with open(path, "rb") as f:
        while True:
            b = f.read(chunk)
            if not b:
                break
            h.update(b)
    return h.hexdigest()


def sha1_text(text: str) -> str:
    return hashlib.sha1(text.encode("utf-8", "replace")).hexdigest()


def is_image(path: str) -> bool:
    return os.path.splitext(path)[1].lower() in IMG_EXTS


def safe_name(s: str, maxlen: int = 90) -> str:
    """去掉 Windows 非法字符，压缩空白，限制长度（保留扩展名）。"""
    s = _ILLEGAL.sub("_", s or "").strip(" .")
    s = _WS.sub(" ", s)
    if not s:
        s = "untitled"
    if len(s) > maxlen:
        stem, ext = os.path.splitext(s)
        if ext and len(ext) <= 12:
            s = stem[: maxlen - len(ext)].rstrip(" .") + ext
        else:
            s = s[:maxlen].rstrip(" .")
    return s


def human_size(n) -> str:
    try:
        n = float(n)
    except Exception:
        return "-"
    for unit in ("B", "KB", "MB", "GB", "TB"):
        if n < 1024 or unit == "TB":
            return ("%.0f %s" % (n, unit)) if unit == "B" else ("%.1f %s" % (n, unit))
        n /= 1024.0
    return "-"


def unique_path(path: str, taken: set | None = None) -> str:
    """同目录下生成不冲突的路径：name.jpg -> name-1.jpg。"""
    if taken is None:
        taken = set()
    cand = path
    stem, ext = os.path.splitext(path)
    i = 1
    while os.path.exists(cand) or cand.lower() in taken:
        cand = "%s-%d%s" % (stem, i, ext)
        i += 1
    taken.add(cand.lower())
    return cand


# ---------------------------------------------------------------- 日期解析

_date_re = re.compile(
    r"(?<!\d)(18\d{2}|19\d{2}|20\d{2})"
    r"(?:[-_.年/]?(\d{1,2})[-_.月/]?(\d{1,2})日?)?"
    r"(?:[ _T\-]?(\d{1,2})[时:._\-]?(\d{1,2})[分:._\-]?(\d{1,2})秒?)?"
    r"(?!\d)"
)

# 常见相机/手机导出文件名里的紧凑时间戳
_ts_re = re.compile(r"(18\d{2}|19\d{2}|20\d{2})(\d{2})(\d{2})[_\-. T]?(\d{2})(\d{2})(\d{2})")
# 微信 / 截图 / 导出常见写法
_compact_re = re.compile(r"(?<!\d)(18\d{2}|19\d{2}|20\d{2})(\d{2})(\d{2})(?!\d)")
_year_only_re = re.compile(r"^(18\d{2}|19\d{2}|20\d{2})$")


def _valid(y, mo, d, h=0, mi=0, s=0):
    import datetime as _dt

    try:
        if not (MIN_YEAR <= int(y) <= _dt.datetime.now().year + 1):
            return None
        mo = int(mo) if mo else 1
        d = int(d) if d else 1
        if not (1 <= mo <= 12):
            return None
        if not (1 <= d <= 31):
            return None
        if not (0 <= int(h) <= 23 and 0 <= int(mi) <= 59 and 0 <= int(s) <= 59):
            return None
        return _dt.datetime(int(y), mo, d, int(h), int(mi), int(s))
    except Exception:
        return None


def parse_datetime(text: str):
    """从文件名 / 目录名里推断拍摄时间。

    返回 (datetime|None, precision)，precision ∈ {'second','day','month','year'}。
    优先级：带时分的紧凑时间戳 > 完整日期时间 > 紧凑日期 > 年月日（含中文） > 年。
    """
    if not text:
        return None, None
    name = os.path.basename(text)

    m = _ts_re.search(name)
    if m:
        y, mo, d, h, mi, s = m.groups()
        dt = _valid(y, mo, d, h, mi, s)
        if dt:
            return dt, "second"

    for m in _date_re.finditer(name):
        y, mo, d, h, mi, s = m.groups()
        if mo is None:
            dt = _valid(y, 1, 1)
            if dt:
                return dt, "year"
            continue
        dt = _valid(y, mo, d, h or 0, mi or 0, s or 0)
        if not dt:
            continue
        if h is not None and s is not None:
            return dt, "second"
        if h is not None:
            return dt, "second"
        if d is not None:
            return dt, "day"
        return dt, "month"

    m = _compact_re.search(name)
    if m:
        y, mo, d = m.groups()
        dt = _valid(y, mo, d)
        if dt:
            return dt, "day"

    m = _year_only_re.match(name.strip())
    if m:
        dt = _valid(m.group(1), 1, 1)
        if dt:
            return dt, "year"
    return None, None


def parse_datetime_from_path(path: str):
    """从完整路径逐级向上找日期线索（老照片常按年份分文件夹）。"""
    parts = []
    p = path
    for _ in range(6):
        head, tail = os.path.split(p)
        if tail:
            parts.append(tail)
        if not head or head == p:
            break
        p = head
    for seg in parts:
        dt, prec = parse_datetime(seg)
        if dt:
            return dt, prec
    return None, None


def iso(dt):
    return dt.strftime("%Y-%m-%d %H:%M:%S") if dt else None


def parse_iso(s):
    import datetime as _dt

    if not s:
        return None
    for fmt in ("%Y-%m-%d %H:%M:%S", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d"):
        try:
            return _dt.datetime.strptime(str(s)[:19], fmt)
        except Exception:
            continue
    return None


def haversine_km(lat1, lon1, lat2, lon2) -> float:
    try:
        r = 6371.0
        p1, p2 = math.radians(lat1), math.radians(lat2)
        dp = p2 - p1
        dl = math.radians(lon2 - lon1)
        a = math.sin(dp / 2) ** 2 + math.cos(p1) * math.cos(p2) * math.sin(dl / 2) ** 2
        return 2 * r * math.asin(min(1.0, math.sqrt(a)))
    except Exception:
        return 0.0


def fmt_seconds(sec) -> str:
    try:
        sec = int(sec)
    except Exception:
        return "-"
    h, rem = divmod(sec, 3600)
    m, s = divmod(rem, 60)
    if h:
        return "%d 小时 %d 分" % (h, m)
    if m:
        return "%d 分 %d 秒" % (m, s)
    return "%d 秒" % s
