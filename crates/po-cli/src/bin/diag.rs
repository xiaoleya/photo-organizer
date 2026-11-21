//! 临时诊断：打印样图的真实指标，用于校准测试断言。

use po_core::imaging::{classify_flags, quality_metrics, is_screen_like, adaptive_blur_threshold};
use po_core::config::Settings;

fn main() {
    let cfg = Settings::new();
    let base = std::path::Path::new("E:/agent xm/照片整理-GitHub发布/源码/samples");
    let names = [
        "1985-06-12 春节全家福.jpg",
        "1985年06月12日 老家院子合影.jpg",
        "1992-10-01 旅行.jpg",
        "IMG_20050315_毕业典礼.jpg",
        "20050315_141530.jpg",
        "扫描件_2008-07-20_证件照.jpg",
        "微信图片_20200101.jpg",
        "DSC_0071.JPG",
        "模糊的老照片 1978.jpg",
    ];
    println!("{:<30} {:>7} {:>7} {:>7} {:>7} {:>7} {:>6} {:>6}  flags", "文件", "blur", "contrast", "noise", "sepia", "color", "sharp", "thresh");
    for n in names {
        let p = base.join(n);
        let Some(img) = po_core::imaging::open_image(&p) else {
            println!("{n:<30} 解码失败");
            continue;
        };
        let m = quality_metrics(&img, 640);
        let sl = is_screen_like(&img, Some(&m));
        let thr = adaptive_blur_threshold(&m, &cfg);
        let flags = classify_flags(&m, &cfg, sl);
        println!(
            "{:<30} {:>7.1} {:>7.4} {:>7.2} {:>7.4} {:>7.4} {:>6.1} {:>6.1}  {} {}",
            n, m.blur, m.contrast, m.noise, m.sepia, m.color, m.sharpness, thr,
            if sl { "[截图]" } else { "" },
            flags.join("|")
        );
    }
}
