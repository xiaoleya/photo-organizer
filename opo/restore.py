# -*- coding: utf-8 -*-
"""老照片修复与增强算法（纯 numpy + Pillow 实现）。

覆盖：
  auto_levels   自动色阶（直方图拉伸，带百分位裁剪）
  gray_world    灰世界白平衡 / 去偏色
  deyellow      去泛黄（棕褐化校正）
  restore_color 褪色复原（饱和度与对比双提升）
  denoise       胶片颗粒/扫描噪点抑制
  despeckle     划痕、灰尘、霉斑去除（顶帽检测 + 中值修补）
  clahe         局部对比度增强（分块直方图均衡，带裁剪限幅）
  unsharp       反锐化掩模锐化
  crop_border   自动裁掉扫描白边/黑边
  deskew        自动纠偏（投影方差最大化）
  fix_red_eye   红眼抑制
  upscale       高质量放大
  auto_enhance  一键全流程
"""
from __future__ import annotations

import math

import numpy as np
from PIL import Image, ImageFilter, ImageOps

# ------------------------------------------------------------------ 基础工具

def _f(arr):
    return arr.astype(np.float32)


def _u(arr):
    return np.clip(arr, 0, 255).astype(np.uint8)


def _gray(arr):
    return (0.299 * arr[:, :, 0] + 0.587 * arr[:, :, 1] + 0.114 * arr[:, :, 2])


def _cumhist(x, bins=65536):
    h, _ = np.histogram(x, bins=bins, range=(0.0, 1.0))
    c = np.cumsum(h).astype(np.float64)
    return c / max(1.0, c[-1])


# ------------------------------------------------------------------ 自动色阶

def auto_levels(im: Image.Image, clip=0.4, per_channel=True) -> Image.Image:
    """按百分位裁剪拉伸对比度，clip 为每侧裁剪百分比。"""
    a = _f(np.asarray(im))
    if a.ndim == 2:
        a = np.stack([a] * 3, axis=-1)
    lo_p, hi_p = clip, 100.0 - clip
    if per_channel:
        lo = np.percentile(a.reshape(-1, 3), lo_p, axis=0)
        hi = np.percentile(a.reshape(-1, 3), hi_p, axis=0)
    else:
        g = _gray(a)
        lo_v = np.percentile(g, lo_p)
        hi_v = np.percentile(g, hi_p)
        lo = np.array([lo_v] * 3)
        hi = np.array([hi_v] * 3)
    span = np.maximum(hi - lo, 1e-3)
    out = (a - lo) / span * 255.0
    return Image.fromarray(_u(out))


def stretch_gray(im: Image.Image, clip=0.5) -> Image.Image:
    return auto_levels(im, clip=clip, per_channel=False)


# ------------------------------------------------------------------ 白平衡

def gray_world(im: Image.Image, strength=0.85) -> Image.Image:
    """灰世界假设：把三通道均值拉到同一水平，去掉整体偏色。"""
    a = _f(np.asarray(im))
    means = a.reshape(-1, 3).mean(axis=0)
    means = np.maximum(means, 1e-3)
    target = means.mean()
    gain = target / means
    gain = 1.0 + (gain - 1.0) * float(strength)
    return Image.fromarray(_u(a * gain))


def white_patch(im: Image.Image, pct=99.5, strength=0.8) -> Image.Image:
    """白块假设：把最亮的 0.5% 视为白。"""
    a = _f(np.asarray(im))
    hi = np.percentile(a.reshape(-1, 3), pct, axis=0)
    hi = np.maximum(hi, 1e-3)
    gain = 255.0 / hi
    gain = 1.0 + (gain - 1.0) * float(strength)
    return Image.fromarray(_u(a * gain))


def neutral_cast(im: Image.Image) -> Image.Image:
    """估计并中和色偏（同时做灰世界与白块，取折中）。"""
    a = _f(np.asarray(im))
    means = np.maximum(a.reshape(-1, 3).mean(axis=0), 1e-3)
    hi = np.maximum(np.percentile(a.reshape(-1, 3), 99.0, axis=0), 1e-3)
    g1 = means.mean() / means
    g2 = 255.0 / hi
    g = (g1 * 0.6 + g2 * 0.4)
    g = np.clip(g, 0.6, 1.8)
    return Image.fromarray(_u(a * g))


def deyellow(im: Image.Image, strength=1.0) -> Image.Image:
    """专门压制泛黄：蓝通道按黄度补偿，同时轻微压红。"""
    a = _f(np.asarray(im))
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    # 黄度：R、G 高而 B 低的程度
    yellowness = np.clip(((r + g) * 0.5 - b) / 255.0, 0, 1)
    k = float(np.clip(yellowness.mean() * 2.0, 0, 1)) * float(strength)
    if k <= 0.01:
        return im
    a[:, :, 2] = np.clip(b + k * 42.0 * (yellowness * 0.7 + 0.3), 0, 255)
    a[:, :, 0] = np.clip(r - k * 12.0 * (yellowness * 0.5 + 0.5), 0, 255)
    return Image.fromarray(_u(a))


def restore_color(im: Image.Image, sat=1.35, contrast=1.12) -> Image.Image:
    """褪色复原：提升饱和度与对比度，并把色相往中性拉少量。"""
    from PIL import ImageEnhance
    out = im
    if abs(contrast - 1.0) > 0.01:
        out = ImageEnhance.Contrast(out).enhance(contrast)
    if abs(sat - 1.0) > 0.01:
        out = ImageEnhance.Color(out).enhance(sat)
    return out


def saturate_by_fade(im: Image.Image, target=0.20) -> Image.Image:
    """测出饱和度后补到目标值附近（老照片褪色越重补得越多）。"""
    a = _f(np.asarray(im))
    mx = a.max(axis=2)
    mn = a.min(axis=2)
    s = np.where(mx > 1e-3, (mx - mn) / np.maximum(mx, 1e-3), 0.0)
    cur = float(s.mean())
    if cur >= target or cur <= 1e-4:
        return im
    factor = min(1.9, max(1.0, target / cur))
    from PIL import ImageEnhance
    return ImageEnhance.Color(im).enhance(factor)


# ------------------------------------------------------------------ 去噪

def denoise(im: Image.Image, strength=1) -> Image.Image:
    """中值 + 高斯混合去噪，保留边缘。"""
    out = im
    if strength >= 1:
        out = out.filter(ImageFilter.MedianFilter(3))
    if strength >= 2:
        out = out.filter(ImageFilter.MedianFilter(3))
    if strength >= 3:
        out = out.filter(ImageFilter.GaussianBlur(0.6))
    return out


def denoise_adaptive(im: Image.Image, noise_level: float) -> Image.Image:
    """按噪点强度选择去噪档位。

    noise_level 为 Immerkaer 噪声估计（0-255 灰度标度）：
    干净数码片 ~1-2，普通胶片扫描 ~2-4，重颗粒/劣质扫描 >5。
    """
    n = float(noise_level or 0.0)
    if n < 1.8:
        return im                      # 已经干净，不动
    if n < 3.2:
        return im.filter(ImageFilter.MedianFilter(3))
    if n < 5.0:
        out = im.filter(ImageFilter.MedianFilter(3))
        return out.filter(ImageFilter.MedianFilter(3))
    out = im.filter(ImageFilter.MedianFilter(5))
    return out.filter(ImageFilter.GaussianBlur(0.5))


# ------------------------------------------------------------------ 划痕 / 灰尘

def _min_filter_1d(a, k, axis):
    """一维最小值滤波（腐蚀），边界 edge 复制。

    注意切片长度必须是"原始"尺寸 a.shape[axis]，
    不能用 pad 之后尺寸（否则每片都退化成整幅图，滤波完全失效）。
    """
    p = k // 2
    n = a.shape[axis]
    if axis == 1:
        pad = np.pad(a, ((0, 0), (p, p)), mode="edge")
        return np.min(np.stack([pad[:, i:i + n] for i in range(k)]), axis=0)
    pad = np.pad(a, ((p, p), (0, 0)), mode="edge")
    return np.min(np.stack([pad[i:i + n] for i in range(k)]), axis=0)


def _max_filter_1d(a, k, axis):
    """一维最大值滤波（膨胀），边界 edge 复制。"""
    p = k // 2
    n = a.shape[axis]
    if axis == 1:
        pad = np.pad(a, ((0, 0), (p, p)), mode="edge")
        return np.max(np.stack([pad[:, i:i + n] for i in range(k)]), axis=0)
    pad = np.pad(a, ((p, p), (0, 0)), mode="edge")
    return np.max(np.stack([pad[i:i + n] for i in range(k)]), axis=0)


def _morph_1d(a, k, op, axis):
    """沿单一方向做 1xK / Kx1 的结构元运算。op='open' 或 'close'。"""
    if op == "open":
        return _max_filter_1d(_min_filter_1d(a, k, axis), k, axis)
    return _min_filter_1d(_max_filter_1d(a, k, axis), k, axis)


def _median1d(a, k, axis):
    """沿单一方向的一维中值滤波（边界 edge 复制）。

    中值滤波的关键性质：只要目标宽度 < k/2，就会被邻域中值"吃掉"，
    因此它能剥离任意长度的细线（开运算只能去掉比结构元短的斑点）。
    """
    p = k // 2
    n = a.shape[axis]
    if axis == 1:
        pad = np.pad(a, ((0, 0), (p, p)), mode="edge")
        st = np.stack([pad[:, i:i + n] for i in range(k)], axis=0)
    else:
        pad = np.pad(a, ((p, p), (0, 0)), mode="edge")
        st = np.stack([pad[i:i + n] for i in range(k)], axis=0)
    return np.median(st, axis=0)


def thin_line_response(g, k=5):
    """提取"细亮线"与"细暗线"的响应图。

    原理（这是本项目里最容易做错的一步，实测踩了三次坑）：
      * 开运算去不掉长直线，只对小于结构元的斑点有效；
      * 闭运算对细亮线同样无效；
      * 一维中值滤波才能稳定剥离"细长"结构：
        沿水平方向取中值，宽度 1-2 像素的竖线会被抹掉，
        原图减去中值图，就得到这条竖线的亮度响应。

    对外层含义：
      亮线响应 = g - median(水平)  与  g - median(垂直) 取大者
      暗线响应 = 同一逻辑作用在 -g 上
    大面积区域（天空、衣服、人脸）响应恒为 0，不会误伤。
    """
    n = g.shape[0], g.shape[1]
    # 亮线
    m_h = _median1d(g, k, 1)      # 水平中值：抹掉竖线
    m_v = _median1d(g, k, 0)      # 垂直中值：抹掉横线
    bright = np.maximum(g - m_h, g - m_v)
    # 暗线
    ng = -g
    n_h = _median1d(ng, k, 1)
    n_v = _median1d(ng, k, 0)
    dark = np.maximum(ng - n_h, ng - n_v)
    return bright, dark


def _dilate(mask, r=1):
    """二维膨胀（方形结构元），用于给掩膜留一点边缘余量。"""
    if r < 1:
        return mask
    k = 2 * r + 1
    pad = np.pad(mask, r, mode="edge")
    acc = np.zeros_like(mask)
    for dy in range(k):
        for dx in range(k):
            acc = np.maximum(acc, pad[dy:dy + mask.shape[0], dx:dx + mask.shape[1]])
    return acc


def despeckle(im: Image.Image, threshold=12.0, dilate=1, max_ratio=0.10) -> Image.Image:
    """去除划痕、灰尘、霉点、折痕。

    只处理"细长且与周围反差明显"的像素：
      1) 一维中值滤波求背景，原图减背景得到细线响应；
      2) 响应超过 threshold（灰度差）才算候选；
      3) 用 5x5 邻域占比过滤掉物体边缘（大片区域占比高）；
      4) 命中面积超过 max_ratio 说明判据失效，直接放弃（宁可不修也不糊图）；
      5) 按响应强度做加权混合修补，避免硬边。
    """
    a = np.asarray(im.convert("RGB"), dtype=np.float32)
    g = _gray(a)
    h, w = g.shape
    if h < 16 or w < 16:
        return im

    th, bh = thin_line_response(g, 5)
    resp = np.maximum(th, bh)
    suspect = resp > threshold
    if not suspect.any():
        return im

    # 过滤物体边缘：5x5 邻域内命中占比高的区域是大面积边缘，不是划痕
    sf = suspect.astype(np.float32)
    pad = np.pad(sf, 2, mode="edge")
    local = np.zeros_like(sf)
    for dy in range(5):
        for dx in range(5):
            local += pad[dy:dy + sf.shape[0], dx:dx + sf.shape[1]]
    local /= 25.0
    suspect = suspect & (local < 0.55)

    n_hit = int(suspect.sum())
    if n_hit == 0:
        return im
    if n_hit > h * w * float(max_ratio):
        return im       # 命中过多，判据不可靠

    mask = _dilate(suspect.astype(np.float32), max(1, int(dilate)))
    # 强度渐入：响应越强，修补越彻底
    strength = np.clip((resp - threshold) / max(threshold * 2.0, 1.0), 0.0, 1.0)
    strength = _dilate(strength, max(1, int(dilate)))
    wgt = np.clip(mask * np.clip(0.45 + strength * 1.2, 0.0, 1.0), 0.0, 1.0)[:, :, None]

    ref = np.asarray(im.filter(ImageFilter.MedianFilter(3)), dtype=np.float32)
    out = a * (1.0 - wgt) + ref * wgt
    return Image.fromarray(_u(out))


# ------------------------------------------------------------------ CLAHE

def clahe(im: Image.Image, clip_limit=2.0, tiles=8) -> Image.Image:
    """分块自适应直方图均衡（在 Y 通道上做，避免色偏）。

    与常规实现一致：每个 tile 的 CDF 作为查找表，
    然后按像素在 tile 网格中的相对位置做双线性插值，
    这样既保留局部细节又不会出现块状边界。
    """
    ycbcr = np.array(im.convert("YCbCr"), dtype=np.uint8, copy=True)
    y = ycbcr[:, :, 0].astype(np.float32) / 255.0
    h, w = y.shape
    if h < tiles or w < tiles:
        return im

    # 每个 tile 的查找表
    xs = np.linspace(0, w, tiles + 1).astype(np.int32)
    ys = np.linspace(0, h, tiles + 1).astype(np.int32)
    lut = np.zeros((tiles, tiles, 256), dtype=np.float32)
    for i in range(tiles):
        for j in range(tiles):
            tile = y[ys[i]:ys[i + 1], xs[j]:xs[j + 1]]
            if tile.size == 0:
                lut[i, j] = np.linspace(0, 1, 256)
                continue
            hist, _ = np.histogram(tile, bins=256, range=(0, 1))
            lim = max(1, int(clip_limit * tile.size / 256.0))
            excess = int(np.maximum(0, hist - lim).sum())
            hist = np.minimum(hist, lim)
            hist = hist + int(excess / 256.0)
            cdf = np.cumsum(hist).astype(np.float32)
            lut[i, j] = cdf / max(1.0, cdf[-1])

    # 像素所属 tile 的浮点坐标
    idx = np.clip((y * 255).astype(np.int32), 0, 255)
    cx = (np.arange(w, dtype=np.float32) + 0.5) * tiles / w - 0.5
    cy = (np.arange(h, dtype=np.float32) + 0.5) * tiles / h - 0.5
    cx = np.clip(cx, 0, tiles - 1)
    cy = np.clip(cy, 0, tiles - 1)
    x0 = np.floor(cx).astype(np.int32); x1 = np.minimum(x0 + 1, tiles - 1)
    y0 = np.floor(cy).astype(np.int32); y1 = np.minimum(y0 + 1, tiles - 1)
    fx = (cx - x0).astype(np.float32)[None, :]
    fy = (cy - y0).astype(np.float32)[:, None]

    l00 = lut[y0[:, None], x0[None, :], idx]
    l01 = lut[y0[:, None], x1[None, :], idx]
    l10 = lut[y1[:, None], x0[None, :], idx]
    l11 = lut[y1[:, None], x1[None, :], idx]
    top = l00 * (1 - fx) + l01 * fx
    bot = l10 * (1 - fx) + l11 * fx
    out = top * (1 - fy) + bot * fy
    ycbcr[:, :, 0] = _u(out * 255.0)
    return Image.fromarray(ycbcr, "YCbCr").convert("RGB")


def _box_blur(a, r):
    if r < 1:
        return a
    k = 2 * r + 1
    pad = np.pad(a, ((r, r), (r, r)), mode="edge")
    c = np.cumsum(np.cumsum(pad, axis=0), axis=1)
    c = np.pad(c, ((1, 0), (1, 0)), mode="constant")
    h, w = a.shape
    s = (c[k:k + h, k:k + w] - c[0:h, k:k + w] - c[k:k + h, 0:w] + c[0:h, 0:w])
    return s / float(k * k)


# ------------------------------------------------------------------ 锐化

def unsharp(im: Image.Image, radius=1.6, percent=140, threshold=3) -> Image.Image:
    return im.filter(ImageFilter.UnsharpMask(radius=radius, percent=int(percent),
                                              threshold=int(threshold)))



def edge_sharpen(im: Image.Image, amount=1.0, radius=1.6, threshold=8) -> Image.Image:
    """边缘感知锐化：只在梯度明显的区域锐化，平坦区（噪点区）原样保留。

    避免老照片去噪后又把颗粒放大出来的常见问题。
    """
    a = np.asarray(im.convert("RGB"), dtype=np.float32)
    g = np.asarray(im.convert("L"), dtype=np.float32)
    # Sobel 梯度
    gx = np.zeros_like(g); gy = np.zeros_like(g)
    gx[:, 1:-1] = g[:, 2:] - g[:, :-2]
    gy[1:-1, :] = g[2:, :] - g[:-2, :]
    mag = np.sqrt(gx ** 2 + gy ** 2)
    mask = np.clip((mag - threshold) / 40.0, 0.0, 1.0)
    # 形态学膨胀一圈，让边缘带一点宽度，避免锐化后出现光晕断点
    m = mask
    pad = np.pad(m, 1, mode="edge")
    m2 = m.copy()
    for dy in (0, 1, 2):
        for dx in (0, 1, 2):
            m2 = np.maximum(m2, pad[dy:dy + m.shape[0], dx:dx + m.shape[1]])
    mask = np.clip(m2, 0, 1)[:, :, None]

    sharp = np.asarray(im.filter(ImageFilter.UnsharpMask(
        radius=radius, percent=int(100 * amount), threshold=2)), dtype=np.float32)
    out = a * (1 - mask) + sharp * mask
    return Image.fromarray(_u(out))


def suppress_noise(im: Image.Image, level: float) -> Image.Image:
    """按噪点强度做一次温和抑制（放在对比度增强之后，抵消放大）。"""
    if level <= 0:
        return im
    if level < 3.5:
        return im
    if level < 6.0:
        return im.filter(ImageFilter.MedianFilter(3))
    out = im.filter(ImageFilter.MedianFilter(3))
    return out.filter(ImageFilter.GaussianBlur(0.4))


def smart_sharpen(im: Image.Image, blur_metric: float) -> Image.Image:
    """越糊的图锐化越强，但设上限防止出现白边噪点。"""
    if blur_metric is None:
        return unsharp(im, 1.4, 110, 4)
    if blur_metric < 40:
        return unsharp(im, 2.2, 200, 2)
    if blur_metric < 90:
        return unsharp(im, 1.9, 165, 3)
    if blur_metric < 200:
        return unsharp(im, 1.5, 120, 4)
    return unsharp(im, 1.1, 80, 5)


# ------------------------------------------------------------------ 边框裁切

def crop_border(im: Image.Image, tol=10, max_frac=0.22, mode="auto") -> Image.Image:
    """裁掉扫描产生的大面积纯色边框（白边/黑边）。"""
    a = np.asarray(im.convert("RGB"), dtype=np.int16)
    h, w = a.shape[:2]
    if h < 20 or w < 20:
        return im
    bg = np.median(np.concatenate([a[0, :], a[-1, :], a[:, 0], a[:, -1]]), axis=0)
    diff = np.abs(a - bg).max(axis=2)
    content = diff > tol
    rows = np.where(content.sum(axis=1) > w * 0.02)[0]
    cols = np.where(content.sum(axis=0) > h * 0.02)[0]
    if len(rows) < 5 or len(cols) < 5:
        return im
    t, b = int(rows[0]), int(rows[-1]) + 1
    l, r = int(cols[0]), int(cols[-1]) + 1
    # 保险：裁掉比例不能过大
    if (b - t) < h * (1 - max_frac) * 1.6 and (b - t) > h * 0.3:
        pass
    if t > h * max_frac or l > w * max_frac or (h - b) > h * max_frac or (w - r) > w * max_frac:
        # 边框比例过大，说明这张图本来就没有扫描边（或整张都是内容），原样返回。
        # 绝不为了"保险"去裁掉一圈——那会凭空降低分辨率并引入插值损失。
        return im
    if t == 0 and l == 0 and b == h and r == w:
        return im
    return im.crop((l, t, r, b))


# ------------------------------------------------------------------ 纠偏

def detect_skew(im: Image.Image, max_angle=12.0, step=0.5, work=420):
    """用投影方差法估计倾斜角（度）。"""
    g = im.convert("L")
    if max(g.size) > work:
        g2 = g.copy()
        g2.thumbnail((work, work), Image.BILINEAR)
    else:
        g2 = g
    a = np.asarray(g2, dtype=np.float32)
    best, best_score = 0.0, -1.0
    ang = -max_angle
    while ang <= max_angle + 1e-6:
        r = Image.fromarray(a.astype(np.uint8)).rotate(ang, resample=Image.BILINEAR,
                                                       fillcolor=int(a.mean()))
        ra = np.asarray(r, dtype=np.float32)
        prof = ra.sum(axis=1)
        prof = prof - prof.mean()
        score = float((prof ** 2).mean())
        if score > best_score:
            best_score, best = score, ang
        ang += step
    return best


def _enhance_screen(im: Image.Image, o: dict) -> Image.Image:
    """截图/文档/界面图的增强：只做"提对比 + 去偏色 + 轻微锐化"。

    刻意不做的事（都会让截图变糊或变形）：
      * 中值滤波/降噪 —— 截图没有胶片颗粒，只会糊掉文字边缘
      * 自动裁边     —— 会把窗口边框当扫描白边裁掉
      * 自动纠偏     —— 会把表格/文字块判成倾斜转歪
      * CLAHE 强拉   —— 会让界面底色出现块状色块

    只做：
      1) 可选白平衡：把界面里偏色的底色拉回中性
      2) 温和色阶：只拉掉两端 0.05% 的极端值，尽量不改变字形灰度
      3) 轻度锐化：很小的半径，仅让文字边缘更实
    """
    out = im

    if o.get("whitebalance"):
        # 截图的底色往往是大面积纯色，用它做白块校正最稳
        out = white_patch(out, pct=99.0, strength=0.55)

    if o.get("levels"):
        # 只裁掉极少量的极端像素，保住文字的灰度层次
        out = auto_levels(out, clip=0.05, per_channel=False)

    if o.get("sharpen"):
        # 半径小、强度低：只让边缘变实，不产生白边
        out = unsharp(out, radius=0.8, percent=55, threshold=3)

    return out


def _is_screen_or_document(im: Image.Image, m: dict = None) -> bool:
    """判断是不是截图/文档/界面图：这类图不该做纠偏。

    纠偏靠"投影方差最大化"，一旦图里是窗口、表格、文字块，
    很容易被误判成倾斜，转出来就是斜的（真实踩过的坑）。
    判据用纯色像素占比 + 高饱和界面色，两者都在截图里特别明显。
    """
    try:
        a = np.asarray(im.convert("RGB").resize((200, 200), Image.BILINEAR),
                       dtype=np.uint8)
    except Exception:
        return False
    total = float(a.shape[0] * a.shape[1])
    if total <= 0:
        return False

    mx = a.max(axis=2).astype(np.int16)
    mn = a.min(axis=2).astype(np.int16)
    spread = mx - mn

    # 1) 大面积近白或近黑：文档、终端、深色主题界面
    flat = ((a > 245).all(axis=2) | (a < 12).all(axis=2)).sum() / total
    # 2) 深色界面占比：IDE / 终端截图常见大片深色
    dark = (mx < 60).sum() / total
    # 3) 高饱和界面色块与彩色文字
    vivid = (spread > 110).sum() / total

    if flat > 0.30 or dark > 0.32 or vivid > 0.08:
        return True

    # 4) 颜色种类极少：截图往往只有少数几种量化色
    try:
        q = (a // 32).astype(np.int32)
        uniq = len(np.unique(q[:, :, 0] * 64 + q[:, :, 1] * 8 + q[:, :, 2]))
    except Exception:
        uniq = 9999
    if uniq < 260 and (flat + dark) > 0.18:
        return True

    if m:
        if m.get("megapixels", 0) > 3 and m.get("contrast", 1) > 0.30:
            return True
        # 文字边缘会被当成噪点，这类"高噪点+有对比"基本是截图
        if m.get("noise", 0) > 6.0 and m.get("contrast", 0) > 0.12:
            return True
    return False


def deskew(im: Image.Image, angle=None, max_angle=12.0, min_angle=0.9,
            min_gain=1.06) -> Image.Image:
    """自动纠偏（仅用于照片，截图/文档自动跳过）。

    三道保险，避免对本来水平的照片做无意义的旋转（旋转会引入插值噪声）：
      0) 截图/文档/界面图直接跳过（否则会把代码截图转斜）；
      1) 角度小于 min_angle 度不转；
      2) 旋转带来的投影方差增益不足 min_gain 倍也不转
         （说明图上没有明确的直线结构，估计值不可信）。
    """
    if _is_screen_or_document(im):
        return im

    g = im.convert("L")
    work = 420
    small = g.copy()
    if max(g.size) > work:
        small.thumbnail((work, work), Image.BILINEAR)

    if angle is None:
        base_score = _proj_score(np.asarray(small, dtype=np.float32))
        angle, best_score = 0.0, base_score
        a = -max_angle
        while a <= max_angle + 1e-6:
            if abs(a) >= min_angle:
                r = small.rotate(a, resample=Image.BILINEAR, fillcolor=int(128))
                sc = _proj_score(np.asarray(r, dtype=np.float32))
                if sc > best_score:
                    best_score, angle = sc, a
            a += 0.5
        if abs(angle) < min_angle:
            return im
        if base_score > 0 and best_score / base_score < min_gain:
            return im
    if abs(angle) < min_angle:
        return im
    return im.rotate(angle, resample=Image.BICUBIC, expand=True, fillcolor=(255, 255, 255))


def _proj_score(arr):
    """投影方差得分：图像摆正时，行/列投影的起伏最剧烈。"""
    prof = arr.sum(axis=1)
    prof = prof - prof.mean()
    return float((prof ** 2).mean())


# ------------------------------------------------------------------ 红眼

def fix_red_eye(im: Image.Image, threshold=1.55) -> Image.Image:
    a = _f(np.asarray(im))
    r, g, b = a[:, :, 0], a[:, :, 1], a[:, :, 2]
    mask = (r > 70) & (r > threshold * np.maximum(1.0, (g + b) * 0.5))
    if not mask.any():
        return im
    m = mask.astype(np.float32)
    m = np.asarray(Image.fromarray((m * 255).astype(np.uint8)).filter(
        ImageFilter.MinFilter(3)), dtype=np.float32) / 255.0
    a[:, :, 0] = r * (1 - m) + ((g + b) * 0.5) * m
    return Image.fromarray(_u(a))


# ------------------------------------------------------------------ 放大

def upscale(im: Image.Image, factor=2.0, max_side=8000) -> Image.Image:
    w, h = im.size
    nw, nh = int(w * factor), int(h * factor)
    if max(nw, nh) > max_side:
        s = max_side / float(max(nw, nh))
        nw, nh = int(nw * s), int(nh * s)
    if nw <= w:
        return im
    up = im.resize((nw, nh), Image.LANCZOS)
    return unsharp(up, radius=1.2, percent=70, threshold=2)


# ------------------------------------------------------------------ 全流程

def _denoise_to_budget(im: Image.Image, budget: float, max_iter=6,
                       keep_contrast=None) -> Image.Image:
    """按需去噪：只用刚好够的滤波强度把噪声压到预算内。

    要点：
      1) 每轮先试 3x3 中值，小步推进，避免一次性糊成一片；
      2) 若某轮会让对比度掉到 keep_contrast 以下，就停止（宁可有少量噪点）；
      3) 用混合而非替换，保住边缘。
    """
    from .imaging import _noise_estimate

    def stats(img):
        g = img.convert("L")
        if max(g.size) > 640:
            g2 = g.copy()
            g2.thumbnail((640, 640), Image.LANCZOS)
            g = g2
        arr = np.asarray(g, dtype=np.uint8)
        return _noise_estimate(arr), float(arr.std()) / 255.0

    cur = im
    now, con = stats(cur)
    if now <= budget:
        return cur
    lo = keep_contrast if keep_contrast is not None else con * 0.80

    for it in range(max_iter):
        if now <= budget:
            break
        over = (now - budget) / max(budget, 0.8)
        size = 3 if (it < 2 or over < 1.2) else 5
        ref = cur.filter(ImageFilter.MedianFilter(size))
        k = min(0.80, 0.30 + over * 0.12)
        a = np.asarray(cur.convert("RGB"), dtype=np.float32)
        b = np.asarray(ref.convert("RGB"), dtype=np.float32)
        trial = Image.fromarray(_u(a * (1 - k) + b * k))
        t_noise, t_con = stats(trial)
        # 噪声远超预算（>1.5 倍）时优先压噪；否则守住对比度
        if t_noise > budget * 1.5:
            cur, now, con = trial, t_noise, t_con
            continue
        if t_con < lo and t_noise > budget:
            break       # 再糊就损失细节了，收手
        cur, now, con = trial, t_noise, t_con
    return cur


def auto_enhance(im: Image.Image, metrics=None, opts=None) -> Image.Image:
    """一键老照片增强。

    顺序经实测标定。三条经验来自真实测量：
      1) 对比度拉伸（auto_levels）会把胶片颗粒放大 2-3 倍，
         所以"去噪"必须放在拉伸之后，而不是之前；
      2) CLAHE 会显著抬高高频噪声（实测 noise +50%），
         因此只在"确实平淡"的图上启用，且强度压低、放在去噪之后；
      3) 划痕修补必须只作用于细长结构，否则会把整张图的细节抹平。

    流程：裁边 -> 纠偏 -> 白平衡 -> 去泛黄 -> 自动色阶 -> 拉伸后去噪
          -> 去划痕 -> 可选局部对比(CLAHE) -> 再校噪 -> 褪色补色
          -> 边缘感知锐化 -> 终检闭环
    """
    from .imaging import _noise_estimate

    o = {
        "crop": True, "deskew": True, "denoise": True, "despeckle": True,
        "whitebalance": True, "deyellow": True, "levels": True,
        "clahe": "auto", "fade": True, "sharpen": True, "upscale": False,
    }
    if opts:
        o.update(opts)
    m = metrics or {}
    noise0 = float(m.get("noise", 0) or 0)

    def _gray_of(img, work=640):
        # 与 imaging.quality_metrics 一致：缩到 640 宽再统计，
        # 避免"原图分辨率"与"工作分辨率"混用导致噪声读数不一致
        g = img.convert("L")
        if max(g.size) > work:
            g2 = g.copy()
            g2.thumbnail((work, work), Image.LANCZOS)
            g = g2
        return np.asarray(g, dtype=np.uint8)

    def _std_of(img):
        return float(_gray_of(img).std()) / 255.0

    out = im

    # 截图/文档类图片走"保清晰"专用流程。
    # 之前的问题：截图被判成"噪点重"，走了中值滤波 + 降噪 + 强锐化，
    # 文字边缘被糊掉，结果比原图还模糊。截图本身没有胶片噪点，
    # 需要的只是把对比度拉开、别动像素结构。
    screen_like = _is_screen_or_document(im, m)
    if screen_like:
        return _enhance_screen(im, o)

    if o["crop"]:
        out = crop_border(out)
    if o["deskew"]:
        out = deskew(out)
    if o["whitebalance"]:
        out = neutral_cast(out)
    if o["deyellow"]:
        out = deyellow(out, 1.0 if float(m.get("sepia", 0) or 0) > 0.40 else 0.5)
    if o["levels"]:
        out = auto_levels(out, clip=0.35)

    budget = max(3.2, noise0 * 1.35 + 1.2)
    if o["denoise"]:
        out = _denoise_to_budget(out, budget, keep_contrast=None)

    if o["despeckle"]:
        # 阈值按实测标定：响应为灰度差，划痕通常 12-60，噪点 <10
        thr = min(30.0, 11.0 + noise0 * 2.2)
        out = despeckle(out, threshold=thr)

    # ---- 局部对比增强：只对平淡图启用，且强度随噪点下降
    want_clahe = o["clahe"]
    if want_clahe == "auto":
        c0 = _std_of(out)
        want_clahe = bool(c0 < 0.150)     # 只看确实"发灰"的图
    if want_clahe:
        n_before = _noise_estimate(_gray_of(out))
        clip = 1.5
        if n_before > 4.0:
            clip *= 0.70
        elif n_before > 2.5:
            clip *= 0.85
        cand = clahe(out, clip_limit=clip)
        # 只有"噪声没有明显变差 且 对比度确实变好"才采纳
        n_after = _noise_estimate(_gray_of(cand))
        # 双重上限：相对当前不超过 25%，相对原图不超过 3 倍，绝对不超过 3.6
        cap = min(max(n_before * 1.25, n_before + 0.8),
                  max(noise0 * 3.0, 2.6), 3.6)
        if _std_of(cand) > _std_of(out) and n_after <= cap:
            out = cand
        if o["denoise"]:
            out = _denoise_to_budget(out, budget)

    if o["fade"]:
        if float(m.get("saturation", 0.30) or 0.30) < 0.18:
            out = saturate_by_fade(out, 0.20)

    # ---- 锐化：强度与原始噪点负相关
    if o["sharpen"]:
        blur = m.get("blur")
        amount, radius = 1.10, 1.6
        if blur is not None:
            if blur < 40:
                amount, radius = 1.80, 2.1
            elif blur < 90:
                amount, radius = 1.50, 1.9
            elif blur < 200:
                amount, radius = 1.20, 1.6
            else:
                amount, radius = 0.90, 1.3
        n_now = _noise_estimate(_gray_of(out))
        if n_now > 6.0:
            amount *= 0.50
        elif n_now > 4.0:
            amount *= 0.70
        elif n_now > 2.5:
            amount *= 0.88
        out = edge_sharpen(out, amount=amount, radius=radius, threshold=8)

    # ---- 终检：锐化会明显抬高噪声，这里统一压回"可接受上限"
    if o["denoise"]:
        n_end = _noise_estimate(_gray_of(out))
        # 上限取三者较小值：原图的 1.5 倍、当前值的 0.85、绝对 6.5
        cap = min(noise0 * 1.5 + 1.2, n_end * 0.85, 6.5)
        cap = max(cap, 3.2)
        if n_end > cap:
            out = _denoise_to_budget(out, cap, max_iter=4)

    if o["upscale"]:
        out = upscale(out, 2.0)
    return out


PRESETS = {
    "轻度修复": {"despeckle": False, "clahe": False, "sharpen": True, "upscale": False},
    "标准修复": {},
    "重度修复": {"upscale": False, "clahe": True},
    "仅去噪": {"crop": False, "deskew": False, "whitebalance": False, "deyellow": False,
            "levels": False, "clahe": False, "fade": False, "sharpen": False,
            "despeckle": False},
    "仅调色": {"crop": False, "deskew": False, "denoise": False, "despeckle": False,
            "sharpen": False},
    "仅裁边纠偏": {"denoise": False, "despeckle": False, "whitebalance": False,
              "deyellow": False, "levels": False, "clahe": False, "fade": False,
              "sharpen": False},
    "去泛黄": {"crop": False, "deskew": False, "denoise": False, "despeckle": False,
            "levels": False, "clahe": False, "fade": False, "sharpen": False},
    "去划痕": {"crop": False, "deskew": False, "whitebalance": False, "deyellow": False,
            "levels": False, "clahe": False, "fade": False, "sharpen": False},
    "增强清晰度": {"crop": False, "deskew": False, "whitebalance": False,
              "deyellow": False, "levels": False, "clahe": True, "fade": False},
}

# 前端展示用：每个开关的中文名与说明
OPTIONS = [
    {"key": "crop", "label": "自动裁边", "desc": "裁掉扫描留下的白边/黑边"},
    {"key": "deskew", "label": "自动纠偏", "desc": "把明显歪斜的照片摆正"},
    {"key": "whitebalance", "label": "白平衡", "desc": "中和整体偏色"},
    {"key": "deyellow", "label": "去泛黄", "desc": "压掉老照片的黄色调"},
    {"key": "levels", "label": "自动色阶", "desc": "拉开明暗对比"},
    {"key": "denoise", "label": "降噪", "desc": "抑制胶片颗粒与扫描噪点"},
    {"key": "despeckle", "label": "去划痕", "desc": "修补细长划痕、灰尘、霉点"},
    {"key": "clahe", "label": "局部对比", "desc": "提升发灰照片的细节层次"},
    {"key": "fade", "label": "褪色补色", "desc": "把发白的颜色补回来"},
    {"key": "sharpen", "label": "锐化", "desc": "让边缘更清楚"},
    {"key": "upscale", "label": "放大 2 倍", "desc": "小图放大（会变慢）"},
]


def preset_options(name: str) -> dict:
    """把预设展开成完整开关表（前端用于显示勾选状态）。"""
    base = {
        "crop": True, "deskew": True, "denoise": True, "despeckle": True,
        "whitebalance": True, "deyellow": True, "levels": True,
        "clahe": "auto", "fade": True, "sharpen": True, "upscale": False,
    }
    base.update(PRESETS.get(name, {}))
    return base