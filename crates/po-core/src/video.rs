//! 视频支持（用 Windows 系统缩略图，不捆绑 ffmpeg）。
//!
//! 设计取舍：**不捆绑 ffmpeg**。
//!
//! ffmpeg 是 60 MB 的独立程序，无法真正静态编译进主程序；
//! 无论打进安装包还是运行时释放，都会让软件体积翻三倍，
//! 对「整理照片」这个小工具不值得。
//!
//! 那视频封面怎么办？用 **Windows 自带的缩略图接口**
//! （`IShellItemImageFactory`，就是资源管理器里预览视频用的那套）。
//! 系统已经装了对应解码器，我们只负责取图，不自己解码。
//!
//! 所以：
//!   * 视频**进图库**，参与时间轴 / 地点 / 归档 / 去重
//!   * 视频**有封面图**（系统缩略图）
//!   * 播放走系统播放器
//!   * 软件体积保持 4 MB 量级

use std::path::Path;

/// 用系统缩略图接口抽取视频封面，存成 JPEG。
///
/// 走 `IShellItemImageFactory::GetImage`，和资源管理器显示视频预览图
/// 是同一套机制，因此无需自带解码器。
/// 失败（系统没装对应解码器、文件损坏）返回 `false`，前端显示占位图标。
#[cfg(target_os = "windows")]
pub fn extract_frame(video: &Path, out: &Path, max_side: u32) -> bool {
    win::shell_thumbnail(video, out, max_side).unwrap_or(false)
}

#[cfg(not(target_os = "windows"))]
pub fn extract_frame(_video: &Path, _out: &Path, _max_side: u32) -> bool {
    false
}

/// 判断一个已存在的文件是不是「有效封面」：
/// 太小、不是 JPEG、或整张几乎全黑，都算无效（系统占位图）。
pub fn is_valid_cover(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() < 512 {
        return false;
    }
    let Ok(img) = image::open(path) else {
        return false;
    };
    let rgb = img.to_rgb8();
    // 缩到小图后统计亮度，避免大图逐像素太慢
    let small = crate::imaging::thumbnail(&rgb, 64);
    let mut sum = 0u64;
    let mut bright = 0u64;
    let mut n = 0u64;
    for p in small.pixels() {
        let luma = (p[0] as u64 * 29 + p[1] as u64 * 150 + p[2] as u64 * 77) / 256;
        sum += luma;
        n += 1;
        if luma > 24 {
            bright += 1;
        }
    }
    if n == 0 {
        return false;
    }
    let avg = sum as f64 / n as f64;
    let bright_ratio = bright as f64 / n as f64;
    avg >= 6.0 && bright_ratio >= 0.005
}

/// 是否能提取系统缩略图（仅 Windows）。
pub fn is_available() -> bool {
    cfg!(target_os = "windows")
}

// ---------------------------------------------------------------- Windows 实现

#[cfg(target_os = "windows")]
mod win {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// 调 Shell API 拿缩略图，再编码成 JPEG 存盘。
    ///
    ///   SHCreateItemFromParsingName -> IShellItem
    ///   IShellItem::cast            -> IShellItemImageFactory
    ///   IShellItemImageFactory::GetImage -> HBITMAP
    pub fn shell_thumbnail(video: &Path, out: &Path, max_side: u32) -> Option<bool> {
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::Foundation::SIZE;
        use windows::Win32::Graphics::Gdi::{
            DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
            BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
        };
        use windows::Win32::System::Com::{
            CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{
            IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
        };

        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let need_uninit = hr.is_ok();

            let result = (|| -> Option<bool> {
                let path_w = wide(video);
                let item: windows::Win32::UI::Shell::IShellItem =
                    SHCreateItemFromParsingName(PCWSTR(path_w.as_ptr()), None).ok()?;

                let factory: IShellItemImageFactory = item.cast().ok()?;

                let size = SIZE {
                    cx: max_side as i32,
                    cy: max_side as i32,
                };
                let hbmp = factory.GetImage(size, SIIGBF_BIGGERSIZEOK).ok()?;

                let mut bmp = BITMAP::default();
                let got = GetObjectW(
                    HGDIOBJ(hbmp.0),
                    std::mem::size_of::<BITMAP>() as i32,
                    Some(&mut bmp as *mut _ as *mut _),
                );
                if got == 0 {
                    let _ = DeleteObject(HGDIOBJ(hbmp.0));
                    return Some(false);
                }

                let (w, h) = (bmp.bmWidth, bmp.bmHeight);
                if w <= 0 || h <= 0 {
                    let _ = DeleteObject(HGDIOBJ(hbmp.0));
                    return Some(false);
                }

                // 32 位 BGRA，自上而下（biHeight 为负）
                let mut bi = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..Default::default()
                    },
                    ..Default::default()
                };

                let mut buf = vec![0u8; (w * h * 4) as usize];
                let dc = GetDC(None);
                let lines = GetDIBits(
                    dc,
                    hbmp,
                    0,
                    h as u32,
                    Some(buf.as_mut_ptr() as *mut _),
                    &mut bi,
                    DIB_RGB_COLORS,
                );
                ReleaseDC(None, dc);
                let _ = DeleteObject(HGDIOBJ(hbmp.0));

                if lines == 0 {
                    return Some(false);
                }

                // BGRA -> RGBA；全透明说明系统没法生成预览
                let mut rgba = Vec::with_capacity((w * h * 4) as usize);
                let mut any_opaque = false;
                let mut sum_luma: u64 = 0;
                let mut bright_px: u64 = 0;
                let mut counted: u64 = 0;
                for px in buf.chunks_exact(4) {
                    let a = px[3];
                    if a > 8 {
                        any_opaque = true;
                    }
                    // Bi=px[0], Gi=px[1], Ri=px[2]（BGRA 顺序）
                    let luma = (px[0] as u64 * 29 + px[1] as u64 * 150 + px[2] as u64 * 77) / 256;
                    sum_luma += luma;
                    counted += 1;
                    if luma > 24 {
                        bright_px += 1;
                    }
                    rgba.push(px[2]);
                    rgba.push(px[1]);
                    rgba.push(px[0]);
                    rgba.push(a);
                }
                if !any_opaque {
                    return Some(false);
                }
                // 系统有时会返回一张「全黑但 alpha=255」的占位图（没装解码器时），
                // 这种图存下来就是用户看到的黑封面，宁可不存、让前端显示占位图标。
                let avg = if counted > 0 { sum_luma as f64 / counted as f64 } else { 0.0 };
                let bright_ratio = if counted > 0 {
                    bright_px as f64 / counted as f64
                } else {
                    0.0
                };
                if avg < 6.0 || bright_ratio < 0.005 {
                    return Some(false);
                }

                let img: image::RgbaImage =
                    image::ImageBuffer::from_raw(w as u32, h as u32, rgba)?;

                // 白底合成后存 JPEG
                let mut rgb = image::RgbImage::new(w as u32, h as u32);
                for (x, y, p) in img.enumerate_pixels() {
                    let a = p.0[3] as u32;
                    let f = |c: u8| (((c as u32) * a + 255 * (255 - a)) / 255) as u8;
                    rgb.put_pixel(x, y, image::Rgb([f(p.0[0]), f(p.0[1]), f(p.0[2])]));
                }

                if let Some(parent) = out.parent() {
                    crate::config::ensure_dir(parent.to_path_buf());
                }
                let mut jpg = Vec::new();
                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpg, 84);
                enc.encode_image(&rgb).ok()?;
                std::fs::write(out, &jpg).ok()?;
                Some(std::fs::metadata(out).map(|m| m.len() > 256).unwrap_or(false))
            })();

            if need_uninit {
                CoUninitialize();
            }
            Some(result.unwrap_or(false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_degrades() {
        let p = Path::new("no_such_video_xyz.mp4");
        assert!(!extract_frame(p, Path::new("o.jpg"), 320));
        assert!(!is_valid_cover(Path::new("no_such_cover.jpg")));
    }

    #[test]
    fn available_matches_platform() {
        assert_eq!(is_available(), cfg!(target_os = "windows"));
    }
}
