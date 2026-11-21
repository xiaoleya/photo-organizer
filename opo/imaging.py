# -*- coding: utf-8 -*-
"""图像度量：缩略图、质量评分、感知哈希、老照片特征检测。

只依赖 Pillow + numpy。所有函数对损坏文件返回 None，不抛异常。
"""
from __future__ import annotations

import hashlib
import math

import numpy as np
from PIL import Image, ImageOps

try:
    Image.MAX_IMAGE_PIXELS = 400_000_000
except Exception:
    pass

THUMB_MAX = 320


def open_image(path: str):
    """打开并修正方向。返回 RGB Image 或 None。"""
    try:
        im = Image.open(path)
        im.load()
    except Exception:
        return None
    try:
        im = ImageOps.exif_transpose(im)
    except Exception:
        pass
    if im.mode in ("RGBA", "LA", "P"):
        try:
            bg = Image.new("RGB", im.size, (255, 255, 255))
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
    return im


def to_array(im, max_side=0):
    if max_side and max(im.size) > max_side:
        im = im.copy()
        im.thumbnail((max_side, max_side), Image.LANCZOS)
    return np.asarray(im, dtype=np.uint8)


def save_thumb(im, out_path: str, size=THUMB_MAX, quality=82) -> bool:
    try:
        t = im.copy()
        t.thumbnail((size, size), Image.LANCZOS)
        t.save(out_path, "JPEG", quality=quality, optimize=True, progressive=True)
        return True
    except Exception:
        return False


# ------------------------------------------------------------------ 感知哈希

def ahash(im, size=8) -> int:
    g = np.asarray(im.convert("L").resize((size, size), Image.LANCZOS), dtype=np.float32)
    m = g.mean()
    bits = 0
    for v in g.flatten():
        bits = (bits << 1) | (1 if v > m else 0)
    return bits


def dhash(im, size=8) -> int:
    g = np.asarray(im.convert("L").resize((size + 1, size), Image.LANCZOS), dtype=np.int16)
    diff = g[:, 1:] > g[:, :-1]
    bits = 0
    for v in diff.flatten():
        bits = (bits << 1) | (1 if v else 0)
    return bits


def phash(im, hash_size=8, highfreq=4) -> int:
    n = hash_size * highfreq
    g = np.asarray(im.convert("L").resize((n, n), Image.LANCZOS), dtype=np.float32)
    d = _dct2(g)
    low = d[:hash_size, :hash_size].flatten()
    med = np.median(low[1:])
    bits = 0
    for v in low:
        bits = (bits << 1) | (1 if v > med else 0)
    return bits


def _dct_matrix(n):
    k = np.arange(n).reshape(-1, 1)
    x = np.arange(n).reshape(1, -1)
    m = np.cos(np.pi * (2 * x + 1) * k / (2.0 * n))
    m[0, :] *= 1.0 / math.sqrt(2.0)
    return m * math.sqrt(2.0 / n)


_DCT_CACHE = {}


def _dct2(a):
    n = a.shape[0]
    m = _DCT_CACHE.get(n)
    if m is None:
        m = _dct_matrix(n)
        _DCT_CACHE[n] = m
    return m @ a @ m.T


def hamming(a: int, b: int) -> int:
    return bin(int(a) ^ int(b)).count("1")


def hexhash(v: int) -> str:
    return "%016x" % (int(v) & 0xFFFFFFFFFFFFFFFF)


# ------------------------------------------------------------------ 质量度量

def _laplacian_var(gray: np.ndarray) -> float:
    a = gray.astype(np.float32)
    if a.shape[0] < 3 or a.shape[1] < 3:
        return 0.0
    lap = (
        -4.0 * a[1:-1, 1:-1]
        + a[:-2, 1:-1] + a[2:, 1:-1] + a[1:-1, :-2] + a[1:-1, 2:]
    )
    return float(lap.var())


def _rgb_to_hsv(arr: np.ndarray):
    r = arr[:, :, 0].astype(np.float32) / 255.0
    g = arr[:, :, 1].astype(np.float32) / 255.0
    b = arr[:, :, 2].astype(np.float32) / 255.0
    mx = np.maximum(np.maximum(r, g), b)
    mn = np.minimum(np.minimum(r, g), b)
    df = mx - mn
    v = mx
    s = np.where(mx > 1e-6, df / np.maximum(mx, 1e-6), 0.0)
    h = np.zeros_like(mx)
    m = df > 1e-6
    idx = m & (mx == r)
    h[idx] = ((g[idx] - b[idx]) / df[idx]) % 6.0
    idx = m & (mx == g)
    h[idx] = ((b[idx] - r[idx]) / df[idx]) + 2.0
    idx = m & (mx == b)
    h[idx] = ((r[idx] - g[idx]) / df[idx]) + 4.0
    h = h * 60.0
    return h, s, v


def quality_metrics(im, work_side=640) -> dict:
    """对一张图算全部质量/风格指标。"""
    w, h = im.size
    mpx = (w * h) / 1_000_000.0
    arr = to_array(im, work_side)
    gray = np.asarray(Image.fromarray(arr).convert("L"), dtype=np.uint8)
    gf = gray.astype(np.float32) / 255.0

    lap = _laplacian_var(gray)
    mean = float(gf.mean())
    std = float(gf.std())
    # 亮度直方图
    hist, _ = np.histogram(gray, bins=256, range=(0, 256))
    total = float(max(1, gray.size))
    dark_ratio = float(hist[:24].sum() / total)     # <24 近乎全黑
    white_ratio = float(hist[236:].sum() / total)   # >236 近乎全白

    hh, ss, vv = _rgb_to_hsv(arr)
    sat_mean = float(ss.mean())
    hue_mean = float(hh.mean())
    # 棕褐化：色相集中在 20-50 度且饱和度适中
    sepia_mask = (hh >= 15) & (hh <= 50) & (ss > 0.12)
    sepia_ratio = float(sepia_mask.mean())
    # 彩色度：高饱和像素占比（老照片大多极低）
    color_ratio = float((ss > 0.22).mean())

    # 肤色占比（有人物的近似判据）
    r = arr[:, :, 0].astype(np.int16)
    g = arr[:, :, 1].astype(np.int16)
    b = arr[:, :, 2].astype(np.int16)
    skin = (
        (r > 95) & (g > 40) & (b > 20)
        & ((np.maximum(np.maximum(r, g), b) - np.minimum(np.minimum(r, g), b)) > 15)
        & (np.abs(r - g) > 15) & (r > g) & (r > b)
    )
    skin_ratio = float(skin.mean())

    # 噪声估计：平坦区的局部标准差均值
    noise = _noise_estimate(gray)

    return {
        "width": int(w), "height": int(h), "megapixels": round(mpx, 3),
        "aspect": round(w / float(h), 4) if h else 0.0,
        "blur": round(lap, 2),
        "brightness": round(mean, 4),
        "contrast": round(std, 4),
        "dark_ratio": round(dark_ratio, 4),
        "white_ratio": round(white_ratio, 4),
        "saturation": round(sat_mean, 4),
        "hue": round(hue_mean, 2),
        "sepia": round(sepia_ratio, 4),
        "color": round(color_ratio, 4),
        "skin": round(skin_ratio, 4),
        "noise": round(noise, 2),
        # 按工作尺寸归一化，避免大图 Laplacian 方差天然偏高导致全满分
        "sharpness": round(_sharp_score(lap, gray.shape), 1),
        "lap_var": round(lap, 2),
    }


def _sharp_score(lap: float, shape) -> float:
    """把 Laplacian 方差映射到 0-100，并对工作分辨率做归一化。

    经验标定：640px 宽的自然照片，清晰片 lap≈300-900，糊片 lap<60。
    """
    h, w = shape[0], shape[1]
    scale = 640.0 / max(1.0, float(w))
    norm = max(0.0, lap) * (scale ** 2)
    # 对数映射：norm=25 -> ~30 分，norm=200 -> ~62 分，norm=900 -> ~89 分
    s = 100.0 * (1.0 - math.exp(-norm / 420.0))
    return max(0.0, min(100.0, s))


def _noise_estimate(gray: np.ndarray) -> float:
    """高频噪声估计（Immerkaer 快速估计法）。

    用二维 Laplacian 掩模对全图卷积后取绝对值均值再归一化。
    对胶片颗粒、扫描噪声敏感，且平坦区响应最大、真实边缘会被抵消，
    比"块内标准差"更贴近人眼看到的颗粒感。
    """
    g = gray.astype(np.float32)
    if g.shape[0] < 4 or g.shape[1] < 4:
        return 0.0
    lap = (
        4.0 * g[1:-1, 1:-1]
        - g[:-2, 1:-1] - g[2:, 1:-1] - g[1:-1, :-2] - g[1:-1, 2:]
    )
    sigma = float(np.mean(np.abs(lap))) * math.sqrt(math.pi / 2.0) / 6.0
    return sigma


def adaptive_blur_threshold(m: dict, cfg: dict) -> float:
    """老照片的 Laplacian 方差天然偏低（颗粒被误当高频、扫描件低对比）。

    按图像自身特征下调阈值，避免把年代久远但可读的照片全判成"模糊"。
    """
    base = float(cfg.get("blur_threshold", 90.0))
    t = base
    # 泛黄/低饱和的胶片扫描：对比低，方差统计上更小
    if m.get("contrast", 0.2) < 0.10:
        t *= 0.55
    elif m.get("contrast", 0.2) < 0.135:
        t *= 0.75
    # 颗粒重的片子，噪声会抬高方差，不该因此显得"清晰"，但也不该判糊
    if m.get("noise", 0.0) > 6.0:
        t *= 0.80
    # 实际分辨率很低（说明原片本就小），不按绝对清晰度苛求
    if m.get("megapixels", 1.0) < 0.5:
        t *= 0.85
    return max(12.0, t)


def is_screen_like(im, m: dict = None) -> bool:
    """判断是不是截图/文档/界面图。

    截图和老照片的统计特征完全不同：
      * 大面积纯色（白色文档底、黑色终端底）→ 对比度天然偏低
      * 界面色块饱和度高 → 会被算成"噪点重"
      * 像素锐利但缺少连续色调 → 色阶统计异常
    用老照片的阈值去套截图，绝大多数都会误判成"褪色/噪点/过亮"。
    """
    try:
        import numpy as _np
        from PIL import Image as _Image
        a = _np.asarray(im.convert("RGB").resize((160, 160), _Image.BILINEAR),
                        dtype=_np.uint8)
    except Exception:
        return False
    total = float(a.shape[0] * a.shape[1])
    if total <= 0:
        return False
    flat = ((a > 245).all(axis=2) | (a < 12).all(axis=2)).sum() / total
    mx = a.max(axis=2).astype(_np.int16)
    mn = a.min(axis=2).astype(_np.int16)
    vivid = ((mx - mn) > 120).sum() / total
    if flat > 0.35 or vivid > 0.10:
        return True
    if m:
        if m.get("megapixels", 0) > 3 and m.get("contrast", 1) > 0.30:
            return True
    return False


def classify_flags(m: dict, cfg: dict, screen_like: bool = False) -> list:
    """把指标翻译成人类可读的质量标签。

    screen_like=True 时用一套专门给截图/文档的宽松标准：
    截图没有胶片颗粒、没有泛黄，也不需要"修复"，报这些标签纯属噪音。
    """
    flags = []

    if screen_like:
        # 截图/文档只报真正影响观看的问题，而且阈值宽松得多
        if m["megapixels"] < max(0.05, cfg["lowres_mp"] * 0.3):
            flags.append("低分辨率")
        if m["brightness"] > 0.97:
            flags.append("过亮")
        if m["brightness"] < 0.05:
            flags.append("过暗")
        if m["blur"] < 2.0 and m.get("megapixels", 0) < 0.02:
            flags.append("模糊")
        return flags

    if m["blur"] < adaptive_blur_threshold(m, cfg):
        flags.append("模糊")
    if m["brightness"] < cfg["dark_threshold"]:
        flags.append("过暗")
    if m["brightness"] > cfg["bright_threshold"]:
        flags.append("过亮")
    if m["white_ratio"] > cfg["white_threshold"]:
        flags.append("高光过曝")
    if m["dark_ratio"] > cfg["white_threshold"] * 1.6:
        flags.append("大面积死黑")
    if m["megapixels"] < cfg["lowres_mp"]:
        flags.append("低分辨率")
    if m["contrast"] < cfg["faded_contrast"]:
        flags.append("褪色平淡")
    if m["saturation"] < cfg["faded_saturation"] and m["color"] < 0.05:
        flags.append("灰白/单色")
    if m["sepia"] > cfg["sepia_threshold"] and m["color"] < 0.10:
        flags.append("泛黄/棕褐")
    if m["noise"] > 5.0:
        flags.append("噪点重")
    return flags


def analyze_photo(path: str, cfg: dict, thumb_out: str | None = None,
                  thumb_size=THUMB_MAX, thumb_quality=82) -> dict | None:
    """一次性完成：尺寸、质量指标、三种哈希、缩略图。"""
    im = open_image(path)
    if im is None:
        return None
    try:
        q = quality_metrics(im)
        res = dict(q)
        res["a_hash"] = hexhash(ahash(im))
        res["d_hash"] = hexhash(dhash(im))
        res["p_hash"] = hexhash(phash(im))
        screen_like = is_screen_like(im, q)
        # 实验室功能未开启时不做质量判定，flags 留空（照片库仍能正常显示）
        if cfg.get("lab_quality", False):
            res["flags"] = classify_flags(q, cfg, screen_like=screen_like)
        else:
            res["flags"] = []
        res["screen_like"] = screen_like
        res["has_people"] = q["skin"] > cfg.get("skin_threshold", 0.055)
        res["is_mono"] = q["color"] < 0.06
        res["is_sepia"] = q["sepia"] > cfg.get("sepia_threshold", 0.45) and q["color"] < 0.10
        # 年代感评分：褪色 + 泛黄 + 低饱和 + 颗粒
        old = 0.0
        old += min(1.0, max(0.0, (cfg["faded_contrast"] * 2.2 - q["contrast"]) / 0.12)) * 25
        old += min(1.0, max(0.0, q["sepia"] / 0.75)) * 25
        old += min(1.0, max(0.0, (0.30 - q["saturation"]) / 0.25)) * 20
        old += min(1.0, max(0.0, q["noise"] / 16.0)) * 15
        old += min(1.0, max(0.0, (0.16 - q["color"]) / 0.16)) * 15
        res["age_score"] = round(max(0.0, min(100.0, old)), 1)
        if thumb_out:
            res["thumb_ok"] = save_thumb(im, thumb_out, thumb_size, thumb_quality)
        return res
    except Exception:
        return None
    finally:
        try:
            im.close()
        except Exception:
            pass


def thumb_key(path: str, size: int, mtime: float) -> str:
    raw = "%s|%d|%.3f" % (path.lower(), size, mtime)
    return hashlib.sha1(raw.encode("utf-8", "replace")).hexdigest()[:20]