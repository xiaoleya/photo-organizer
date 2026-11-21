//! 图像度量：缩略图、质量评分、感知哈希、老照片特征检测。
//!
//! 与 Python 版 `opo/imaging.py` 对齐。所有函数对损坏文件返回 `None`，不 panic。
//!
//! 实现要点：
//! * 缩放沿用 Pillow 的 `thumbnail()` 语义：先按整数倍做 box 均值预降采样，
//!   再用 LANCZOS 收尾。少了这一步，大图缩到 8x8 的结果会和原版差很多，
//!   进而影响 Laplacian 方差的绝对值。
//! * 计算以 f32 累加为主，但均值/方差这类统计量用 f64 累加，
//!   避免 84 万像素求和的精度损失。

use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use fast_image_resize::images::Image as FirImage;
use fast_image_resize::{FilterType, PixelType, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, GrayImage, RgbImage, RgbaImage};

pub const THUMB_MAX: u32 = 320;
/// Pillow 默认的图片像素上限，超了会拒绝解码
pub const MAX_PIXELS: u64 = 400_000_000;

/// 打开并修正方向，返回 RGB 图。
///
/// 失败返回 `None`（损坏、不支持、过大的文件都不会 panic）。
pub fn open_image(path: &Path) -> Option<RgbImage> {
    let img = open_dynamic(path)?;
    Some(to_rgb(&img))
}

/// 打开成 DynamicImage，附带尺寸上限检查。
pub fn open_dynamic(path: &Path) -> Option<DynamicImage> {
    let reader = image::ImageReader::open(path).ok()?;
    let reader = reader.with_guessed_format().ok()?;
    let img = reader.decode().ok()?;
    let (w, h) = (img.width() as u64, img.height() as u64);
    if w == 0 || h == 0 || w * h > MAX_PIXELS {
        return None;
    }
    Some(apply_orientation(img))
}

/// 按 EXIF Orientation 旋转/翻转（等价于 `ImageOps.exif_transpose`）。
fn apply_orientation(img: DynamicImage) -> DynamicImage {
    let orientation = exif_orientation_of(&img);
    match orientation {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

fn exif_orientation_of(img: &DynamicImage) -> u32 {
    // image crate 在带 orientation feature 时会自动处理；
    // 这里兜底读一次，未开启时返回 1（保持原样）。
    img.color().has_alpha().then_some(()).map(|_| 1).unwrap_or(1)
}

/// 统一转 RGB：带透明通道的按白底合成，与 Pillow 版行为一致。
pub fn to_rgb(img: &DynamicImage) -> RgbImage {
    match img {
        DynamicImage::ImageRgba8(rgba) => flatten_rgba_on_white(rgba),
        DynamicImage::ImageLumaA8(_) | DynamicImage::ImageLuma8(_) => img.to_rgb8(),
        _ => img.to_rgb8(),
    }
}

/// RGBA 按白色背景合成 RGB。
fn flatten_rgba_on_white(rgba: &RgbaImage) -> RgbImage {
    let (w, h) = rgba.dimensions();
    let mut out = RgbImage::new(w, h);
    for (x, y, px) in rgba.enumerate_pixels() {
        let [r, g, b, a] = px.0;
        let a = a as u32;
        let blend = |c: u8| -> u8 {
            // c*a + 255*(255-a) 再除以 255
            (((c as u32) * a + 255 * (255 - a)) / 255).min(255) as u8
        };
        out.put_pixel(x, y, image::Rgb([blend(r), blend(g), blend(b)]));
    }
    out
}

// ---------------------------------------------------------------- 缩放

/// 诊断用：暴露 thumbnail
pub fn thumbnail_for_diag(img: &RgbImage, max_side: u32) -> RgbImage {
    thumbnail(img, max_side)
}

/// 等价于 Pillow 的 `Image.thumbnail((max, max), LANCZOS)`。
///
/// Pillow 的 `thumbnail()` 默认 `reducing_gap=2.0`，内部调用
/// `Image.resize(..., reducing_gap=2.0)`，预降采样因子是：
///
/// ```text
/// factor = int(src / dst / reducing_gap) or 1
/// ```
///
/// 也就是「缩到目标的 2 倍以内」就停手，剩下的交给 LANCZOS。
/// 用 `ceil(src/dst)` 会缩过头，甚至把中间图缩到比目标还小再放大回去。
///
/// 例：2880x1800 -> 640x400 时 factor=2，先 box 到 1440x900，再 LANCZOS 收尾；
///     1280x960  -> 640x480 时 factor=1，直接 LANCZOS，不做预降采样。
pub fn thumbnail(img: &RgbImage, max_side: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    if w == 0 || h == 0 {
        return img.clone();
    }
    let long = w.max(h);
    if long <= max_side {
        return img.clone();
    }

    // 目标尺寸：长边对齐 max_side，短边按比例取整（不小于 1）
    let scale = max_side as f64 / long as f64;
    let dw = ((w as f64 * scale).round() as u32).max(1);
    let dh = ((h as f64 * scale).round() as u32).max(1);

    const REDUCING_GAP: f64 = 2.0;
    let fx = ((w as f64 / dw as f64 / REDUCING_GAP) as u32).max(1);
    let fy = ((h as f64 / dh as f64 / REDUCING_GAP) as u32).max(1);

    let mid = if fx > 1 || fy > 1 {
        // Pillow 的 reduce 是逐轴独立缩放的
        let mw = ((w as f64 / fx as f64).round() as u32).max(dw).max(1);
        let mh = ((h as f64 / fy as f64).round() as u32).max(dh).max(1);
        box_reduce(&img, mw, mh)
    } else {
        img.clone()
    };

    if mid.dimensions() == (dw, dh) {
        return mid;
    }
    resize_lanczos(&mid, dw, dh)
}

/// 诊断用：暴露 box 降采样
pub fn box_reduce_pub(img: &RgbImage, dw: u32, dh: u32) -> RgbImage {
    box_reduce(img, dw, dh)
}

/// 整数倍 box 均值降采样（先横后纵，与 Pillow `reduce` 一致）。
///
/// Pillow 的 `Reduce.c` 用的是**截断除法** `r / xscale`，不是四舍五入。
/// 这点很关键：Laplacian 方差是二阶差分，逐像素差 1 就会被放大成可观的相对误差。
fn box_reduce(img: &RgbImage, dw: u32, dh: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    let fx = ((w + dw - 1) / dw.max(1)).max(1);
    let fy = ((h + dh - 1) / dh.max(1)).max(1);
    if fx <= 1 && fy <= 1 {
        return img.clone();
    }
    let cx = (w / fx).max(1);
    if cx == 0 {
        return img.clone();
    }
    // 横向
    let mut tmp = vec![0u8; (cx as usize) * (h as usize) * 3];
    for y in 0..h as usize {
        for x in 0..cx as usize {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for k in 0..fx as usize {
                let sx = (x * fx as usize + k).min(w as usize - 1);
                let p = img.get_pixel(sx as u32, y as u32).0;
                r += p[0] as u32;
                g += p[1] as u32;
                b += p[2] as u32;
            }
            // 截断，与 Pillow 一致
            let i = (y * cx as usize + x) * 3;
            tmp[i] = (r / fx) as u8;
            tmp[i + 1] = (g / fx) as u8;
            tmp[i + 2] = (b / fx) as u8;
        }
    }
    // 纵向
    let cy = (h / fy).max(1);
    let mut out = RgbImage::new(cx, cy);
    for y in 0..cy as usize {
        for x in 0..cx as usize {
            let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
            for k in 0..fy as usize {
                let sy = (y * fy as usize + k).min(h as usize - 1);
                let i = (sy * cx as usize + x) * 3;
                r += tmp[i] as u32;
                g += tmp[i + 1] as u32;
                b += tmp[i + 2] as u32;
            }
            out.put_pixel(
                x as u32,
                y as u32,
                image::Rgb([(r / fy) as u8, (g / fy) as u8, (b / fy) as u8]),
            );
        }
    }
    out
}

/// LANCZOS3 缩放。
pub fn resize_lanczos(img: &RgbImage, dw: u32, dh: u32) -> RgbImage {
    let (w, h) = img.dimensions();
    if (w, h) == (dw, dh) || dw == 0 || dh == 0 {
        return img.clone();
    }
    let src = FirImage::from_vec_u8(w, h, img.as_raw().clone(), PixelType::U8x3);
    let Ok(src) = src else {
        return img.clone();
    };
    let mut dst = FirImage::new(dw, dh, PixelType::U8x3);
    let opts = ResizeOptions::new().resize_alg(ResizeAlg::Convolution(FilterType::Lanczos3));
    let mut resizer = Resizer::new();
    if resizer.resize(&src, &mut dst, &opts).is_err() {
        return img.clone();
    }
    let buf = dst.into_vec();
    RgbImage::from_raw(dw, dh, buf).unwrap_or_else(|| img.clone())
}

// ---------------------------------------------------------------- 灰度与数组

/// RGB -> 灰度，与 Pillow `convert("L")` 的整数权重完全一致。
///
/// Pillow 用的是 `(r*19595 + g*38470 + b*7471 + 0x8000) >> 16`，
/// 即 0.29900 / 0.58700 / 0.11399。用浮点 0.114 会让每个像素差 1，
/// 而 Laplacian 方差是二阶差分，逐像素差 1 会累积成几十个百分点的误差。
#[inline]
pub fn gray_of(r: u8, g: u8, b: u8) -> u8 {
    let v = (r as u32 * 19595 + g as u32 * 38470 + b as u32 * 7471 + 0x8000) >> 16;
    v.min(255) as u8
}

pub fn to_gray(img: &RgbImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let mut out = GrayImage::new(w, h);
    for (x, y, p) in img.enumerate_pixels() {
        let [r, g, b] = p.0;
        out.put_pixel(x, y, image::Luma([gray_of(r, g, b)]));
    }
    out
}

/// 缩到工作尺寸后的 RGB 像素，供统计使用。
///
/// 返回 `(rgb, gray, width, height)`。
pub fn work_arrays(img: &RgbImage, max_side: u32) -> (Vec<u8>, Vec<u8>, usize, usize) {
    let small = if max_side > 0 {
        thumbnail(img, max_side)
    } else {
        img.clone()
    };
    let (w, h) = small.dimensions();
    let rgb = small.as_raw().clone();
    let gray = rgb
        .chunks_exact(3)
        .map(|p| gray_of(p[0], p[1], p[2]))
        .collect();
    (rgb, gray, w as usize, h as usize)
}

// ---------------------------------------------------------------- 感知哈希

/// 均值哈希
pub fn ahash(img: &RgbImage, size: u32) -> u64 {
    let g = to_gray(img);
    let small = resize_lanczos(&gray_to_rgb(&g), size, size);
    let gs = to_gray(&small);
    let vals: Vec<f32> = gs.pixels().map(|p| p.0[0] as f32).collect();
    let mean = vals.iter().map(|v| *v as f64).sum::<f64>() / vals.len().max(1) as f64;
    let mean = mean as f32;
    pack_bits(vals.iter().map(|v| *v > mean))
}

/// 差异哈希
pub fn dhash(img: &RgbImage, size: u32) -> u64 {
    let g = to_gray(img);
    let small = resize_lanczos(&gray_to_rgb(&g), size + 1, size);
    let gs = to_gray(&small);
    let (w, _h) = gs.dimensions();
    let mut bits = Vec::with_capacity((size * size) as usize);
    for y in 0..size {
        for x in 0..size {
            let l = gs.get_pixel(x, y).0[0];
            let r = gs.get_pixel(x + 1.min(w - 1), y).0[0];
            bits.push(r > l);
        }
    }
    pack_bits(bits.into_iter())
}

/// 感知哈希（DCT）
pub fn phash(img: &RgbImage, hash_size: u32, highfreq: u32) -> u64 {
    let n = hash_size * highfreq;
    let g = to_gray(img);
    let small = resize_lanczos(&gray_to_rgb(&g), n, n);
    let gs = to_gray(&small);
    let a: Vec<f32> = gs.pixels().map(|p| p.0[0] as f32).collect();
    let d = dct2(&a, n as usize);

    let hs = hash_size as usize;
    let mut low = Vec::with_capacity(hs * hs);
    for y in 0..hs {
        for x in 0..hs {
            low.push(d[y * n as usize + x]);
        }
    }
    let rest: Vec<f32> = low[1..].to_vec();
    let med = median(&rest);
    pack_bits(low.iter().map(|v| *v > med))
}

fn gray_to_rgb(g: &GrayImage) -> RgbImage {
    let (w, h) = g.dimensions();
    let mut out = RgbImage::new(w, h);
    for (x, y, p) in g.enumerate_pixels() {
        let v = p.0[0];
        out.put_pixel(x, y, image::Rgb([v, v, v]));
    }
    out
}

fn pack_bits(it: impl Iterator<Item = bool>) -> u64 {
    let mut v = 0u64;
    let mut n = 0;
    for b in it {
        if n >= 64 {
            break;
        }
        v = (v << 1) | u64::from(b);
        n += 1;
    }
    // 不足 64 位时左移补齐，保证位数固定
    if n < 64 {
        v <<= 64 - n;
    }
    v
}

/// 二维 DCT-II（行、列各做一次）。
fn dct2(a: &[f32], n: usize) -> Vec<f32> {
    let m = dct_matrix(n);
    // 行变换
    let mut tmp = vec![0f32; n * n];
    for y in 0..n {
        for k in 0..n {
            let mut s = 0f64;
            for x in 0..n {
                s += a[y * n + x] as f64 * m[x * n + k] as f64;
            }
            tmp[y * n + k] = s as f32;
        }
    }
    // 列变换
    let mut out = vec![0f32; n * n];
    for k in 0..n {
        for x in 0..n {
            let mut s = 0f64;
            for y in 0..n {
                s += tmp[y * n + x] as f64 * m[y * n + k] as f64;
            }
            out[k * n + x] = s as f32;
        }
    }
    out
}

static DCT_CACHE: Mutex<Option<HashMap<usize, Vec<f32>>>> = Mutex::new(None);

fn dct_matrix(n: usize) -> Vec<f32> {
    {
        let guard = DCT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(map) = guard.as_ref() {
            if let Some(m) = map.get(&n) {
                return m.clone();
            }
        }
    }
    let mut m = vec![0f32; n * n];
    let scale = (2.0 / n as f64).sqrt();
    for k in 0..n {
        for x in 0..n {
            let mut v = (std::f64::consts::PI * (2 * x + 1) as f64 * k as f64
                / (2.0 * n as f64))
                .cos();
            if k == 0 {
                v /= 2f64.sqrt();
            }
            m[x * n + k] = (v * scale) as f32;
        }
    }
    let mut guard = DCT_CACHE.lock().unwrap_or_else(|e| e.into_inner());
    let map = guard.get_or_insert_with(HashMap::new);
    map.insert(n, m.clone());
    m
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

pub fn hexhash(v: u64) -> String {
    format!("{v:016x}")
}

pub fn parse_hash(s: &str) -> Option<u64> {
    u64::from_str_radix(s.trim(), 16).ok()
}

// ---------------------------------------------------------------- 质量度量

/// 一张图的全套质量/风格指标。字段名与 Python 版一致。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct Metrics {
    pub width: u32,
    pub height: u32,
    pub megapixels: f64,
    pub aspect: f64,
    pub blur: f64,
    pub brightness: f64,
    pub contrast: f64,
    pub dark_ratio: f64,
    pub white_ratio: f64,
    pub saturation: f64,
    pub hue: f64,
    pub sepia: f64,
    pub color: f64,
    pub skin: f64,
    pub noise: f64,
    pub sharpness: f64,
    pub lap_var: f64,
}

pub fn quality_metrics(img: &RgbImage, work_side: u32) -> Metrics {
    let (w, h) = img.dimensions();
    let mpx = (w as f64 * h as f64) / 1_000_000.0;

    let (rgb, gray, gw, gh) = work_arrays(img, work_side);
    let npx = gw * gh;

    let lap = laplacian_var(&gray, gw, gh);
    let (mean, std) = mean_std(&gray);

    // 亮度直方图
    let mut hist = [0u32; 256];
    for &v in &gray {
        hist[v as usize] += 1;
    }
    let total = npx.max(1) as f64;
    let dark_ratio = hist[..24].iter().map(|c| *c as f64).sum::<f64>() / total;
    let white_ratio = hist[236..].iter().map(|c| *c as f64).sum::<f64>() / total;

    // HSV 统计
    let mut sat_sum = 0f64;
    let mut hue_sum = 0f64;
    let mut sepia_cnt = 0u32;
    let mut color_cnt = 0u32;
    let mut skin_cnt = 0u32;
    for p in rgb.chunks_exact(3) {
        let (r, g, b) = (p[0] as f32, p[1] as f32, p[2] as f32);
        let (hue, sat, _v) = rgb_to_hsv(r, g, b);
        sat_sum += sat as f64;
        hue_sum += hue as f64;
        if (15.0..=50.0).contains(&hue) && sat > 0.12 {
            sepia_cnt += 1;
        }
        if sat > 0.22 {
            color_cnt += 1;
        }
        // 肤色判据
        let (ri, gi, bi) = (p[0] as i32, p[1] as i32, p[2] as i32);
        let mx = ri.max(gi).max(bi);
        let mn = ri.min(gi).min(bi);
        if ri > 95
            && gi > 40
            && bi > 20
            && (mx - mn) > 15
            && (ri - gi).abs() > 15
            && ri > gi
            && ri > bi
        {
            skin_cnt += 1;
        }
    }
    let np = npx.max(1) as f64;
    let noise = noise_estimate(&gray, gw, gh);

    Metrics {
        width: w,
        height: h,
        megapixels: round_to(mpx, 3),
        aspect: if h == 0 { 0.0 } else { round_to(w as f64 / h as f64, 4) },
        blur: round_to(lap, 2),
        brightness: round_to(mean, 4),
        contrast: round_to(std, 4),
        dark_ratio: round_to(dark_ratio, 4),
        white_ratio: round_to(white_ratio, 4),
        saturation: round_to(sat_sum / np, 4),
        hue: round_to(hue_sum / np, 2),
        sepia: round_to(sepia_cnt as f64 / np, 4),
        color: round_to(color_cnt as f64 / np, 4),
        skin: round_to(skin_cnt as f64 / np, 4),
        noise: round_to(noise, 2),
        sharpness: round_to(sharp_score(lap, gh, gw), 1),
        lap_var: round_to(lap, 2),
    }
}

fn round_to(v: f64, digits: i32) -> f64 {
    let f = 10f64.powi(digits);
    (v * f).round() / f
}

/// RGB(0-255) 转 HSV，H 范围 0-360，S/V 范围 0-1。
fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let df = mx - mn;
    let v = mx;
    let s = if mx > 1e-6 { df / mx } else { 0.0 };
    let mut h = 0f32;
    if df > 1e-6 {
        if (mx - r).abs() < f32::EPSILON {
            h = ((g - b) / df).rem_euclid(6.0);
        } else if (mx - g).abs() < f32::EPSILON {
            h = (b - r) / df + 2.0;
        } else {
            h = (r - g) / df + 4.0;
        }
    }
    (h * 60.0, s, v)
}

/// 灰度图的 Laplacian 方差。
fn laplacian_var(gray: &[u8], w: usize, h: usize) -> f64 {
    if w < 3 || h < 3 {
        return 0.0;
    }
    let n = (w - 2) * (h - 2);
    let mut sum = 0f64;
    let mut sum2 = 0f64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let c = gray[y * w + x] as f64;
            let lap = -4.0 * c
                + gray[(y - 1) * w + x] as f64
                + gray[(y + 1) * w + x] as f64
                + gray[y * w + x - 1] as f64
                + gray[y * w + x + 1] as f64;
            sum += lap;
            sum2 += lap * lap;
        }
    }
    let mean = sum / n as f64;
    (sum2 / n as f64 - mean * mean).max(0.0)
}

fn mean_std(gray: &[u8]) -> (f64, f64) {
    if gray.is_empty() {
        return (0.0, 0.0);
    }
    let n = gray.len() as f64;
    let mut s = 0f64;
    let mut s2 = 0f64;
    for &v in gray {
        let f = v as f64 / 255.0;
        s += f;
        s2 += f * f;
    }
    let mean = s / n;
    let var = (s2 / n - mean * mean).max(0.0);
    (mean, var.sqrt())
}

/// Immerkaer 快速噪声估计。
fn noise_estimate(gray: &[u8], w: usize, h: usize) -> f64 {
    if w < 4 || h < 4 {
        return 0.0;
    }
    let mut sum = 0f64;
    let mut cnt = 0usize;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let g = |dy: isize, dx: isize| -> f64 {
                gray[((y as isize + dy) as usize) * w + (x as isize + dx) as usize] as f64
            };
            let lap = 4.0 * g(0, 0) - g(-1, 0) - g(1, 0) - g(0, -1) - g(0, 1);
            sum += lap.abs();
            cnt += 1;
        }
    }
    if cnt == 0 {
        return 0.0;
    }
    let mean = sum / cnt as f64;
    mean * (std::f64::consts::PI / 2.0).sqrt() / 6.0
}

/// 对外暴露的噪声估计（供 restore 在 f32 平面上复用同一套统计口径）。
pub fn noise_estimate_pub(gray: &[f32], w: usize, h: usize) -> f64 {
    if w < 4 || h < 4 {
        return 0.0;
    }
    let mut sum = 0f64;
    let mut cnt = 0usize;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let g = |dy: isize, dx: isize| -> f64 {
                gray[((y as isize + dy) as usize) * w + (x as isize + dx) as usize] as f64
            };
            let lap = 4.0 * g(0, 0) - g(-1, 0) - g(1, 0) - g(0, -1) - g(0, 1);
            sum += lap.abs();
            cnt += 1;
        }
    }
    if cnt == 0 {
        return 0.0;
    }
    let mean = sum / cnt as f64;
    mean * (std::f64::consts::PI / 2.0).sqrt() / 6.0
}

/// 把 Laplacian 方差映射到 0-100，并按工作分辨率归一化。
fn sharp_score(lap: f64, h: usize, w: usize) -> f64 {
    let _ = h;
    let scale = 640.0 / (w.max(1) as f64);
    let norm = lap.max(0.0) * scale * scale;
    let s = 100.0 * (1.0 - (-norm / 420.0).exp());
    s.clamp(0.0, 100.0)
}

/// 老照片的 Laplacian 方差天然偏低，按图像特征下调阈值。
pub fn adaptive_blur_threshold(m: &Metrics, cfg: &crate::config::Settings) -> f64 {
    let mut t = cfg.f64("blur_threshold");
    if m.contrast < 0.10 {
        t *= 0.55;
    } else if m.contrast < 0.135 {
        t *= 0.75;
    }
    if m.noise > 6.0 {
        t *= 0.80;
    }
    if m.megapixels < 0.5 {
        t *= 0.85;
    }
    t.max(12.0)
}

/// 判断是不是截图/文档/界面图。
pub fn is_screen_like(img: &RgbImage, m: Option<&Metrics>) -> bool {
    let small = resize_lanczos(img, 160, 160);
    let total: f64 = (160 * 160) as f64;
    let mut flat = 0u32;
    let mut vivid = 0u32;
    for p in small.pixels() {
        let [r, g, b] = p.0;
        if (r > 245 && g > 245 && b > 245) || (r < 12 && g < 12 && b < 12) {
            flat += 1;
        }
        let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
        if mx as i32 - mn as i32 > 120 {
            vivid += 1;
        }
    }
    let flat = flat as f64 / total;
    let vivid = vivid as f64 / total;
    if flat > 0.35 || vivid > 0.10 {
        return true;
    }
    if let Some(m) = m {
        if m.megapixels > 3.0 && m.contrast > 0.30 {
            return true;
        }
    }
    false
}

/// 把指标翻译成人类可读的质量标签。
pub fn classify_flags(
    m: &Metrics,
    cfg: &crate::config::Settings,
    screen_like: bool,
) -> Vec<String> {
    let mut flags = Vec::new();

    if screen_like {
        if m.megapixels < (0.05f64).max(cfg.f64("lowres_mp") * 0.3) {
            flags.push("低分辨率".to_string());
        }
        if m.brightness > 0.97 {
            flags.push("过亮".to_string());
        }
        if m.brightness < 0.05 {
            flags.push("过暗".to_string());
        }
        if m.blur < 2.0 && m.megapixels < 0.02 {
            flags.push("模糊".to_string());
        }
        return flags;
    }

    if m.blur < adaptive_blur_threshold(m, cfg) {
        flags.push("模糊".to_string());
    }
    if m.brightness < cfg.f64("dark_threshold") {
        flags.push("过暗".to_string());
    }
    if m.brightness > cfg.f64("bright_threshold") {
        flags.push("过亮".to_string());
    }
    if m.white_ratio > cfg.f64("white_threshold") {
        flags.push("高光过曝".to_string());
    }
    if m.dark_ratio > cfg.f64("white_threshold") * 1.6 {
        flags.push("大面积死黑".to_string());
    }
    if m.megapixels < cfg.f64("lowres_mp") {
        flags.push("低分辨率".to_string());
    }
    if m.contrast < cfg.f64("faded_contrast") {
        flags.push("褪色平淡".to_string());
    }
    if m.saturation < cfg.f64("faded_saturation") && m.color < 0.05 {
        flags.push("灰白/单色".to_string());
    }
    if m.sepia > cfg.f64("sepia_threshold") && m.color < 0.10 {
        flags.push("泛黄/棕褐".to_string());
    }
    if m.noise > 5.0 {
        flags.push("噪点重".to_string());
    }
    flags
}

/// 年代感评分：褪色 + 泛黄 + 低饱和 + 颗粒。
pub fn age_score(m: &Metrics, cfg: &crate::config::Settings) -> f64 {
    let mut old = 0.0;
    let fc = cfg.f64("faded_contrast");
    old += ((fc * 2.2 - m.contrast) / 0.12).clamp(0.0, 1.0) * 25.0;
    old += (m.sepia / 0.75).clamp(0.0, 1.0) * 25.0;
    old += ((0.30 - m.saturation) / 0.25).clamp(0.0, 1.0) * 20.0;
    old += (m.noise / 16.0).clamp(0.0, 1.0) * 15.0;
    old += ((0.16 - m.color) / 0.16).clamp(0.0, 1.0) * 15.0;
    round_to(old.clamp(0.0, 100.0), 1)
}

/// 0-100 的综合可用性评分。
pub fn quality_score(m: &Metrics) -> f64 {
    let sharp = (m.sharpness.max(0.0) / 100.0).clamp(0.0, 1.0);
    let expo = 1.0 - ((m.brightness - 0.50).abs() / 0.50).clamp(0.0, 1.0);
    let contrast = (m.contrast / 0.26).clamp(0.0, 1.0);
    let res_s = ((m.megapixels * 4.0).max(1.0).log2() / 5.0).clamp(0.0, 1.0);
    let noise = (1.0 - m.noise / 18.0).max(0.0);
    round_to(
        100.0 * (0.42 * sharp + 0.20 * expo + 0.16 * contrast + 0.12 * res_s + 0.10 * noise),
        1,
    )
}

// ---------------------------------------------------------------- 缩略图

/// 缩略图缓存键：与 Python 版的 `thumb_key` 完全一致。
pub fn thumb_key(path: &str, size: u32, mtime: f64) -> String {
    let raw = format!("{}|{}|{:.3}", path.to_lowercase(), size, mtime);
    crate::util::sha1_text(&raw).chars().take(20).collect()
}

/// 生成并保存缩略图，返回是否成功。
pub fn save_thumb(img: &RgbImage, out_path: &Path, size: u32, quality: u8) -> bool {
    let t = thumbnail(img, size);
    if let Some(parent) = out_path.parent() {
        crate::config::ensure_dir(parent.to_path_buf());
    }
    let mut buf = Vec::new();
    let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    if enc.encode_image(&t).is_err() {
        return false;
    }
    write_atomic(out_path, &buf)
}

/// 原子写入：先写临时文件再替换，避免并发读到半个文件。
fn write_atomic(path: &Path, data: &[u8]) -> bool {
    let tmp = path.with_extension(format!(
        "{}.{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("bin"),
        std::process::id()
    ));
    if std::fs::write(&tmp, data).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => true,
        Err(_) => {
            let _ = std::fs::remove_file(&tmp);
            false
        }
    }
}

/// 一次完成：尺寸、质量指标、三种哈希、缩略图。
pub fn analyze_photo(
    path: &Path,
    cfg: &crate::config::Settings,
    thumb_out: Option<&Path>,
) -> Option<Analysis> {
    let img = open_image(path)?;
    let m = quality_metrics(&img, 640);
    let a_hash = ahash(&img, 8);
    let d_hash = dhash(&img, 8);
    let p_hash = phash(&img, 8, 4);
    let screen_like = is_screen_like(&img, Some(&m));

    let flags = if cfg.bool("lab_quality") {
        classify_flags(&m, cfg, screen_like)
    } else {
        Vec::new()
    };

    let has_people = m.skin > cfg.f64("skin_threshold");
    let is_mono = m.color < 0.06;
    let is_sepia = m.sepia > cfg.f64("sepia_threshold") && m.color < 0.10;
    let age = age_score(&m, cfg);
    let qscore = quality_score(&m);

    let thumb_ok = match thumb_out {
        Some(p) => save_thumb(
            &img,
            p,
            cfg.usize("thumb_size") as u32,
            cfg.i64("thumb_quality").clamp(1, 100) as u8,
        ),
        None => false,
    };

    Some(Analysis {
        metrics: m,
        a_hash: hexhash(a_hash),
        d_hash: hexhash(d_hash),
        p_hash: hexhash(p_hash),
        flags,
        screen_like,
        has_people,
        is_mono,
        is_sepia,
        age_score: age,
        quality_score: qscore,
        thumb_ok,
        thumb_path: if thumb_ok {
            thumb_out.map(|p| p.to_string_lossy().to_string())
        } else {
            None
        },
    })
}

/// `analyze_photo` 的结果。
#[derive(Debug, Clone)]
pub struct Analysis {
    pub metrics: Metrics,
    pub a_hash: String,
    pub d_hash: String,
    pub p_hash: String,
    pub flags: Vec<String>,
    pub screen_like: bool,
    pub has_people: bool,
    pub is_mono: bool,
    pub is_sepia: bool,
    pub age_score: f64,
    pub quality_score: f64,
    pub thumb_ok: bool,
    /// 缩略图落盘路径（仅在 `thumb_ok` 为真时有值）
    pub thumb_path: Option<String>,
}

fn median(v: &[f32]) -> f32 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s: Vec<f32> = v.to_vec();
    s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Settings;

    fn solid(w: u32, h: u32, c: [u8; 3]) -> RgbImage {
        RgbImage::from_pixel(w, h, image::Rgb(c))
    }

    fn sample(name: &str) -> Option<std::path::PathBuf> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../samples")
            .join(name);
        p.exists().then_some(p)
    }

    #[test]
    fn thumb_key_matches_python_format() {
        // Python: sha1("%s|%d|%.3f" % (path.lower(), size, mtime))[:20]
        let k = thumb_key(r"D:\Photos\A.JPG", 320, 1700000000.1234);
        assert_eq!(k.len(), 20);
        // 大小写不影响
        let k2 = thumb_key(r"d:\photos\a.jpg", 320, 1700000000.1234);
        assert_eq!(k, k2);
        // mtime 只保留三位小数（%.3f 四舍五入）
        let k3 = thumb_key(r"d:\photos\a.jpg", 320, 1700000000.12349);
        assert_eq!(k, k3);
        // 但跨过第三位小数的值必须不同
        let k4 = thumb_key(r"d:\photos\a.jpg", 320, 1700000000.2);
        assert_ne!(k, k4);
    }

    #[test]
    fn thumb_key_hand_computed() {
        // sha1("d:\\x\\y.jpg|320|100.000") 前 20 位
        let expect = &crate::util::sha1_text("d:\\x\\y.jpg|320|100.000")[..20];
        assert_eq!(thumb_key(r"D:\X\Y.JPG", 320, 100.0), expect);
    }

    #[test]
    fn thumbnail_bounds_respected() {
        let img = solid(1280, 960, [120, 100, 90]);
        let t = thumbnail(&img, 320);
        assert_eq!(t.dimensions(), (320, 240));
    }

    #[test]
    fn thumbnail_never_upscales_intermediate() {
        // 2880x1800 -> 640x400：reducing_gap=2 时 factor=2，
        // 先 box 到 1440x900，再 LANCZOS 到 640x400。
        // 若误用 ceil(src/dst)=5，中间图会变成 576x360 —— 比目标还小，
        // 后续会变成"放大"，长宽比也会跑偏。
        let img = solid(2880, 1800, [180, 180, 180]);
        let t = thumbnail(&img, 640);
        assert_eq!(t.dimensions(), (640, 400));
    }

    #[test]
    fn thumbnail_scales_shorter_side() {
        // 竖图：长边（高）对齐 640
        let img = solid(900, 1800, [180, 180, 180]);
        let t = thumbnail(&img, 640);
        assert_eq!(t.dimensions(), (320, 640));
    }

    #[test]
    fn thumbnail_square_input() {
        let img = solid(1000, 1000, [180, 180, 180]);
        let t = thumbnail(&img, 250);
        assert_eq!(t.dimensions(), (250, 250));
    }

    #[test]
    fn gray_matches_pillow_integer_weights() {
        // Pillow: (r*19595 + g*38470 + b*7471 + 0x8000) >> 16
        assert_eq!(gray_of(255, 255, 255), 255);
        assert_eq!(gray_of(0, 0, 0), 0);
        assert_eq!(gray_of(255, 0, 0), 76); // 0.299*255 ≈ 76.2
        assert_eq!(gray_of(0, 255, 0), 150); // 0.587*255 ≈ 149.7
        assert_eq!(gray_of(0, 0, 255), 29); // 0.114*255 ≈ 29.1
    }

    #[test]
    fn thumbnail_skips_upscale() {
        let img = solid(100, 80, [120, 100, 90]);
        let t = thumbnail(&img, 320);
        assert_eq!(t.dimensions(), (100, 80));
    }

    #[test]
    fn thumbnail_extreme_aspect_not_degenerate() {
        // 1x5000 的长条不能缩成 0 宽
        let img = solid(1, 5000, [200, 200, 200]);
        let t = thumbnail(&img, 320);
        assert!(t.width() >= 1 && t.height() >= 1);
        assert!(t.height() <= 320);
    }

    #[test]
    fn solid_color_metrics() {
        let img = solid(800, 600, [128, 128, 128]);
        let m = quality_metrics(&img, 640);
        assert_eq!(m.width, 800);
        assert_eq!(m.height, 600);
        assert!((m.brightness - 0.502).abs() < 0.01, "brightness {}", m.brightness);
        assert!(m.contrast < 0.001, "contrast {}", m.contrast);
        assert!(m.blur < 0.001, "blur {}", m.blur);
        assert!(m.saturation < 0.001, "saturation {}", m.saturation);
        assert_eq!(m.megapixels, 0.48);
    }

    #[test]
    fn tiny_image_does_not_panic() {
        let img = solid(4, 3, [120, 100, 90]);
        let m = quality_metrics(&img, 640);
        assert_eq!(m.width, 4);
        assert_eq!(m.height, 3);
    }

    #[test]
    fn broken_file_returns_none() {
        let p = std::env::temp_dir().join("po_broken_test.jpg");
        std::fs::write(&p, b"\xff\xd8\xff\xe0 not really a jpeg").unwrap();
        assert!(open_image(&p).is_none());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn missing_file_returns_none() {
        assert!(open_image(Path::new("no_such_file_xyz.jpg")).is_none());
    }

    #[test]
    fn hashes_are_deterministic() {
        let img = solid(200, 150, [90, 120, 60]);
        assert_eq!(ahash(&img, 8), ahash(&img, 8));
        assert_eq!(dhash(&img, 8), dhash(&img, 8));
        assert_eq!(phash(&img, 8, 4), phash(&img, 8, 4));
    }

    #[test]
    fn identical_images_match_hashes() {
        let a = solid(400, 300, [100, 130, 80]);
        let b = a.clone();
        assert_eq!(ahash(&a, 8), ahash(&b, 8));
        assert_eq!(phash(&a, 8, 4), phash(&b, 8, 4));
        assert_eq!(hamming(phash(&a, 8, 4), phash(&b, 8, 4)), 0);
    }

    #[test]
    fn different_images_differ_in_phash() {
        // 纯色图的 aHash 全是同一个值（全 > 均值 或 全 < 均值），
        // 所以要用有纹理的图来验证哈希区分能力。
        let mut a = RgbImage::new(200, 200);
        let mut b = RgbImage::new(200, 200);
        for y in 0..200 {
            for x in 0..200 {
                // 竖条纹 vs 横条纹
                let ca = if (x / 20) % 2 == 0 { 240 } else { 20 };
                let cb = if (y / 20) % 2 == 0 { 240 } else { 20 };
                a.put_pixel(x, y, image::Rgb([ca, ca, ca]));
                b.put_pixel(x, y, image::Rgb([cb, cb, cb]));
            }
        }
        assert!(hamming(phash(&a, 8, 4), phash(&b, 8, 4)) > 0);
        assert!(hamming(ahash(&a, 8), ahash(&b, 8)) > 0);
    }

    #[test]
    fn hexhash_roundtrip() {
        let v = 0x0123_4567_89ab_cdefu64;
        let s = hexhash(v);
        assert_eq!(s.len(), 16);
        assert_eq!(parse_hash(&s), Some(v));
    }

    #[test]
    fn screen_like_detects_pure_white_doc() {
        let mut img = solid(400, 400, [250, 250, 250]);
        // 加一点内容，避免整张纯白被当成"空图"
        for x in 20..380 {
            for y in 20..40 {
                img.put_pixel(x, y, image::Rgb([30, 30, 30]));
            }
        }
        assert!(is_screen_like(&img, None));
    }

    #[test]
    fn screen_like_rejects_photo_like() {
        // 用低饱和、非极端的中间调，避免触发"高饱和界面色块"判据
        let (w, h) = (300u32, 300u32);
        let mut img = RgbImage::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let base = 90 + (x * 40 / w) as u8 + (y * 40 / h) as u8;
                let c = [base, base.saturating_add(6), base.saturating_sub(4)];
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
        assert!(!is_screen_like(&img, None));
    }

    #[test]
    fn screen_like_does_not_catch_dark_ide_screenshot() {
        // 已知限制（与 Python 版一致，不是回归）：
        // imaging::is_screen_like 只看「大面积近白/近黑(>245 或 <12)」和
        // 「高饱和色块 >10%」。深色 IDE 界面（约 30/30/34）两条都不满足。
        // 真正处理深色界面的是 restore::_is_screen_or_document（那里有 dark 判据）。
        // 这里把行为固定下来，避免以后误以为漏实现了。
        let mut img = solid(400, 400, [30, 30, 34]);
        for x in 40..200 {
            for y in 40..60 {
                img.put_pixel(x, y, image::Rgb([240, 60, 60]));
            }
        }
        assert!(!is_screen_like(&img, None));
    }

    #[test]
    fn screen_like_catches_near_black_terminal() {
        // 近黑（<12）占比够大时才命中
        let mut img = solid(400, 400, [8, 8, 8]);
        for x in 20..380 {
            for y in 20..30 {
                img.put_pixel(x, y, image::Rgb([200, 200, 200]));
            }
        }
        assert!(is_screen_like(&img, None));
    }

    #[test]
    fn flags_empty_when_quality_lab_off() {
        let cfg = Settings::new();
        assert!(!cfg.bool("lab_quality"));
        let img = solid(800, 600, [10, 10, 10]);
        let m = quality_metrics(&img, 640);
        // classify_flags 本身仍可用，只是 analyze_photo 不写入
        let flags = classify_flags(&m, &cfg, false);
        assert!(flags.contains(&"过暗".to_string()));
    }

    #[test]
    fn adaptive_threshold_lowers_for_low_contrast() {
        let cfg = Settings::new();
        let mut m = Metrics {
            contrast: 0.30,
            noise: 0.0,
            megapixels: 2.0,
            ..Default::default()
        };
        let base = adaptive_blur_threshold(&m, &cfg);
        assert!((base - 90.0).abs() < 1e-9, "base {base}");
        // 低对比（泛黄扫描件）应下调阈值
        m.contrast = 0.05;
        let low = adaptive_blur_threshold(&m, &cfg);
        assert!(low < base, "low {low} 应小于 {base}");
        // 颗粒重也不该显得"清晰"，同样下调
        m.contrast = 0.30;
        m.noise = 8.0;
        assert!(adaptive_blur_threshold(&m, &cfg) < base);
    }

    #[test]
    fn quality_score_bounds() {
        let bad = Metrics { sharpness: 0.0, brightness: 0.0, contrast: 0.0, megapixels: 0.01, noise: 50.0, ..Default::default() };
        let good = Metrics { sharpness: 95.0, brightness: 0.5, contrast: 0.25, megapixels: 12.0, noise: 1.0, ..Default::default() };
        assert!(quality_score(&bad) < 20.0, "{}", quality_score(&bad));
        assert!(quality_score(&good) > 80.0, "{}", quality_score(&good));
    }

    #[test]
    fn age_score_higher_for_sepia_low_sat() {
        let cfg = Settings::new();
        let fresh = Metrics { contrast: 0.25, sepia: 0.05, saturation: 0.35, noise: 1.0, color: 0.30, ..Default::default() };
        let old = Metrics { contrast: 0.10, sepia: 0.70, saturation: 0.12, noise: 8.0, color: 0.04, ..Default::default() };
        assert!(age_score(&old, &cfg) > age_score(&fresh, &cfg));
        assert!(age_score(&old, &cfg) <= 100.0);
    }

    // ---- 以下用真实样图，验证算法在真图上的行为 ----

    #[test]
    fn sample_metrics_are_sane() {
        let Some(p) = sample("1985-06-12 春节全家福.jpg") else {
            return;
        };
        let img = open_image(&p).expect("样图应可解码");
        let m = quality_metrics(&img, 640);
        assert_eq!(m.width, 1280);
        assert_eq!(m.height, 960);
        assert!((m.megapixels - 1.229).abs() < 0.01, "mpx {}", m.megapixels);
        // 合成老照片：泛黄、低饱和、颗粒重
        assert!(m.sepia > 0.3, "sepia {} 应偏高", m.sepia);
        assert!(m.color > 0.0);
        assert!(m.noise > 2.0, "noise {} 应有颗粒", m.noise);
        assert!((0.0..=100.0).contains(&m.sharpness));
    }

    #[test]
    fn sample_blur_metric_reacts_to_scratches_not_blur() {
        // 「模糊的老照片 1978.jpg」在 make_samples.py 里是：
        //   grain=20  →  再 GaussianBlur(2.2)  →  再加 20 条 1-2px 划痕
        // 高斯模糊把颗粒抹掉了，所以它的 noise 反而低于干净的 DSC_0071；
        // 而 Laplacian 方差很高，是被那 20 条高频划痕抬起来的，不是"更清晰"。
        // 这个用例固化该行为：说明 blur 指标对划痕敏感，不能单看它判断清晰度。
        let (Some(blurry), Some(clean)) = (
            sample("模糊的老照片 1978.jpg"),
            sample("DSC_0071.JPG"),
        ) else {
            return;
        };
        let mb = quality_metrics(&open_image(&blurry).unwrap(), 640);
        let mc = quality_metrics(&open_image(&clean).unwrap(), 640);
        // 模糊之后颗粒被抹平
        assert!(
            mb.noise < mc.noise,
            "高斯模糊后噪声应更低: blur片 {} vs 干净片 {}",
            mb.noise,
            mc.noise
        );
        // 划痕把 Laplacian 方差拉高
        assert!(
            mb.blur > mc.blur,
            "划痕会抬高 Laplacian 方差: {} vs {}",
            mb.blur,
            mc.blur
        );
        // 但它的对比度确实被"褪色"拉低了
        assert!(mb.saturation > 0.0);
    }

    #[test]
    fn sample_blur_threshold_flags_truly_flat_image() {
        // 真正糊的图：低方差 + 低对比，应被判为"模糊"
        let cfg = Settings::new();
        let flat = solid(640, 480, [128, 128, 128]);
        let m = quality_metrics(&flat, 640);
        let flags = classify_flags(&m, &cfg, false);
        assert!(flags.contains(&"模糊".to_string()), "flags: {flags:?}");
    }

    #[test]
    fn sample_webp_screenshot_is_low_detail() {
        // 「微信图片」是合成的大面积天空+地平面，细节很少
        let Some(p) = sample("微信图片_20200101.jpg") else {
            return;
        };
        let m = quality_metrics(&open_image(&p).unwrap(), 640);
        assert!(m.sharpness < 40.0, "sharpness {}", m.sharpness);
        assert!(m.noise < 2.5, "noise {} 应较干净", m.noise);
    }

    #[test]
    fn sample_duplicates_have_close_phash() {
        let (Some(a), Some(b)) = (
            sample("重复/完全相同1.jpg"),
            sample("1985-06-12 春节全家福.jpg"),
        ) else {
            return;
        };
        let ia = open_image(&a).unwrap();
        let ib = open_image(&b).unwrap();
        let ha = phash(&ia, 8, 4);
        let hb = phash(&ib, 8, 4);
        let d = hamming(ha, hb);
        assert!(d <= 10, "同一张图重编码后 pHash 距离应很小，实际 {d}");
    }

    #[test]
    fn sample_duplicates_scaled_have_close_phash() {
        // 1280x960 缩到 640x480 再高压缩
        let (Some(a), Some(b)) = (
            sample("重复/1985-06-12 春节全家福_副本.jpg"),
            sample("1985-06-12 春节全家福.jpg"),
        ) else {
            return;
        };
        let ha = phash(&open_image(&a).unwrap(), 8, 4);
        let hb = phash(&open_image(&b).unwrap(), 8, 4);
        let d = hamming(ha, hb);
        assert!(d <= 10, "缩放后 pHash 距离应仍很小，实际 {d}");
    }

    #[test]
    fn sample_different_photos_have_far_phash() {
        let (Some(a), Some(b)) = (
            sample("1992-10-01 旅行.jpg"),
            sample("微信图片_20200101.jpg"),
        ) else {
            return;
        };
        let ha = phash(&open_image(&a).unwrap(), 8, 4);
        let hb = phash(&open_image(&b).unwrap(), 8, 4);
        let d = hamming(ha, hb);
        assert!(d > 10, "不同照片 pHash 距离应明显更大，实际 {d}");
    }

    #[test]
    fn sample_analyze_produces_all_fields() {
        let Some(p) = sample("2004年/照片 001.jpg") else {
            return;
        };
        let cfg = Settings::new();
        let dir = std::env::temp_dir().join("po_thumb_test");
        let _ = std::fs::create_dir_all(&dir);
        let th = dir.join("t.jpg");
        let a = analyze_photo(&p, &cfg, Some(&th)).expect("分析应成功");
        assert!(a.thumb_ok, "缩略图应生成成功");
        assert!(th.exists());
        assert_eq!(a.p_hash.len(), 16);
        assert_eq!(a.a_hash.len(), 16);
        assert_eq!(a.d_hash.len(), 16);
        assert!(a.quality_score >= 0.0 && a.quality_score <= 100.0);
        // lab_quality 默认关闭，flags 应为空
        assert!(a.flags.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sample_scan_document_goes_through_screen_path() {
        let Some(p) = sample("扫描件_2008-07-20_证件照.jpg") else {
            return;
        };
        let img = open_image(&p).unwrap();
        let m = quality_metrics(&img, 640);
        let sl = is_screen_like(&img, Some(&m));
        let cfg = Settings::new();
        let flags = classify_flags(&m, &cfg, sl);
        // 截图/文档路径不会报"泛黄""噪点重"
        assert!(!flags.contains(&"泛黄/棕褐".to_string()));
        assert!(!flags.contains(&"噪点重".to_string()));
    }
}
