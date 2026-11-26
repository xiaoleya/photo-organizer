//! 老照片修复与增强。
//!
//! 与 Python 版 `opo/restore.py` 对齐。全部以 f32 平面缓冲实现。
//!
//! `auto_enhance` 的处理顺序是实测标定的，不能随意调换：
//!   1) 对比度拉伸会把胶片颗粒放大 2-3 倍，所以去噪必须在拉伸**之后**；
//!   2) CLAHE 会显著抬高高频噪声（实测 +50%），因此只在「确实发淡」的图上启用，
//!      强度压低，且放在去噪之后；
//!   3) 划痕修补必须只作用于细长结构，否则会把整张图的细节抹平。

use image::RgbImage;

use crate::imaging::{gray_of, open_image, thumbnail};

// ---------------------------------------------------------------- 基础工具

#[derive(Clone)]
pub struct Plane {
    pub w: usize,
    pub h: usize,
    pub data: Vec<f32>,
}

impl Plane {
    fn new(w: usize, h: usize) -> Self {
        Self {
            w,
            h,
            data: vec![0.0; w * h],
        }
    }

    fn get(&self, x: usize, y: usize) -> f32 {
        self.data[y * self.w + x]
    }

    fn set(&mut self, x: usize, y: usize, v: f32) {
        self.data[y * self.w + x] = v;
    }
}

/// RGB 图 -> 三个平面（0-255 f32）
fn to_planes(img: &RgbImage) -> [Plane; 3] {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut ps = [Plane::new(w, h), Plane::new(w, h), Plane::new(w, h)];
    for (i, px) in img.pixels().enumerate() {
        ps[0].data[i] = px.0[0] as f32;
        ps[1].data[i] = px.0[1] as f32;
        ps[2].data[i] = px.0[2] as f32;
    }
    ps
}

/// 三个平面 -> RGB 图（自动截断到 0-255）
fn from_planes(ps: &[Plane; 3]) -> RgbImage {
    let (w, h) = (ps[0].w as u32, ps[0].h as u32);
    let mut img = RgbImage::new(w, h);
    for i in 0..(w as usize * h as usize) {
        let x = (i % w as usize) as u32;
        let y = (i / w as usize) as u32;
        img.put_pixel(
            x,
            y,
            image::Rgb([
                ps[0].data[i].clamp(0.0, 255.0) as u8,
                ps[1].data[i].clamp(0.0, 255.0) as u8,
                ps[2].data[i].clamp(0.0, 255.0) as u8,
            ]),
        );
    }
    img
}

/// 灰度平面
pub fn gray_plane(img: &RgbImage) -> Plane {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let mut p = Plane::new(w, h);
    for (i, px) in img.pixels().enumerate() {
        p.data[i] = gray_of(px.0[0], px.0[1], px.0[2]) as f32;
    }
    p
}

/// 直方图分位数（0-100）
fn percentile(values: &[f32], pct: f64) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f32> = values.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let idx = ((pct / 100.0) * (s.len() - 1) as f64).clamp(0.0, (s.len() - 1) as f64);
    let lo = idx.floor() as usize;
    let hi = idx.ceil() as usize;
    if lo == hi {
        s[lo]
    } else {
        let t = idx - lo as f64;
        s[lo] * (1.0 - t as f32) + s[hi] * t as f32
    }
}

// ---------------------------------------------------------------- 自动色阶

/// 按百分位裁剪拉伸对比度，`clip` 为每侧裁剪百分比。
pub fn auto_levels(img: &RgbImage, clip: f64, per_channel: bool) -> RgbImage {
    let mut ps = to_planes(img);
    let (lo_p, hi_p) = (clip, 100.0 - clip);

    let all: Vec<f32> = ps.iter().flat_map(|p| p.data.iter().copied()).collect();
    if per_channel {
        for chan in ps.iter_mut() {
            let lo = percentile(&chan.data, lo_p);
            let hi = percentile(&chan.data, hi_p);
            let span = (hi - lo).max(1e-3);
            for v in chan.data.iter_mut() {
                *v = (*v - lo) / span * 255.0;
            }
        }
    } else {
        let lo = percentile(&all, lo_p);
        let hi = percentile(&all, hi_p);
        let span = (hi - lo).max(1e-3);
        for chan in ps.iter_mut() {
            for v in chan.data.iter_mut() {
                *v = (*v - lo) / span * 255.0;
            }
        }
    }
    from_planes(&ps)
}

/// 只拉灰度的自动色阶（保持色彩关系）
pub fn stretch_gray(img: &RgbImage, clip: f64) -> RgbImage {
    auto_levels(img, clip, false)
}

// ---------------------------------------------------------------- 白平衡

/// 灰世界假设：把三通道均值拉到同一水平。
pub fn gray_world(img: &RgbImage, strength: f32) -> RgbImage {
    let mut ps = to_planes(img);
    let n = (ps[0].w * ps[0].h).max(1) as f64;
    let mut means = [0f64; 3];
    for (c, p) in ps.iter().enumerate() {
        means[c] = p.data.iter().map(|v| *v as f64).sum::<f64>() / n;
        means[c] = means[c].max(1e-3);
    }
    let target = (means[0] + means[1] + means[2]) / 3.0;
    for (c, p) in ps.iter_mut().enumerate() {
        let gain = 1.0 + ((target / means[c]) as f32 - 1.0) * strength;
        for v in p.data.iter_mut() {
            *v *= gain;
        }
    }
    from_planes(&ps)
}

/// 白块假设：把最亮的部分视为白。
pub fn white_patch(img: &RgbImage, pct: f64, strength: f32) -> RgbImage {
    let mut ps = to_planes(img);
    for p in ps.iter_mut() {
        let hi = percentile(&p.data, pct).max(1e-3);
        let gain = 1.0 + ((255.0 / hi) - 1.0) * strength;
        for v in p.data.iter_mut() {
            *v *= gain;
        }
    }
    from_planes(&ps)
}

/// 估计并中和色偏（灰世界与白块的折中）。
pub fn neutral_cast(img: &RgbImage) -> RgbImage {
    let mut ps = to_planes(img);
    let n_all = (ps[0].w * ps[0].h).max(1) as f64;
    // 三通道一起算，才能真正"中和"偏色
    let mut means = [0f64; 3];
    for (c, p) in ps.iter().enumerate() {
        means[c] = (p.data.iter().map(|v| *v as f64).sum::<f64>() / n_all).max(1e-3);
    }
    let mean_of_means = (means[0] + means[1] + means[2]) / 3.0;
    for (c, p) in ps.iter_mut().enumerate() {
        let hi = percentile(&p.data, 99.0).max(1e-3) as f64;
        let g1 = mean_of_means / means[c];
        let g2 = 255.0 / hi;
        let g = ((g1 * 0.6 + g2 * 0.4) as f32).clamp(0.6, 1.8);
        for v in p.data.iter_mut() {
            *v *= g;
        }
    }
    from_planes(&ps)
}

/// 专门压制泛黄：蓝通道按黄度补偿，同时轻微压红。
pub fn deyellow(img: &RgbImage, strength: f32) -> RgbImage {
    let mut ps = to_planes(img);
    let n = (ps[0].w * ps[0].h).max(1) as f64;
    // 黄度 = ((R+G)/2 - B) / 255
    let mut yellowness = vec![0f32; ps[0].data.len()];
    let mut sum = 0f64;
    for i in 0..yellowness.len() {
        let y = ((ps[0].data[i] + ps[1].data[i]) * 0.5 - ps[2].data[i]) / 255.0;
        let y = y.clamp(0.0, 1.0);
        yellowness[i] = y;
        sum += y as f64;
    }
    let k = ((sum / n * 2.0) as f32).clamp(0.0, 1.0) * strength;
    if k <= 0.01 {
        return img.clone();
    }
    for i in 0..yellowness.len() {
        let y = yellowness[i];
        ps[2].data[i] = (ps[2].data[i] + k * 42.0 * (y * 0.7 + 0.3)).clamp(0.0, 255.0);
        ps[0].data[i] = (ps[0].data[i] - k * 12.0 * (y * 0.5 + 0.5)).clamp(0.0, 255.0);
    }
    from_planes(&ps)
}

/// 对比度调整（围绕中灰缩放）：等价于 PIL `ImageEnhance.Contrast`。
pub fn adjust_contrast(img: &RgbImage, factor: f32) -> RgbImage {
    let mut ps = to_planes(img);
    // PIL 用整图均值作为"中灰"
    let n = (ps[0].w * ps[0].h).max(1) as f64;
    let mut mean = 0f64;
    for p in ps.iter() {
        mean += p.data.iter().map(|v| *v as f64).sum::<f64>();
    }
    mean /= n * 3.0;
    for p in ps.iter_mut() {
        for v in p.data.iter_mut() {
            *v = mean as f32 + (*v - mean as f32) * factor;
        }
    }
    from_planes(&ps)
}

/// 饱和度调整：等价于 PIL `ImageEnhance.Color`（向灰度插值）。
pub fn adjust_saturation(img: &RgbImage, factor: f32) -> RgbImage {
    let mut ps = to_planes(img);
    for i in 0..ps[0].data.len() {
        let g = ps[0].data[i] * 0.299 + ps[1].data[i] * 0.587 + ps[2].data[i] * 0.114;
        for c in 0..3 {
            ps[c].data[i] = g + (ps[c].data[i] - g) * factor;
        }
    }
    from_planes(&ps)
}

/// 测出饱和度后补到目标值附近（老照片褪色越重补得越多）。
pub fn saturate_by_fade(img: &RgbImage, target: f32) -> RgbImage {
    let ps = to_planes(img);
    let n = (ps[0].w * ps[0].h).max(1) as f64;
    let mut sum = 0f64;
    for i in 0..ps[0].data.len() {
        let (r, g, b) = (ps[0].data[i], ps[1].data[i], ps[2].data[i]);
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let s = if mx > 1e-3 { (mx - mn) / mx } else { 0.0 };
        sum += s as f64;
    }
    let cur = (sum / n) as f32;
    if cur >= target || cur <= 1e-4 {
        return img.clone();
    }
    let factor = (target / cur).clamp(1.0, 1.9);
    adjust_saturation(img, factor)
}

// ---------------------------------------------------------------- 中值滤波

/// 中值滤波（方形窗口，边界复制）。
pub fn median_filter(img: &RgbImage, k: usize) -> RgbImage {
    if k <= 1 {
        return img.clone();
    }
    let mut ps = to_planes(img);
    let r = k / 2;
    for p in ps.iter_mut() {
        *p = median_filter_plane(p, r);
    }
    from_planes(&ps)
}

fn median_filter_plane(src: &Plane, r: usize) -> Plane {
    let (w, h) = (src.w, src.h);
    let mut out = Plane::new(w, h);
    let mut buf: Vec<f32> = Vec::with_capacity((2 * r + 1) * (2 * r + 1));
    for y in 0..h {
        for x in 0..w {
            buf.clear();
            for dy in 0..=(2 * r) {
                let sy = (y + dy).saturating_sub(r).min(h - 1);
                for dx in 0..=(2 * r) {
                    let sx = (x + dx).saturating_sub(r).min(w - 1);
                    buf.push(src.get(sx, sy));
                }
            }
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            out.set(x, y, buf[buf.len() / 2]);
        }
    }
    out
}

/// 高斯模糊（3x3 近似，σ≈0.6）
pub fn gaussian_blur(img: &RgbImage) -> RgbImage {
    let mut ps = to_planes(img);
    // 一维 [1,2,1]/4 两次 = 近似 3x3 高斯
    for p in ps.iter_mut() {
        let tmp = box_blur_1d(p, true);
        *p = box_blur_1d(&tmp, false);
    }
    from_planes(&ps)
}

fn box_blur_1d(src: &Plane, horizontal: bool) -> Plane {
    let (w, h) = (src.w, src.h);
    let mut out = Plane::new(w, h);
    if horizontal {
        for y in 0..h {
            for x in 0..w {
                let l = src.get(x.saturating_sub(1), y);
                let c = src.get(x, y);
                let r = src.get((x + 1).min(w - 1), y);
                out.set(x, y, (l + 2.0 * c + r) / 4.0);
            }
        }
    } else {
        for y in 0..h {
            for x in 0..w {
                let t = src.get(x, y.saturating_sub(1));
                let c = src.get(x, y);
                let b = src.get(x, (y + 1).min(h - 1));
                out.set(x, y, (t + 2.0 * c + b) / 4.0);
            }
        }
    }
    out
}

/// 中值 + 高斯混合去噪。
pub fn denoise(img: &RgbImage, strength: u32) -> RgbImage {
    let mut out = img.clone();
    if strength >= 1 {
        out = median_filter(&out, 3);
    }
    if strength >= 2 {
        out = median_filter(&out, 3);
    }
    if strength >= 3 {
        out = gaussian_blur(&out);
    }
    out
}

// ---------------------------------------------------------------- 细线检测

/// 一维中值滤波（沿单一方向，边界复制）。
fn median1d(src: &Plane, k: usize, horizontal: bool) -> Plane {
    let (w, h) = (src.w, src.h);
    let r = k / 2;
    let mut out = Plane::new(w, h);
    let mut buf: Vec<f32> = Vec::with_capacity(k);
    for y in 0..h {
        for x in 0..w {
            buf.clear();
            for t in 0..k {
                let (sx, sy) = if horizontal {
                    ((x + t).saturating_sub(r).min(w - 1), y)
                } else {
                    (x, (y + t).saturating_sub(r).min(h - 1))
                };
                buf.push(src.get(sx, sy));
            }
            buf.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            out.set(x, y, buf[buf.len() / 2]);
        }
    }
    out
}

/// 提取「细亮线」与「细暗线」的响应图。
///
/// 关键点（这里踩过三次坑）：
///   * 开运算只能去掉小于结构元的斑点，对长直线无效；
///   * 闭运算对细亮线同样无效；
///   * 真正有效的是**一维中值滤波**：沿水平方向取中值能抹掉竖线，
///     原图减去中值图就得到这条竖线的亮度响应。
pub fn thin_line_response(g: &Plane, k: usize) -> (Plane, Plane) {
    let m_h = median1d(g, k, true);
    let m_v = median1d(g, k, false);

    let mut bright = Plane::new(g.w, g.h);
    for i in 0..g.data.len() {
        bright.data[i] = (g.data[i] - m_h.data[i]).max(g.data[i] - m_v.data[i]);
    }

    let mut neg = Plane::new(g.w, g.h);
    for i in 0..g.data.len() {
        neg.data[i] = -g.data[i];
    }
    let n_h = median1d(&neg, k, true);
    let n_v = median1d(&neg, k, false);
    let mut dark = Plane::new(g.w, g.h);
    for i in 0..g.data.len() {
        dark.data[i] = (neg.data[i] - n_h.data[i]).max(neg.data[i] - n_v.data[i]);
    }
    (bright, dark)
}

/// 二维最大值膨胀（方形结构元），±1 像素。
fn dilate(mask: &Plane, r: usize) -> Plane {
    if r < 1 {
        return mask.clone();
    }
    let (w, h) = (mask.w, mask.h);
    let mut out = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut m = f32::MIN;
            for dy in 0..=(2 * r) {
                let sy = (y + dy).saturating_sub(r).min(h - 1);
                for dx in 0..=(2 * r) {
                    let sx = (x + dx).saturating_sub(r).min(w - 1);
                    m = m.max(mask.get(sx, sy));
                }
            }
            out.set(x, y, m);
        }
    }
    out
}

/// 去除划痕、灰尘、霉点、折痕。
///
/// 只处理「细长且与周围反差明显」的像素；命中面积过大就整体放弃（宁可不修也不糊图）。
pub fn despeckle(
    img: &RgbImage,
    threshold: f32,
    dilate_r: usize,
    max_ratio: f32,
) -> RgbImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if h < 16 || w < 16 {
        return img.clone();
    }
    let g = gray_plane(img);
    let (th, bh) = thin_line_response(&g, 5);

    let mut resp = Plane::new(w, h);
    let mut any = false;
    for i in 0..resp.data.len() {
        let v = th.data[i].max(bh.data[i]);
        resp.data[i] = v;
        if v > threshold {
            any = true;
        }
    }
    if !any {
        return img.clone();
    }

    // 过滤物体边缘：5x5 邻域内命中占比高的地方是大面积边缘，不是划痕
    let mut suspect = Plane::new(w, h);
    for i in 0..suspect.data.len() {
        suspect.data[i] = if resp.data[i] > threshold { 1.0 } else { 0.0 };
    }
    let mut local = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut sum = 0f32;
            for dy in 0..5usize {
                let sy = (y + dy).saturating_sub(2).min(h - 1);
                for dx in 0..5usize {
                    let sx = (x + dx).saturating_sub(2).min(w - 1);
                    sum += suspect.get(sx, sy);
                }
            }
            local.set(x, y, sum / 25.0);
        }
    }

    let mut hits = 0usize;
    let mut mask = Plane::new(w, h);
    for i in 0..mask.data.len() {
        let hit = suspect.data[i] > 0.5 && local.data[i] < 0.55;
        mask.data[i] = if hit { 1.0 } else { 0.0 };
        if hit {
            hits += 1;
        }
    }
    if hits == 0 {
        return img.clone();
    }
    if hits as f32 > h as f32 * w as f32 * max_ratio {
        // 命中过多，判据不可靠，宁可原样返回
        return img.clone();
    }

    let mask_d = dilate(&mask, dilate_r.max(1));
    // 强度渐入：响应越强，修补越彻底
    let mut strength = Plane::new(w, h);
    for i in 0..strength.data.len() {
        strength.data[i] =
            ((resp.data[i] - threshold) / (threshold * 2.0).max(1.0)).clamp(0.0, 1.0);
    }
    let strength_d = dilate(&strength, dilate_r.max(1));

    let ref_img = median_filter_plane3(img, 3);
    let mut ps = to_planes(img);
    let rps = to_planes(&ref_img);
    for c in 0..3 {
        for i in 0..ps[c].data.len() {
            let wgt = (mask_d.data[i] * (0.45 + strength_d.data[i] * 1.2).clamp(0.0, 1.0))
                .clamp(0.0, 1.0);
            ps[c].data[i] = ps[c].data[i] * (1.0 - wgt) + rps[c].data[i] * wgt;
        }
    }
    from_planes(&ps)
}

fn median_filter_plane3(img: &RgbImage, k: usize) -> RgbImage {
    median_filter(img, k)
}

// ---------------------------------------------------------------- CLAHE

/// 分块自适应直方图均衡（在 Y 通道上做，避免色偏）。
pub fn clahe(img: &RgbImage, clip_limit: f32, tiles: usize) -> RgbImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if h < tiles || w < tiles {
        return img.clone();
    }
    // RGB -> YCbCr（整数近似，与 PIL 一致）
    let mut y = Plane::new(w, h);
    let mut cb = Plane::new(w, h);
    let mut cr = Plane::new(w, h);
    for (i, px) in img.pixels().enumerate() {
        let (r, g, b) = (px.0[0] as f32, px.0[1] as f32, px.0[2] as f32);
        y.data[i] = 0.299 * r + 0.587 * g + 0.114 * b;
        cb.data[i] = 128.0 - 0.168_736 * r - 0.331_264 * g + 0.5 * b;
        cr.data[i] = 128.0 + 0.5 * r - 0.418_688 * g - 0.081_312 * b;
    }

    // 每个 tile 的查找表
    let mut lut = vec![vec![0f32; 256]; tiles * tiles];
    for ty in 0..tiles {
        for tx in 0..tiles {
            let x0 = tx * w / tiles;
            let x1 = ((tx + 1) * w / tiles).max(x0 + 1).min(w);
            let y0 = ty * h / tiles;
            let y1 = ((ty + 1) * h / tiles).max(y0 + 1).min(h);
            let mut hist = [0f32; 256];
            let mut count = 0f32;
            for yy in y0..y1 {
                for xx in x0..x1 {
                    let v = y.get(xx, yy).clamp(0.0, 255.0) as usize;
                    hist[v] += 1.0;
                    count += 1.0;
                }
            }
            if count == 0.0 {
                for i in 0..256 {
                    lut[ty * tiles + tx][i] = i as f32 / 255.0;
                }
                continue;
            }
            let lim = (clip_limit * count / 256.0).max(1.0);
            let mut excess = 0f32;
            for hval in hist.iter_mut() {
                if *hval > lim {
                    excess += *hval - lim;
                    *hval = lim;
                }
            }
            let add = excess / 256.0;
            let mut cdf = 0f32;
            let mut total = 0f32;
            for v in hist.iter() {
                total += *v + add;
            }
            for i in 0..256 {
                cdf += hist[i] + add;
                lut[ty * tiles + tx][i] = (cdf / total.max(1.0)).clamp(0.0, 1.0);
            }
        }
    }

    // 双线性插值
    let mut out_y = Plane::new(w, h);
    for yy in 0..h {
        // 像素在 tile 网格中的浮点位置
        let cy = ((yy as f32 + 0.5) * tiles as f32 / h as f32 - 0.5).clamp(0.0, (tiles - 1) as f32);
        let y0 = cy.floor() as usize;
        let y1 = (y0 + 1).min(tiles - 1);
        let fy = cy - y0 as f32;
        for xx in 0..w {
            let cx =
                ((xx as f32 + 0.5) * tiles as f32 / w as f32 - 0.5).clamp(0.0, (tiles - 1) as f32);
            let x0 = cx.floor() as usize;
            let x1 = (x0 + 1).min(tiles - 1);
            let fx = cx - x0 as f32;

            let v = y.get(xx, yy).clamp(0.0, 255.0) as usize;
            let l00 = lut[y0 * tiles + x0][v];
            let l01 = lut[y0 * tiles + x1][v];
            let l10 = lut[y1 * tiles + x0][v];
            let l11 = lut[y1 * tiles + x1][v];
            let top = l00 * (1.0 - fx) + l01 * fx;
            let bot = l10 * (1.0 - fx) + l11 * fx;
            out_y.set(xx, yy, (top * (1.0 - fy) + bot * fy) * 255.0);
        }
    }

    // YCbCr -> RGB
    let mut out = RgbImage::new(w as u32, h as u32);
    for yy in 0..h {
        for xx in 0..w {
            let i = yy * w + xx;
            let (yyv, cbb, crr) = (out_y.data[i], cb.data[i] - 128.0, cr.data[i] - 128.0);
            let r = yyv + 1.402 * crr;
            let g = yyv - 0.344_136 * cbb - 0.714_136 * crr;
            let b = yyv + 1.772 * cbb;
            out.put_pixel(
                xx as u32,
                yy as u32,
                image::Rgb([
                    r.clamp(0.0, 255.0) as u8,
                    g.clamp(0.0, 255.0) as u8,
                    b.clamp(0.0, 255.0) as u8,
                ]),
            );
        }
    }
    out
}

// ---------------------------------------------------------------- 锐化

/// 反锐化掩模。
pub fn unsharp(img: &RgbImage, radius: f32, percent: f32, threshold: f32) -> RgbImage {
    if percent <= 0.0 {
        return img.clone();
    }
    // 用高斯模糊近似 PIL 的 UnsharpMask 的模糊核
    let mut blurred = img.clone();
    let passes = (radius.round() as usize).clamp(1, 3);
    for _ in 0..passes {
        blurred = gaussian_blur(&blurred);
    }
    let mut ps = to_planes(img);
    let bps = to_planes(&blurred);
    for c in 0..3 {
        for i in 0..ps[c].data.len() {
            let diff = ps[c].data[i] - bps[c].data[i];
            if diff.abs() >= threshold {
                ps[c].data[i] += diff * (percent / 100.0);
            }
        }
    }
    from_planes(&ps)
}

/// 边缘感知锐化：只在梯度明显的区域锐化，平坦区原样保留。
pub fn edge_sharpen(img: &RgbImage, amount: f32, radius: f32, threshold: f32) -> RgbImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    let g = gray_plane(img);

    // Sobel 梯度幅值
    let mut mag = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let l = g.get(x.saturating_sub(1), y);
            let r = g.get((x + 1).min(w - 1), y);
            let t = g.get(x, y.saturating_sub(1));
            let b = g.get(x, (y + 1).min(h - 1));
            mag.set(x, y, ((r - l).powi(2) + (b - t).powi(2)).sqrt());
        }
    }
    let mut mask = Plane::new(w, h);
    for i in 0..mask.data.len() {
        mask.data[i] = ((mag.data[i] - threshold) / 40.0).clamp(0.0, 1.0);
    }
    // 膨胀一圈，让边缘带一点宽度，避免锐化后出现断点
    let mask = dilate(&mask, 1);

    let sharp = unsharp(img, radius, 100.0 * amount, 2.0);
    let mut ps = to_planes(img);
    let sps = to_planes(&sharp);
    for c in 0..3 {
        for i in 0..ps[c].data.len() {
            let m = mask.data[i].clamp(0.0, 1.0);
            ps[c].data[i] = ps[c].data[i] * (1.0 - m) + sps[c].data[i] * m;
        }
    }
    from_planes(&ps)
}

// ---------------------------------------------------------------- 边框裁切

/// 裁掉扫描产生的大面积纯色边框（白边/黑边）。
pub fn crop_border(img: &RgbImage, tol: f32, max_frac: f32) -> RgbImage {
    let (w, h) = (img.width() as usize, img.height() as usize);
    if h < 20 || w < 20 {
        return img.clone();
    }
    // 四边像素的中位数作为背景色
    let mut border: Vec<(f32, f32, f32)> = Vec::new();
    for x in 0..w {
        for px in [img.get_pixel(x as u32, 0), img.get_pixel(x as u32, (h - 1) as u32)] {
            border.push((px.0[0] as f32, px.0[1] as f32, px.0[2] as f32));
        }
    }
    for y in 0..h {
        for px in [img.get_pixel(0, y as u32), img.get_pixel((w - 1) as u32, y as u32)] {
            border.push((px.0[0] as f32, px.0[1] as f32, px.0[2] as f32));
        }
    }
    let med = |idx: usize| -> f32 {
        let mut v: Vec<f32> = border.iter().map(|p| match idx { 0 => p.0, 1 => p.1, _ => p.2 }).collect();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        v.get(v.len() / 2).copied().unwrap_or(0.0)
    };
    let bg = (med(0), med(1), med(2));

    // 每行/每列是否有"内容"
    let is_content = |px: &image::Rgb<u8>| -> bool {
        let d = [
            (px.0[0] as f32 - bg.0).abs(),
            (px.0[1] as f32 - bg.1).abs(),
            (px.0[2] as f32 - bg.2).abs(),
        ];
        d[0].max(d[1]).max(d[2]) > tol
    };

    let row_hits: Vec<usize> = (0..h)
        .map(|y| (0..w).filter(|x| is_content(img.get_pixel(*x as u32, y as u32))).count())
        .collect();
    let col_hits: Vec<usize> = (0..w)
        .map(|x| (0..h).filter(|y| is_content(img.get_pixel(x as u32, *y as u32))).count())
        .collect();

    let rows: Vec<usize> = (0..h).filter(|y| row_hits[*y] as f32 > w as f32 * 0.02).collect();
    let cols: Vec<usize> = (0..w).filter(|x| col_hits[*x] as f32 > h as f32 * 0.02).collect();
    if rows.len() < 5 || cols.len() < 5 {
        return img.clone();
    }
    let (t, b) = (rows[0], rows[rows.len() - 1] + 1);
    let (l, r) = (cols[0], cols[cols.len() - 1] + 1);

    // 边框比例过大 → 说明这张图本来就没有扫描边，原样返回。
    // 绝不为了"保险"去裁掉一圈：那会凭空降低分辨率并引入插值损失。
    let max_h = h as f32 * max_frac;
    let max_w = w as f32 * max_frac;
    if t as f32 > max_h || l as f32 > max_w || (h - b) as f32 > max_h || (w - r) as f32 > max_w {
        return img.clone();
    }
    if t == 0 && l == 0 && b == h && r == w {
        return img.clone();
    }
    image::imageops::crop_imm(img, l as u32, t as u32, (r - l) as u32, (b - t) as u32).to_image()
}

// ---------------------------------------------------------------- 纠偏

/// 投影方差得分：图像摆正时，行投影的起伏最剧烈。
fn proj_score(gray: &Plane) -> f64 {
    let (w, h) = (gray.w, gray.h);
    let mut prof = vec![0f64; h];
    for y in 0..h {
        let mut s = 0f64;
        for x in 0..w {
            s += gray.get(x, y) as f64;
        }
        prof[y] = s;
    }
    let mean = prof.iter().sum::<f64>() / h.max(1) as f64;
    prof.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / h.max(1) as f64
}

/// 用投影方差法估计倾斜角（度）。
pub fn detect_skew(img: &RgbImage, max_angle: f32, step: f32) -> f32 {
    let small = thumbnail(img, 420);
    let g = gray_plane(&small);
    let mut best = 0.0f32;
    let mut best_score = -1.0f64;
    let mut ang = -max_angle;
    while ang <= max_angle + 1e-6 {
        let rot = rotate_gray(&g, ang);
        let score = proj_score(&rot);
        if score > best_score {
            best_score = score;
            best = ang;
        }
        ang += step;
    }
    best
}

/// 旋转灰度平面（双线性，边界填均值）。
fn rotate_gray(src: &Plane, deg: f32) -> Plane {
    let (w, h) = (src.w, src.h);
    let mean = src.data.iter().sum::<f32>() / src.data.len().max(1) as f32;
    let rad = -deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    let cx = w as f32 / 2.0;
    let cy = h as f32 / 2.0;
    let mut out = Plane::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let sx = cos * dx + sin * dy + cx;
            let sy = -sin * dx + cos * dy + cy;
            out.set(x, y, sample_bilinear(src, sx, sy, mean));
        }
    }
    out
}

fn sample_bilinear(src: &Plane, x: f32, y: f32, fill: f32) -> f32 {
    if x < 0.0 || y < 0.0 || x >= (src.w - 1) as f32 || y >= (src.h - 1) as f32 {
        return fill;
    }
    let x0 = x.floor() as usize;
    let y0 = y.floor() as usize;
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let a = src.get(x0, y0);
    let b = src.get(x0 + 1, y0);
    let c = src.get(x0, y0 + 1);
    let d = src.get(x0 + 1, y0 + 1);
    a * (1.0 - fx) * (1.0 - fy)
        + b * fx * (1.0 - fy)
        + c * (1.0 - fx) * fy
        + d * fx * fy
}

/// 判断是不是截图/文档/界面图：这类图不该做纠偏。
///
/// 判据用纯色像素占比 + 高饱和界面色 + 量化色种类，三者都在截图里特别明显。
///
/// 注意 `flat` 只看严格的白(>245)/黑(<12)，所以「有大片亮天空的照片」
/// 也可能命中。为了不误伤这类真实照片，命中 flat 后还要过一道
/// **色彩丰富度**检查：真实照片（即便是合成图）的量化色种类远多于截图。
pub fn is_screen_or_document(img: &RgbImage, m: Option<&crate::imaging::Metrics>) -> bool {
    let a = thumbnail(img, 200);
    let total = (a.width() as f64 * a.height() as f64).max(1.0);
    let mut flat = 0usize;
    let mut dark = 0usize;
    let mut vivid = 0usize;
    for px in a.pixels() {
        let [r, g, b] = px.0;
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        if (r > 245 && g > 245 && b > 245) || (r < 12 && g < 12 && b < 12) {
            flat += 1;
        }
        if mx < 60 {
            dark += 1;
        }
        if mx as i32 - mn as i32 > 110 {
            vivid += 1;
        }
    }
    let flat = flat as f64 / total;
    let dark = dark as f64 / total;
    let vivid = vivid as f64 / total;

    // 量化色种类：每通道 16 级（4 bit），共 4096 桶。
    // 用 32 级太粗，200x200 的图撑不起差异（真实照片和截图都只有 30 来个桶）；
    // 16 级下真实照片仍能铺开几百桶，而界面截图集中在少数几十个。
    let mut seen = std::collections::HashSet::new();
    for px in a.pixels() {
        let q = |v: u8| (v / 16) as u32;
        seen.insert(q(px.0[0]) * 256 + q(px.0[1]) * 16 + q(px.0[2]));
    }
    let uniq = seen.len();

    // 高饱和「界面色块」：必须同时色彩贫乏才算。
    //
    // 单独用 vivid 会误伤真实照片 —— 合成图的天空渐变色相跨度大，
    // 实测「春节全家福」vivid 就有 0.096。
    if vivid > 0.08 && uniq < 60 {
        return true;
    }
    // 大面积纯白/纯黑 → 文档/终端
    if flat > 0.30 && uniq < 120 {
        return true;
    }
    // 深色界面：近黑占比大 + 色彩相对贫乏。
    //
    // 实测深色 IDE/浏览器截图 dark 在 0.55~0.79、uniq 在 250~360
    // （图标和语法高亮会贡献不少色阶），而真实照片 dark 基本为 0。
    // 所以这里只要 dark 足够高、uniq 没到照片那种量级就能判。
    if dark > 0.32 && uniq < 400 {
        return true;
    }
    if let Some(m) = m {
        // 文字边缘会被当成噪点，这类「高噪点 + 有对比 + 色彩贫乏」基本是截图
        if m.noise > 6.0 && m.contrast > 0.12 && uniq < 150 {
            return true;
        }
    }
    false
}

/// 自动纠偏（仅用于照片，截图/文档自动跳过）。
///
/// 三道保险，避免对本来就水平的照片做无意义旋转（旋转会引入插值噪声）：
///   0) 截图/文档直接跳过；
///   1) 角度小于 min_angle 不转；
///   2) 旋转的投影方差增益不足 min_gain 不转。
pub fn deskew(
    img: &RgbImage,
    angle: Option<f32>,
    max_angle: f32,
    min_angle: f32,
    min_gain: f64,
) -> RgbImage {
    if is_screen_or_document(img, None) {
        return img.clone();
    }
    let small = thumbnail(img, 420);
    let g = gray_plane(&small);

    let angle = match angle {
        Some(a) => a,
        None => {
            let base = proj_score(&g);
            let mut best_angle = 0.0f32;
            let mut best_score = base;
            let mut a = -max_angle;
            while a <= max_angle + 1e-6 {
                if a.abs() >= min_angle {
                    let rot = rotate_gray(&g, a);
                    let sc = proj_score(&rot);
                    if sc > best_score {
                        best_score = sc;
                        best_angle = a;
                    }
                }
                a += 0.5;
            }
            if best_angle.abs() < min_angle {
                return img.clone();
            }
            if base > 0.0 && best_score / base < min_gain {
                return img.clone();
            }
            best_angle
        }
    };
    if angle.abs() < min_angle {
        return img.clone();
    }
    rotate_rgb_expand(img, angle)
}

/// 旋转并以白色填充（对应 PIL 的 `rotate(expand=True, fillcolor=white)`）。
fn rotate_rgb_expand(img: &RgbImage, deg: f32) -> RgbImage {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let rad = deg.to_radians();
    let (sin, cos) = rad.sin_cos();
    // 旋转后的包围盒
    let nw = (w * cos.abs() + h * sin.abs()).ceil().max(1.0) as u32;
    let nh = (w * sin.abs() + h * cos.abs()).ceil().max(1.0) as u32;
    let mut out = RgbImage::new(nw, nh);
    let (cx, cy) = (w / 2.0, h / 2.0);
    let (ncx, ncy) = (nw as f32 / 2.0, nh as f32 / 2.0);
    for y in 0..nh {
        for x in 0..nw {
            let dx = x as f32 - ncx;
            let dy = y as f32 - ncy;
            let sx = cos * dx - sin * dy + cx;
            let sy = sin * dx + cos * dy + cy;
            let px = if sx < 0.0 || sy < 0.0 || sx >= w - 1.0 || sy >= h - 1.0 {
                image::Rgb([255, 255, 255])
            } else {
                let x0 = sx.floor() as u32;
                let y0 = sy.floor() as u32;
                let fx = sx - x0 as f32;
                let fy = sy - y0 as f32;
                let a = img.get_pixel(x0, y0).0;
                let b = img.get_pixel(x0 + 1, y0).0;
                let c = img.get_pixel(x0, y0 + 1).0;
                let d = img.get_pixel(x0 + 1, y0 + 1).0;
                let mut ch = [0u8; 3];
                for i in 0..3 {
                    let v = a[i] as f32 * (1.0 - fx) * (1.0 - fy)
                        + b[i] as f32 * fx * (1.0 - fy)
                        + c[i] as f32 * (1.0 - fx) * fy
                        + d[i] as f32 * fx * fy;
                    ch[i] = v.clamp(0.0, 255.0) as u8;
                }
                image::Rgb(ch)
            };
            out.put_pixel(x, y, px);
        }
    }
    out
}

// ---------------------------------------------------------------- 红眼 / 放大

/// 红眼抑制。
pub fn fix_red_eye(img: &RgbImage, threshold: f32) -> RgbImage {
    let mut ps = to_planes(img);
    let n = ps[0].data.len();
    let mut mask = Plane::new(ps[0].w, ps[0].h);
    let mut any = false;
    for i in 0..n {
        let (r, g, b) = (ps[0].data[i], ps[1].data[i], ps[2].data[i]);
        let avg = ((g + b) * 0.5).max(1.0);
        if r > 70.0 && r > threshold * avg {
            mask.data[i] = 1.0;
            any = true;
        }
    }
    if !any {
        return img.clone();
    }
    // 3x3 腐蚀，收紧掩膜
    let mut eroded = Plane::new(mask.w, mask.h);
    for y in 0..mask.h {
        for x in 0..mask.w {
            let mut m = f32::MAX;
            for dy in 0..3usize {
                let sy = (y + dy).saturating_sub(1).min(mask.h - 1);
                for dx in 0..3usize {
                    let sx = (x + dx).saturating_sub(1).min(mask.w - 1);
                    m = m.min(mask.get(sx, sy));
                }
            }
            eroded.set(x, y, m);
        }
    }
    for i in 0..n {
        let m = eroded.data[i];
        if m <= 0.0 {
            continue;
        }
        let avg = (ps[1].data[i] + ps[2].data[i]) * 0.5;
        ps[0].data[i] = ps[0].data[i] * (1.0 - m) + avg * m;
    }
    from_planes(&ps)
}

/// 高质量放大。
pub fn upscale(img: &RgbImage, factor: f32, max_side: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let mut nw = (w as f32 * factor) as u32;
    let mut nh = (h as f32 * factor) as u32;
    if nw.max(nh) > max_side {
        let s = max_side as f32 / nw.max(nh) as f32;
        nw = (nw as f32 * s) as u32;
        nh = (nh as f32 * s) as u32;
    }
    if nw <= w || nw == 0 || nh == 0 {
        return img.clone();
    }
    let up = crate::imaging::resize_lanczos(img, nw, nh);
    unsharp(&up, 1.2, 70.0, 2.0)
}

// ---------------------------------------------------------------- 全流程

/// 各开关的默认值（`clahe` 用 `"auto"` 表示自动判断）
#[derive(Debug, Clone)]
pub struct Options {
    pub crop: bool,
    pub deskew: bool,
    pub denoise: bool,
    pub despeckle: bool,
    pub whitebalance: bool,
    pub deyellow: bool,
    pub levels: bool,
    /// 三态：自动 / 强制开 / 强制关
    pub clahe: ClaheMode,
    pub fade: bool,
    pub sharpen: bool,
    pub upscale: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaheMode {
    Auto,
    On,
    Off,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            crop: true,
            deskew: true,
            denoise: true,
            despeckle: true,
            whitebalance: true,
            deyellow: true,
            levels: true,
            clahe: ClaheMode::Auto,
            fade: true,
            sharpen: true,
            upscale: false,
        }
    }
}

impl Options {
    /// 从字符串键值覆盖（键名与前端一致）。
    pub fn set(&mut self, key: &str, value: &serde_json::Value) {
        let b = |v: &serde_json::Value| -> Option<bool> {
            if let Some(x) = v.as_bool() {
                return Some(x);
            }
            v.as_i64().map(|i| i != 0)
        };
        match key {
            "crop" => self.crop = b(value).unwrap_or(self.crop),
            "deskew" => self.deskew = b(value).unwrap_or(self.deskew),
            "denoise" => self.denoise = b(value).unwrap_or(self.denoise),
            "despeckle" => self.despeckle = b(value).unwrap_or(self.despeckle),
            "whitebalance" => self.whitebalance = b(value).unwrap_or(self.whitebalance),
            "deyellow" => self.deyellow = b(value).unwrap_or(self.deyellow),
            "levels" => self.levels = b(value).unwrap_or(self.levels),
            "fade" => self.fade = b(value).unwrap_or(self.fade),
            "sharpen" => self.sharpen = b(value).unwrap_or(self.sharpen),
            "upscale" => self.upscale = b(value).unwrap_or(self.upscale),
            "clahe" => {
                self.clahe = match value {
                    serde_json::Value::Bool(true) => ClaheMode::On,
                    serde_json::Value::Bool(false) => ClaheMode::Off,
                    serde_json::Value::String(s) if s == "auto" => ClaheMode::Auto,
                    _ => self.clahe,
                };
            }
            _ => {}
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "crop": self.crop,
            "deskew": self.deskew,
            "denoise": self.denoise,
            "despeckle": self.despeckle,
            "whitebalance": self.whitebalance,
            "deyellow": self.deyellow,
            "levels": self.levels,
            "clahe": match self.clahe {
                ClaheMode::Auto => serde_json::Value::String("auto".into()),
                ClaheMode::On => serde_json::Value::Bool(true),
                ClaheMode::Off => serde_json::Value::Bool(false),
            },
            "fade": self.fade,
            "sharpen": self.sharpen,
            "upscale": self.upscale,
        })
    }
}

/// 预设 -> 开关覆盖
pub const PRESETS: &[(&str, &[(&str, &str)])] = &[
    ("轻度修复", &[("despeckle", "0"), ("clahe", "0"), ("sharpen", "1"), ("upscale", "0")]),
    ("标准修复", &[]),
    ("重度修复", &[("upscale", "0"), ("clahe", "1")]),
    (
        "仅去噪",
        &[
            ("crop", "0"), ("deskew", "0"), ("whitebalance", "0"), ("deyellow", "0"),
            ("levels", "0"), ("clahe", "0"), ("fade", "0"), ("sharpen", "0"),
            ("despeckle", "0"),
        ],
    ),
    (
        "仅调色",
        &[
            ("crop", "0"), ("deskew", "0"), ("denoise", "0"), ("despeckle", "0"),
            ("sharpen", "0"),
        ],
    ),
    (
        "仅裁边纠偏",
        &[
            ("denoise", "0"), ("despeckle", "0"), ("whitebalance", "0"), ("deyellow", "0"),
            ("levels", "0"), ("clahe", "0"), ("fade", "0"), ("sharpen", "0"),
        ],
    ),
    (
        "去泛黄",
        &[
            ("crop", "0"), ("deskew", "0"), ("denoise", "0"), ("despeckle", "0"),
            ("levels", "0"), ("clahe", "0"), ("fade", "0"), ("sharpen", "0"),
        ],
    ),
    (
        "去划痕",
        &[
            ("crop", "0"), ("deskew", "0"), ("whitebalance", "0"), ("deyellow", "0"),
            ("levels", "0"), ("clahe", "0"), ("fade", "0"), ("sharpen", "0"),
        ],
    ),
    (
        "增强清晰度",
        &[
            ("crop", "0"), ("deskew", "0"), ("whitebalance", "0"), ("deyellow", "0"),
            ("levels", "0"), ("clahe", "1"), ("fade", "0"),
        ],
    ),
];

/// 前端展示用的开关说明
pub fn options_meta() -> serde_json::Value {
    serde_json::json!([
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
    ])
}

pub fn preset_names() -> Vec<&'static str> {
    PRESETS.iter().map(|(n, _)| *n).collect()
}

/// 把预设展开成完整开关表（前端显示勾选状态用）。
pub fn preset_options(name: &str) -> Options {
    let mut o = Options::default();
    if let Some((_, overrides)) = PRESETS.iter().find(|(n, _)| *n == name) {
        for (k, v) in overrides.iter() {
            match *k {
                "clahe" => {
                    o.clahe = match *v {
                        "auto" => ClaheMode::Auto,
                        "0" => ClaheMode::Off,
                        _ => ClaheMode::On,
                    };
                }
                key => o.set(key, &serde_json::json!(*v == "1")),
            }
        }
    }
    o
}

/// 截图/文档的增强：只做「提对比 + 去偏色 + 轻微锐化」。
///
/// 刻意不做的事（都会让截图变糊或变形）：
///   * 中值滤波降噪 —— 截图没有胶片颗粒，只会糊掉文字边缘
///   * 自动裁边     —— 会把窗口边框当扫描白边裁掉
///   * 自动纠偏     —— 会把表格/文字块判成倾斜转歪
///   * CLAHE 强拉   —— 会让界面底色出现块状色块
fn enhance_screen(img: &RgbImage, o: &Options) -> RgbImage {
    let mut out = img.clone();
    if o.whitebalance {
        out = white_patch(&out, 99.0, 0.55);
    }
    if o.levels {
        out = auto_levels(&out, 0.05, false);
    }
    if o.sharpen {
        out = unsharp(&out, 0.8, 55.0, 3.0);
    }
    out
}

/// 灰度图的噪声估计（与 imaging 一致：缩到 640 宽后统计）
fn noise_of(img: &RgbImage) -> f64 {
    let small = thumbnail(img, 640);
    let g = gray_plane(&small);
    crate::imaging::noise_estimate_pub(&g.data, g.w, g.h)
}

/// 灰度标准差 / 255
fn std_of(img: &RgbImage) -> f32 {
    let small = thumbnail(img, 640);
    let g = gray_plane(&small);
    let n = g.data.len().max(1) as f32;
    let mean = g.data.iter().sum::<f32>() / n;
    let var = g.data.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / n;
    var.sqrt() / 255.0
}

/// 按需去噪：只用刚好够的滤波强度把噪声压到预算内。
fn denoise_to_budget(
    img: &RgbImage,
    budget: f64,
    max_iter: usize,
    keep_contrast: Option<f32>,
) -> RgbImage {
    let cur = img.clone();
    let now = noise_of(&cur);
    if now <= budget {
        return cur;
    }
    let mut cur = cur;
    let mut now = now;
    // 保住原始对比度：低于这个值就说明糊过头了
    let lo = keep_contrast.unwrap_or_else(|| std_of(&cur) * 0.80);

    for it in 0..max_iter {
        if now <= budget {
            break;
        }
        let over = (now - budget) / budget.max(0.8);
        let size = if it < 2 || over < 1.2 { 3 } else { 5 };
        let ref_img = median_filter(&cur, size);
        let k = (0.30 + over as f32 * 0.12).min(0.80);
        let a = to_planes(&cur);
        let b = to_planes(&ref_img);
        let mut mixed = [
            Plane::new(a[0].w, a[0].h),
            Plane::new(a[1].w, a[1].h),
            Plane::new(a[2].w, a[2].h),
        ];
        for c in 0..3 {
            for i in 0..a[c].data.len() {
                mixed[c].data[i] = a[c].data[i] * (1.0 - k) + b[c].data[i] * k;
            }
        }
        let trial = from_planes(&mixed);
        let t_noise = noise_of(&trial);
        let t_con = std_of(&trial);
        // 噪声远超预算时优先压噪；否则守住对比度
        if t_noise > budget * 1.5 {
            cur = trial;
            now = t_noise;
            continue;
        }
        if t_con < lo && t_noise > budget {
            break; // 再糊就损失细节了，收手
        }
        cur = trial;
        now = t_noise;
    }
    cur
}

/// 一键老照片增强。
pub fn auto_enhance(
    img: &RgbImage,
    metrics: Option<&crate::imaging::Metrics>,
    opts: &Options,
) -> RgbImage {
    let noise0 = metrics.map(|m| m.noise).unwrap_or(0.0);
    let sepia0 = metrics.map(|m| m.sepia).unwrap_or(0.0);
    let sat0 = metrics.map(|m| m.saturation).unwrap_or(0.30);
    let blur0 = metrics.map(|m| m.blur);

    // 截图/文档走「保清晰」专用流程
    if is_screen_or_document(img, metrics) {
        return enhance_screen(img, opts);
    }

    let mut out = img.clone();
    if opts.crop {
        out = crop_border(&out, 10.0, 0.22);
    }
    if opts.deskew {
        out = deskew(&out, None, 12.0, 0.9, 1.06);
    }
    if opts.whitebalance {
        out = neutral_cast(&out);
    }
    if opts.deyellow {
        out = deyellow(&out, if sepia0 > 0.40 { 1.0 } else { 0.5 });
    }
    if opts.levels {
        out = auto_levels(&out, 0.35, true);
    }

    let budget = (noise0 * 1.35 + 1.2).max(3.2);
    if opts.denoise {
        out = denoise_to_budget(&out, budget, 6, None);
    }
    if opts.despeckle {
        // 阈值按实测标定：划痕响应通常 12-60，噪点 <10
        let thr = (11.0 + noise0 as f32 * 2.2).min(30.0);
        out = despeckle(&out, thr, 1, 0.10);
    }

    // 局部对比增强：只对发淡图启用，强度随噪点下降
    let want_clahe = match opts.clahe {
        ClaheMode::On => true,
        ClaheMode::Off => false,
        ClaheMode::Auto => std_of(&out) < 0.150,
    };
    if want_clahe {
        let n_before = noise_of(&out);
        let mut clip = 1.5f32;
        if n_before > 4.0 {
            clip *= 0.70;
        } else if n_before > 2.5 {
            clip *= 0.85;
        }
        let cand = clahe(&out, clip, 8);
        let n_after = noise_of(&cand);
        // 双重上限：相对当前不超过 25%，相对原图不超过 3 倍，绝对不超过 3.6
        let cap = (n_before * 1.25)
            .max(n_before + 0.8)
            .min((noise0 * 3.0).max(2.6))
            .min(3.6);
        if std_of(&cand) > std_of(&out) && n_after <= cap {
            out = cand;
        }
        if opts.denoise {
            out = denoise_to_budget(&out, budget, 6, None);
        }
    }

    if opts.fade && sat0 < 0.18 {
        out = saturate_by_fade(&out, 0.20);
    }

    // 锐化：强度与原始噪点负相关
    if opts.sharpen {
        let (mut amount, mut radius) = (1.10f32, 1.6f32);
        if let Some(b) = blur0 {
            if b < 40.0 {
                amount = 1.80;
                radius = 2.1;
            } else if b < 90.0 {
                amount = 1.50;
                radius = 1.9;
            } else if b < 200.0 {
                amount = 1.20;
                radius = 1.6;
            } else {
                amount = 0.90;
                radius = 1.3;
            }
        }
        let n_now = noise_of(&out);
        if n_now > 6.0 {
            amount *= 0.50;
        } else if n_now > 4.0 {
            amount *= 0.70;
        } else if n_now > 2.5 {
            amount *= 0.88;
        }
        out = edge_sharpen(&out, amount, radius, 8.0);
    }

    // 终检：锐化会抬高噪声，统一压回可接受上限
    if opts.denoise {
        let n_end = noise_of(&out);
        let cap = ((noise0 * 1.5 + 1.2).min(n_end * 0.85).min(6.5)).max(3.2);
        if n_end > cap {
            out = denoise_to_budget(&out, cap, 4, None);
        }
    }

    if opts.upscale {
        out = upscale(&out, 2.0, 8000);
    }
    out
}

// ---------------------------------------------------------------- 入口

/// 读图并修复，返回结果图与前后指标。
pub struct EnhanceResult {
    pub image: RgbImage,
    pub before: crate::imaging::Metrics,
    pub after: crate::imaging::Metrics,
    pub screen_like: bool,
}

/// 对文件执行修复。
pub fn enhance_file(path: &std::path::Path, opts: &Options) -> Option<EnhanceResult> {
    let img = open_image(path)?;
    let before = crate::imaging::quality_metrics(&img, 640);
    let screen_like = is_screen_or_document(&img, Some(&before));
    let out = auto_enhance(&img, Some(&before), opts);
    let after = crate::imaging::quality_metrics(&out, 640);
    Some(EnhanceResult {
        image: out,
        before,
        after,
        screen_like,
    })
}

/// 保存为 JPEG。
pub fn save_jpeg(img: &RgbImage, out_path: &std::path::Path, quality: u8) -> bool {
    if let Some(parent) = out_path.parent() {
        crate::config::ensure_dir(parent.to_path_buf());
    }
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    if enc.encode_image(img).is_err() {
        return false;
    }
    std::fs::write(out_path, buf).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::imaging::{open_image, quality_metrics};
    use std::path::PathBuf;

    fn sample(name: &str) -> Option<PathBuf> {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../samples")
            .join(name);
        p.exists().then_some(p)
    }

    fn solid(w: u32, h: u32, c: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, image::Rgb(c))
    }

    // ---------------- 各子算法独立可用 ----------------

    #[test]
    fn auto_levels_stretches_contrast() {
        // 一张对比度很低的图，拉伸后应显著提高
        let (w, h) = (200u32, 200u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = 100 + ((x + y) % 30) as u8;
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        let out = auto_levels(&img, 0.4, true);
        let before = quality_metrics(&img, 640).contrast;
        let after = quality_metrics(&out, 640).contrast;
        assert!(after > before * 1.5, "对比度应提升: {before} -> {after}");
    }

    #[test]
    fn gray_world_neutralizes_cast() {
        // 整体偏蓝的图，灰世界后三通道应更接近
        let mut img = solid(120, 120, [80, 90, 160]);
        let out = gray_world(&img, 0.85);
        let means = channel_means(&out);
        let spread = means.iter().cloned().fold(f32::MIN, f32::max)
            - means.iter().cloned().fold(f32::MAX, f32::min);
        let before = channel_means(&img);
        let bspread = before.iter().cloned().fold(f32::MIN, f32::max)
            - before.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread < bspread, "偏色应被中和: {bspread} -> {spread}");
        let _ = &mut img;
    }

    #[test]
    fn deyellow_reduces_sepia() {
        let Some(p) = sample("1985-06-12 春节全家福.jpg") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let before = quality_metrics(&img, 640).sepia;
        let out = deyellow(&img, 1.0);
        let after = quality_metrics(&out, 640).sepia;
        assert!(after < before, "泛黄应下降: {before} -> {after}");
    }

    #[test]
    fn denoise_reduces_noise() {
        let Some(p) = sample("模糊的老照片 1978.jpg") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let before = quality_metrics(&img, 640).noise;
        let out = denoise(&img, 2);
        let after = quality_metrics(&out, 640).noise;
        assert!(after < before, "噪点应下降: {before} -> {after}");
    }

    #[test]
    fn clahe_raises_contrast_on_flat_image() {
        let (w, h) = (300u32, 300u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = 110 + ((x * 7 + y * 3) % 25) as u8;
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        let before = quality_metrics(&img, 640).contrast;
        let out = clahe(&img, 2.0, 8);
        let after = quality_metrics(&out, 640).contrast;
        assert!(after > before, "CLAHE 应提升对比度: {before} -> {after}");
    }

    #[test]
    fn unsharp_increases_detail() {
        let Some(p) = sample("DSC_0071.JPG") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let before = quality_metrics(&img, 640).blur;
        let out = unsharp(&img, 1.6, 140.0, 3.0);
        let after = quality_metrics(&out, 640).blur;
        assert!(after > before, "锐化应提升拉普拉斯方差: {before} -> {after}");
    }

    #[test]
    fn crop_border_trims_scanned_margin() {
        // 造一张白边 + 居中内容的图
        let mut img = solid(400, 400, [250, 250, 250]);
        for y in 60..340 {
            for x in 60..340 {
                img.put_pixel(x, y, image::Rgb([60, 60, 60]));
            }
        }
        let out = crop_border(&img, 10.0, 0.22);
        assert!(out.width() < 400, "应裁掉白边，实际 {}", out.width());
        assert!(out.width() > 200, "不该裁掉太多，实际 {}", out.width());
    }

    #[test]
    fn crop_border_leaves_full_bleed_image_alone() {
        // 整张都是内容时不该裁
        let Some(p) = sample("DSC_0071.JPG") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let out = crop_border(&img, 10.0, 0.22);
        // 允许有极小的裁切，但不能明显缩小
        assert!(
            out.width() as f32 > img.width() as f32 * 0.75,
            "{} -> {}",
            img.width(),
            out.width()
        );
    }

    #[test]
    fn despeckle_removes_thin_scratches() {
        // 干净底图 + 一条 1px 亮线
        let mut img = solid(200, 200, [120, 120, 120]);
        for y in 40..160 {
            img.put_pixel(100, y, image::Rgb([235, 235, 235]));
        }
        let out = despeckle(&img, 12.0, 1, 0.10);
        // 划痕位置应被修补得接近背景
        let px = out.get_pixel(100, 100).0[0];
        assert!(px < 200, "划痕应被压掉，实际像素 {px}");
    }

    #[test]
    fn despeckle_gives_up_on_too_many_hits() {
        // 整张高频噪点：命中率过高时应原样返回，避免糊图
        let (w, h) = (120u32, 120u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 20 } else { 230 };
                img.put_pixel(x, y, image::Rgb([v, v, v]));
            }
        }
        let out = despeckle(&img, 5.0, 1, 0.10);
        assert_eq!(out.dimensions(), img.dimensions());
    }

    #[test]
    fn fix_red_eye_reduces_red() {
        let mut img = solid(100, 100, [30, 30, 30]);
        for y in 40..60 {
            for x in 40..60 {
                img.put_pixel(x, y, image::Rgb([220, 40, 40]));
            }
        }
        let out = fix_red_eye(&img, 1.55);
        let px = out.get_pixel(50, 50).0;
        assert!(px[0] < 220, "红通道应被压低，实际 {}", px[0]);
    }

    #[test]
    fn upscale_increases_size() {
        let img = solid(100, 80, [120, 120, 120]);
        let out = upscale(&img, 2.0, 8000);
        assert_eq!(out.dimensions(), (200, 160));
    }

    #[test]
    fn upscale_respects_max_side() {
        let img = solid(1000, 500, [120, 120, 120]);
        let out = upscale(&img, 4.0, 1200);
        assert!(out.width() <= 1200, "{}", out.width());
    }

    #[test]
    fn detect_skew_finds_rotation() {
        let img = solid(300, 300, [250, 250, 250]);
        // 画一组水平线，再旋转 3 度
        let mut striped = img.clone();
        for i in 0..10 {
            let y = 30 + i * 25;
            for x in 20..280 {
                striped.put_pixel(x, y, image::Rgb([20, 20, 20]));
            }
        }
        let rotated = rotate_rgb_expand(&striped, 3.0);
        let detected = detect_skew(&rotated, 12.0, 0.5);
        // 检测出的角度应能把图转回去，符号相反且量级接近
        assert!(
            (detected + 3.0).abs() < 2.0,
            "应检测到约 -3 度，实际 {detected}"
        );
    }

    #[test]
    fn deskew_skips_screen_like() {
        // 文档类图（大面积白）不该被旋转
        let mut img = solid(400, 400, [250, 250, 250]);
        for y in 100..300 {
            for x in 100..300 {
                img.put_pixel(x, y, image::Rgb([60, 60, 60]));
            }
        }
        let out = deskew(&img, None, 12.0, 0.9, 1.06);
        assert_eq!(out.dimensions(), img.dimensions(), "截图/文档不该被旋转");
    }

    // ---------------- 预设 ----------------

    #[test]
    fn all_presets_produce_valid_images() {
        let Some(p) = sample("1985-06-12 春节全家福.jpg") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let m = quality_metrics(&img, 640);
        for name in preset_names() {
            let opts = preset_options(name);
            let out = auto_enhance(&img, Some(&m), &opts);
            assert!(out.width() > 0 && out.height() > 0, "预设 {name} 输出为空");
        }
    }

    #[test]
    fn preset_expands_to_full_options() {
        let o = preset_options("仅去噪");
        assert!(!o.crop);
        assert!(!o.deskew);
        assert!(!o.whitebalance);
        assert!(!o.sharpen);
        assert!(!o.levels);
        // denoise 保持默认开启
        assert!(o.denoise);

        let o2 = preset_options("标准修复");
        assert!(o2.crop && o2.deskew && o2.denoise && o2.sharpen);
        assert_eq!(o2.clahe, ClaheMode::Auto);
    }

    #[test]
    fn preset_options_yaml_roundtrip() {
        for name in preset_names() {
            let o = preset_options(name);
            let j = o.to_json();
            let mut o2 = Options::default();
            if let Some(map) = j.as_object() {
                for (k, v) in map {
                    o2.set(k, v);
                }
            }
            assert_eq!(o2.crop, o.crop, "{name} crop");
            assert_eq!(o2.deskew, o.deskew, "{name} deskew");
            assert_eq!(o2.clahe, o.clahe, "{name} clahe");
        }
    }

    // ---------------- 完整流程验收（对齐 test_all.py 门槛） ----------------

    /// test_all.py 的验收：对比度 >=1.20x、泛黄 <=0.5x、噪点受控、细节 >=0.80x
    fn check_enhance(name: &str, min_contrast_gain: f64) {
        let Some(p) = sample(name) else {
            return;
        };
        let img = open_image(&p).unwrap();
        let m = quality_metrics(&img, 640);
        let opts = preset_options("标准修复");
        let out = auto_enhance(&img, Some(&m), &opts);
        let o = quality_metrics(&out, 640);

        assert!(
            o.contrast >= m.contrast * min_contrast_gain,
            "{name}: 对比度提升不足 {:.2}x（{:.4} -> {:.4}）",
            o.contrast / m.contrast.max(1e-6),
            m.contrast,
            o.contrast
        );
        assert!(
            o.noise <= (m.noise * 1.60).max(m.noise + 2.5),
            "{name}: 噪点失控 {:.2} -> {:.2}",
            m.noise,
            o.noise
        );
        assert!(
            o.blur >= m.blur * 0.80,
            "{name}: 细节损失过大 {:.1} -> {:.1}",
            m.blur,
            o.blur
        );
    }

    #[test]
    fn enhance_family_portrait_meets_bar() {
        check_enhance("1985-06-12 春节全家福.jpg", 1.20);
    }

    #[test]
    fn enhance_yard_group_meets_bar() {
        check_enhance("1985年06月12日 老家院子合影.jpg", 1.10);
    }

    #[test]
    fn enhance_travel_meets_bar() {
        check_enhance("1992-10-01 旅行.jpg", 1.15);
    }

    #[test]
    fn enhance_document_scan_meets_bar() {
        check_enhance("扫描件_2008-07-20_证件照.jpg", 1.10);
    }

    #[test]
    fn enhance_wechat_photo_meets_bar() {
        // 这张本身很平，提升空间最大
        check_enhance("微信图片_20200101.jpg", 1.30);
    }

    #[test]
    fn enhance_dsc_meets_bar() {
        check_enhance("DSC_0071.JPG", 1.10);
    }

    #[test]
    fn enhance_broken_file_returns_none() {
        let p = std::env::temp_dir().join("po_restore_broken.jpg");
        std::fs::write(&p, b"\xff\xd8\xff\xe0 nope").unwrap();
        assert!(enhance_file(&p, &Options::default()).is_none());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn enhance_tiny_image_does_not_panic() {
        let img = solid(4, 3, [120, 100, 90]);
        let m = quality_metrics(&img, 640);
        let out = auto_enhance(&img, Some(&m), &Options::default());
        assert!(out.width() > 0);
    }

    #[test]
    fn screen_like_detects_white_document() {
        // 大面积纯白 + 少量深色文字：典型文档扫描
        let mut img = solid(400, 400, [250, 250, 250]);
        for y in 60..340 {
            for x in 60..340 {
                img.put_pixel(x, y, image::Rgb([55, 55, 55]));
            }
        }
        assert!(is_screen_or_document(&img, None));
    }

    #[test]
    fn screen_like_detects_dark_ide() {
        // 深色界面：大面积近黑，少量高饱和色块
        let mut img = solid(400, 400, [32, 32, 36]);
        for x in 40..200 {
            for y in 40..60 {
                img.put_pixel(x, y, image::Rgb([230, 70, 70]));
            }
        }
        assert!(is_screen_or_document(&img, None));
    }

    #[test]
    fn screen_like_does_not_catch_photo_with_bright_sky() {
        // 有大量亮天空的真实照片不该被判成截图
        let (w, h) = (400u32, 300u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                // 天空：从浅蓝渐变到米白；下半部分地面
                if y < h / 2 {
                    let t = y * 2 * 200 / h;
                    img.put_pixel(x, y, image::Rgb([150 + t as u8 / 2, 175 + t as u8 / 3, 210]));
                } else {
                    let v = 90 + ((x * 7 + y * 3) % 60) as u8;
                    img.put_pixel(x, y, image::Rgb([v, v.saturating_sub(10), v.saturating_sub(30)]));
                }
            }
        }
        assert!(!is_screen_or_document(&img, None), "亮天空的照片不该判成截图");
    }

    #[test]
    fn screen_like_does_not_catch_synthetic_photos() {
        // 用真实合成样图批量验证：一张都不该被判成截图
        let names = [
            "1985-06-12 春节全家福.jpg",
            "1985年06月12日 老家院子合影.jpg",
            "1992-10-01 旅行.jpg",
            "20050315_141530.jpg",
            "DSC_0071.JPG",
            "IMG_20050315_毕业典礼.jpg",
            "微信图片_20200101.jpg",
            "扫描件_2008-07-20_证件照.jpg",
            "模糊的老照片 1978.jpg",
        ];
        for n in names {
            let Some(p) = sample(n) else { continue };
            let Some(img) = open_image(&p) else { continue };
            let m = quality_metrics(&img, 640);
            assert!(
                !is_screen_or_document(&img, Some(&m)),
                "{n} 是照片，不该被判成截图"
            );
        }
    }

    #[test]
    fn enhance_is_deterministic() {
        let Some(p) = sample("DSC_0071.JPG") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let m = quality_metrics(&img, 640);
        let opts = preset_options("标准修复");
        let a = auto_enhance(&img, Some(&m), &opts);
        let b = auto_enhance(&img, Some(&m), &opts);
        assert_eq!(a.dimensions(), b.dimensions());
        assert_eq!(a.as_raw(), b.as_raw(), "同样输入应得到同样输出");
    }

    fn channel_means(img: &RgbImage) -> [f32; 3] {
        let mut s = [0f64; 3];
        for p in img.pixels() {
            for c in 0..3 {
                s[c] += p.0[c] as f64;
            }
        }
        let n = img.pixels().len().max(1) as f64;
        [
            (s[0] / n) as f32,
            (s[1] / n) as f32,
            (s[2] / n) as f32,
        ]
    }

    #[test]
    fn options_set_parses_json_types() {
        let mut o = Options::default();
        o.set("crop", &serde_json::json!(false));
        assert!(!o.crop);
        o.set("crop", &serde_json::json!(1));
        assert!(o.crop);
        o.set("clahe", &serde_json::json!("auto"));
        assert_eq!(o.clahe, ClaheMode::Auto);
        o.set("clahe", &serde_json::json!(true));
        assert_eq!(o.clahe, ClaheMode::On);
        o.set("clahe", &serde_json::json!(false));
        assert_eq!(o.clahe, ClaheMode::Off);
        // 未知键应被忽略而不 panic
        o.set("no_such_key", &serde_json::json!(true));
    }
}
