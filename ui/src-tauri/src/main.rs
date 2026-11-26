#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::path::PathBuf;
use std::process::Command;

use commands::AppState;
use std::sync::Arc;
use tauri::http::{Request, Response, StatusCode};
use tauri::Manager;

/// 图片走自定义协议 `po://local/thumb?id=1&size=480`。
///
/// 原来这些请求走本地 HTTP 后端（`127.0.0.1:18921`），
/// 现在引擎在进程内，直接读字节返回，少一个进程和一次网络往返。
fn handle_po_protocol(request: Request<Vec<u8>>, state: &AppState) -> Response<Vec<u8>> {
    let not_found = || {
        Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Vec::new())
            .unwrap()
    };

    let uri = request.uri().clone();
    // 视频播放要靠 Range 头做分段读取，得先把它取出来
    let range = request
        .headers()
        .get("range")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let path = uri.path().trim_start_matches('/').to_string();
    let query: std::collections::HashMap<String, String> = uri
        .query()
        .map(|q| {
            q.split('&')
                .filter_map(|kv| {
                    let mut it = kv.splitn(2, '=');
                    let k = it.next()?.to_string();
                    let v = it.next().unwrap_or("");
                    Some((k, percent_decode(v)))
                })
                .collect()
        })
        .unwrap_or_default();

    let id = query.get("id").and_then(|s| s.parse::<i64>().ok());
    let file_path = query.get("path").cloned();
    let size = query
        .get("size")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(480);

    let Some(resolved) = commands::resolve_image_path(state, id, file_path.as_deref()) else {
        return not_found();
    };

    // 视频没有"原图"可实时缩放：它的缩略图是扫描时用系统接口
    // 预生成好的 JPEG（存在 thumb 字段里）。所以视频请求直接读那个文件，
    // 不能走 serve_thumb（那会对视频调图像解码器，必然失败）。
    let is_video_file = matches!(
        resolved
            .extension()
            .and_then(|s| s.to_str())
            .map(|s| s.to_ascii_lowercase())
            .as_deref(),
        Some("mp4" | "m4v" | "mov" | "avi" | "mkv" | "wmv" | "flv" | "webm" | "3gp" | "mpg" | "mpeg" | "mts" | "m2ts")
    );

    let (body, mime) = match path.as_str() {
        "thumb" => {
            // 视频源文件 → 直接读预生成的缩略图
            if is_video_file {
                // 先从库里读预生成的缩略图路径
                let stored = state
                    .with_lib(|lib| id.and_then(|i| lib.photo(i)).and_then(|r| r.thumb))
                    .ok()
                    .flatten();

                // 缓存目录可能被用户清理、或换了机器/版本，导致文件已不在。
                // 这时现场重新生成一次并写回库，避免整片视频永久显示占位图。
                let usable = stored.filter(|s| std::path::Path::new(s).is_file());
                let cover = match usable {
                    Some(t) => Some(t),
                    None => commands::regenerate_video_cover(state, id, &resolved),
                };
                let Some(t) = cover else {
                    return not_found();
                };
                match std::fs::read(&t) {
                    Ok(d) => (d, "image/jpeg".to_string()),
                    Err(_) => return not_found(),
                }
            } else {
                match commands::serve_thumb(state, &resolved, size) {
                    Some(d) => (d, "image/jpeg".to_string()),
                    None => return not_found(),
                }
            }
        },
        "photo" => match std::fs::read(&resolved) {
            Ok(d) => {
                let mime = mime_of(&resolved).to_string();
                (d, mime)
            }
            Err(_) => {
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Vec::new())
                    .unwrap()
            }
        },
        // 视频：支持 Range 请求，浏览器才能拖动进度条
        "video" => match video_response(&resolved, range.as_deref()) {
            Some(r) => return r,
            None => return not_found(),
        },
        _ => return not_found(),
    };

    // 缩略图可以长缓存（键里含 mtime+size，内容变了 URL 会变）；
    // 原图必须短缓存，否则重置索引后 id 复用会显示旧图。
    let cache = if path == "thumb" {
        "public, max-age=604800"
    } else {
        "no-store, must-revalidate"
    };

    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", mime)
        .header("Cache-Control", cache)
        .header("Access-Control-Allow-Origin", "*")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

fn mime_of(p: &PathBuf) -> &'static str {
    match p
        .extension()
        .and_then(|s| s.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("jpg") | Some("jpeg") | Some("jpe") | Some("jfif") => "image/jpeg",
        Some("png") => "image/png",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("bmp") => "image/bmp",
        Some("tif") | Some("tiff") => "image/tiff",
        Some("mp4") | Some("m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("webm") => "video/webm",
        Some("mkv") => "video/x-matroska",
        Some("avi") => "video/x-msvideo",
        _ => "application/octet-stream",
    }
}

/// 提供视频字节，支持 HTTP Range。
///
/// `<video>` 要能拖进度条就必须支持 206 Partial Content，
/// 否则 WebView2 只能从头播、拖不动。
fn video_response(file: &std::path::Path, range: Option<&str>) -> Option<Response<Vec<u8>>> {
    use std::io::{Read, Seek, SeekFrom};

    let meta = std::fs::metadata(file).ok()?;
    let total = meta.len();
    if total == 0 {
        return None;
    }
    let mime = mime_of(&file.to_path_buf());

    let mut f = std::fs::File::open(file).ok()?;

    // 解析 "bytes=start-end"
    let parsed = range.and_then(|r| {
        let spec = r.trim().strip_prefix("bytes=")?;
        let (a, b) = spec.split_once('-')?;
        let start: u64 = a.trim().parse().ok()?;
        let end: u64 = if b.trim().is_empty() {
            total - 1
        } else {
            b.trim().parse::<u64>().ok()?.min(total - 1)
        };
        (start <= end && start < total).then_some((start, end))
    });

    match parsed {
        Some((start, end)) => {
            let len = (end - start + 1) as usize;
            let _ = f.seek(SeekFrom::Start(start));
            let mut buf = vec![0u8; len];
            let mut read_total = 0usize;
            while read_total < len {
                match f.read(&mut buf[read_total..]) {
                    Ok(0) => break,
                    Ok(n) => read_total += n,
                    Err(_) => return None,
                }
            }
            buf.truncate(read_total);
            if read_total == 0 {
                return None;
            }
            Response::builder()
                .status(StatusCode::PARTIAL_CONTENT)
                .header("Content-Type", mime)
                .header("Content-Length", read_total.to_string())
                .header(
                    "Content-Range",
                    format!("bytes {start}-{}-{total}", start + read_total as u64 - 1),
                )
                .header("Accept-Ranges", "bytes")
                .header("Cache-Control", "no-store, must-revalidate")
                .header("Access-Control-Allow-Origin", "*")
                .body(buf)
                .ok()
        }
        None => {
            // 没有 Range（或格式不认识）：整文件返回。
            // 大文件会占内存，但视频通常只有几十 MB，可接受。
            let mut buf = Vec::with_capacity(total as usize);
            f.read_to_end(&mut buf).ok()?;
            Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", mime)
                .header("Content-Length", buf.len().to_string())
                .header("Accept-Ranges", "bytes")
                .header("Cache-Control", "no-store, must-revalidate")
                .header("Access-Control-Allow-Origin", "*")
                .body(buf)
                .ok()
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 用系统默认程序打开文件或文件夹。
#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // 用 raw_arg 显式加引号：路径含空格或中文时更稳
        Command::new("explorer")
            .raw_arg(format!("\"{}\"", path.replace('"', "")))
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("unsupported".into())
    }
}

/// 决定「在资源管理器中打开」时实际要传的路径。
///
/// * 文件存在 → `/select,<文件>` 定位并高亮
/// * 文件不存在但父目录在 → 打开父目录
/// * 目录 → 直接打开
/// 抽成独立函数是为了能单测这三条分支。
fn reveal_target(path: &str) -> Option<String> {
    let p = std::path::PathBuf::from(path);
    if p.is_file() {
        return Some(format!("/select,{path}"));
    }
    if p.is_dir() {
        return Some(path.to_string());
    }
    // 文件不在了：退回父目录
    p.parent()
        .filter(|d| d.is_dir())
        .map(|d| d.to_string_lossy().to_string())
}

/// 在资源管理器里定位并选中文件。
///
/// `explorer /select,<path>` 会打开所在文件夹并高亮该文件。
///
/// 两个坑：
///  1. `/select,` 后面**不能有空格**，否则会被当成另一个参数；
///  2. 路径含空格时，必须用 `raw_arg` 手动加引号。
///     用 `arg()` 让 Rust 自动加引号的话，explorer 拿到的是
///     `"/select,C:\a b\x.jpg"` 这种整体带引号的形式，它识别不了，
///     结果只是打开「文档」目录而不定位文件。
#[tauri::command]
fn reveal_in_explorer(path: String) -> Result<(), String> {
    if path.trim().is_empty() {
        return Err("路径为空".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        let Some(target) = reveal_target(&path) else {
            return Err("路径不存在".into());
        };
        let mut cmd = Command::new("explorer");
        if let Some(rest) = target.strip_prefix("/select,") {
            // 路径里的引号在 Windows 文件名中不合法，直接拼双引号即可
            cmd.raw_arg(format!("/select,\"{rest}\""));
        } else {
            cmd.raw_arg(format!("\"{target}\""));
        }
        cmd.spawn().map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = path;
        Err("unsupported".into())
    }
}

/// 用 Windows 现代文件资源管理器对话框选文件夹。
///
/// Tauri 的 dialog 插件在 Windows 上默认走旧式 SHBrowseForFolder，
/// 外观是老式树形框；这里改用 COM 的 IFileOpenDialog + FOS_PICKFOLDERS，
/// 得到和「另存为」一致的现代资源管理器界面。
#[tauri::command]
async fn pick_folder() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use windows::core::PCWSTR;
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, IFileOpenDialog, IShellItem, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
            FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
        };

        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let need_uninit = hr.is_ok();

            let result = (|| -> Option<String> {
                let dialog: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;

                let mut opts = dialog.GetOptions().ok()?;
                opts |= FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST;
                dialog.SetOptions(opts).ok()?;
                let title = windows::core::HSTRING::from("选择文件夹");
                let ok_label = windows::core::HSTRING::from("选择此文件夹");
                dialog.SetTitle(PCWSTR(title.as_ptr())).ok()?;
                dialog.SetOkButtonLabel(PCWSTR(ok_label.as_ptr())).ok()?;

                dialog.Show(None).ok()?;

                let item: IShellItem = dialog.GetResult().ok()?;
                let pw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let s = pw.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(pw.0 as *const _));
                s
            })();

            if need_uninit {
                CoUninitialize();
            }
            result
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
}

/// 选择单个文件（备份恢复用）。
#[tauri::command]

fn main() {
    // 固定大小的解码线程池：一次加载几百张缩略图时，    // 不能每个请求都 spawn 一个线程（会瞬间冒出几百个线程），
    // 也不能在主线程同步解码（会卡死 UI）。
    let pool = Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(
                std::thread::available_parallelism()
                    .map(|n| n.get())
                    .unwrap_or(4)
                    .clamp(2, 8),
            )
            .thread_name(|i| format!("po-thumb-{i}"))
            .build()
            .expect("缩略图线程池创建失败"),
    );

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        // 必须用异步版：同步协议回调跑在 WebView2 的 IPC 线程上，
        // 一次加载几百张缩略图时会在那里同步解码 JPEG，
        // 把整个线程堵死 —— 表现就是图片全黑 + 界面卡顿无响应。
        .register_asynchronous_uri_scheme_protocol("po", move |ctx, request, responder| {
            let state = ctx.app_handle().state::<AppState>().inner().clone();
            pool.spawn(move || {
                responder.respond(handle_po_protocol(request, &state));
            });
        })
        .invoke_handler(tauri::generate_handler![
            // 原生能力
            open_path,
            reveal_in_explorer,
            pick_folder,
            // 查询
            commands::ping,
            commands::get_stats,
            commands::get_photos,
            commands::get_photo,
            commands::get_timeline,
            commands::get_places,
            commands::get_duplicates,
            commands::get_roots,
            commands::get_settings,
            commands::get_restore_presets,
            commands::get_scan_progress,
            // 信息修正
            commands::edit_photo,
            commands::set_photo_tags,
            // 写操作
            commands::save_settings,
            commands::add_root,
            commands::remove_root,
            commands::scan_start,
            commands::scan_cancel,
            commands::set_favorite,
            commands::reclassify,
            commands::backfill_places,
            commands::reset_index,
            commands::set_dup_keeper,
            commands::delete_photos,
            commands::export_photos,
            // 回收站
            commands::get_recycle,
            commands::restore_recycle,
            commands::purge_recycle,
            // 修复
            commands::restore_apply,
            commands::restore_batch,
            // 整理
            commands::organize,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_target_file_uses_select_flag() {
        let f = std::env::temp_dir().join("po_reveal_test.jpg");
        std::fs::write(&f, b"x").unwrap();
        let got = reveal_target(&f.to_string_lossy());
        // 必须带 /select,，否则只会打开目录而不高亮文件
        assert!(got.unwrap().starts_with("/select,"));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn reveal_arg_quotes_path_with_spaces() {
        // 含空格的路径必须整体加引号，否则 explorer 会把它拆成两个参数
        let f = std::env::temp_dir().join("po reveal space test.jpg");
        std::fs::write(&f, b"x").unwrap();
        let target = reveal_target(&f.to_string_lossy()).expect("应定位到文件");
        let rest = target.strip_prefix("/select,").unwrap();
        let arg = format!("/select,\"{rest}\"");
        assert!(arg.starts_with("/select,\""), "arg: {arg}");
        assert!(arg.ends_with('"'), "arg: {arg}");
        // /select, 与引号之间不能有空格
        assert!(!arg.starts_with("/select, "), "不能在 /select, 后加空格");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn reveal_arg_handles_chinese_path() {
        let f = std::env::temp_dir().join("照片 中文测试.jpg");
        std::fs::write(&f, b"x").unwrap();
        let target = reveal_target(&f.to_string_lossy()).expect("中文路径也应能定位");
        assert!(target.contains("照片 中文测试.jpg"), "target: {target}");
        assert!(target.starts_with("/select,"));
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn open_path_arg_is_quoted() {
        // open_path 用 raw_arg 显式加引号，这里验证拼出来的字符串形态
        let p = r"C:\a b\照片.jpg";
        let arg = format!("\"{}\"", p.replace('"', ""));
        assert!(arg.starts_with('"') && arg.ends_with('"'));
        assert_eq!(arg, "\"C:\\a b\\照片.jpg\"");
    }

    #[test]
    fn reveal_target_directory_is_plain() {
        let d = std::env::temp_dir();
        let got = reveal_target(&d.to_string_lossy()).unwrap();
        assert!(!got.starts_with("/select,"), "目录不该带 /select");
    }

    #[test]
    fn reveal_target_missing_file_falls_back_to_parent() {
        // 文件不存在 → 退回父目录（父目录存在）
        let p = std::env::temp_dir().join("no_such_po_file.jpg");
        let got = reveal_target(&p.to_string_lossy()).expect("应退回父目录");
        // 用 Path 比较，避免末尾斜杠差异导致误判
        assert_eq!(
            std::path::Path::new(&got),
            std::env::temp_dir().as_path(),
            "got: {got}"
        );
    }

    #[test]
    fn reveal_target_nonexistent_everything_returns_none() {
        assert!(reveal_target("Z:\\no\\such\\dir\\x.jpg").is_none());
    }

    #[test]
    fn percent_decode_handles_escapes() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("%E5%8C%97%E4%BA%AC"), "北京");
        assert_eq!(percent_decode("a+b"), "a b");
        assert_eq!(percent_decode("abc"), "abc");
        // 不完整的转义不能 panic
        assert_eq!(percent_decode("a%"), "a%");
        assert_eq!(percent_decode("a%2"), "a%2");
    }

    #[test]
    fn mime_of_handles_video_extensions() {
        assert_eq!(mime_of(&PathBuf::from("a.mp4")), "video/mp4");
        assert_eq!(mime_of(&PathBuf::from("a.MOV")), "video/quicktime");
        assert_eq!(mime_of(&PathBuf::from("a.webm")), "video/webm");
        assert_eq!(mime_of(&PathBuf::from("a.jpg")), "image/jpeg");
        assert_eq!(mime_of(&PathBuf::from("a.unknown")), "application/octet-stream");
    }

    #[test]
    fn video_response_supports_range() {
        // 造一个 2KB 的假文件
        let f = std::env::temp_dir().join("po_range_test.mp4");
        let data: Vec<u8> = (0..2048u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&f, &data).unwrap();

        // 带 Range → 206 + 正确长度
        let r = video_response(&f, Some("bytes=0-99")).expect("应有响应");
        assert_eq!(r.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(r.body().len(), 100);

        // 不带 Range → 200 + 全文件
        let r2 = video_response(&f, None).expect("应有响应");
        assert_eq!(r2.status(), StatusCode::OK);
        assert_eq!(r2.body().len(), 2048);
        assert_eq!(r2.body(), &data);

        // 末尾越界的 Range 要夹到文件尾
        let r3 = video_response(&f, Some("bytes=2000-99999")).expect("应有响应");
        assert_eq!(r3.body().len(), 48);

        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn video_response_rejects_missing_file() {
        assert!(video_response(std::path::Path::new("no_such.mp4"), None).is_none());
    }
}
