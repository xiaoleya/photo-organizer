//! 对真实库里的照片重算指标，与 Python 版存库结果对比。
//! 只读：不写回 library.db。

use po_core::imaging::{open_image, quality_metrics};
use po_core::Library;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let db = args
        .first()
        .cloned()
        .unwrap_or_else(|| "E:/po-verify/library.db".to_string());
    let limit: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);

    let lib = Library::open(Path::new(&db)).expect("打开库失败");
    let rows = lib.all_photos("analyzed_at IS NOT NULL AND path IS NOT NULL", "id");
    println!("库内已分析照片: {}，抽样 {} 张对比", rows.len(), limit);

    // 各指标的相对误差统计
    let keys = [
        "blur", "brightness", "contrast", "saturation", "sepia", "color", "skin", "noise",
        "sharpness", "dark_ratio", "white_ratio",
    ];
    let mut errs: Vec<(&str, Vec<f64>)> = keys.iter().map(|k| (*k, Vec::new())).collect();
    let mut compared = 0usize;
    let mut skipped = 0usize;

    for r in rows.iter().take(limit) {
        let Some(img) = open_image(Path::new(&r.path)) else {
            skipped += 1;
            continue;
        };
        let m = quality_metrics(&img, 640);
        let pairs: [(&str, f64, f64); 11] = [
            ("blur", r.blur.unwrap_or(0.0), m.blur),
            ("brightness", r.brightness.unwrap_or(0.0), m.brightness),
            ("contrast", r.contrast.unwrap_or(0.0), m.contrast),
            ("saturation", r.saturation.unwrap_or(0.0), m.saturation),
            ("sepia", r.sepia.unwrap_or(0.0), m.sepia),
            ("color", r.color.unwrap_or(0.0), m.color),
            ("skin", r.skin.unwrap_or(0.0), m.skin),
            ("noise", r.noise.unwrap_or(0.0), m.noise),
            ("sharpness", r.sharpness.unwrap_or(0.0), m.sharpness),
            ("dark_ratio", r.dark_ratio.unwrap_or(0.0), m.dark_ratio),
            ("white_ratio", r.white_ratio.unwrap_or(0.0), m.white_ratio),
        ];
        for (name, py, rs) in pairs {
            // 相对误差；两值都极小时按绝对差处理
            let denom = py.abs().max(0.02);
            let e = (rs - py).abs() / denom;
            if let Some((_, v)) = errs.iter_mut().find(|(k, _)| *k == name) {
                v.push(e);
            }
        }
        compared += 1;
    }

    println!("成功比对 {compared} 张（跳过解码失败 {skipped} 张）\n");
    println!("{:<12} {:>8} {:>8} {:>8} {:>8}", "指标", "中位误差", "均值误差", "P90", "最大");
    for (name, mut v) in errs {
        if v.is_empty() {
            continue;
        }
        v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = v.len();
        let med = v[n / 2];
        let mean = v.iter().sum::<f64>() / n as f64;
        let p90 = v[(n * 9 / 10).min(n - 1)];
        let max = v[n - 1];
        println!(
            "{:<12} {:>7.1}% {:>7.1}% {:>7.1}% {:>7.1}%",
            name, med * 100.0, mean * 100.0, p90 * 100.0, max * 100.0
        );
    }
    println!("\n（相对误差 = |Rust - Python| / max(|Python|, 0.02)）");
}
