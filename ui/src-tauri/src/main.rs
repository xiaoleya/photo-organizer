#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;

use std::path::PathBuf;

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

    let (body, mime) = match path.as_str() {
        "thumb" => match commands::serve_thumb(state, &resolved, size) {
            Some(d) => (d, "image/jpeg"),
            None => return not_found(),
        },
        "photo" => match std::fs::read(&resolved) {
            Ok(d) => {
                let mime = mime_of(&resolved);
                (d, mime)
            }
            Err(_) => {
                return Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(Vec::new())
                    .unwrap()
            }
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
        _ => "application/octet-stream",
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
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
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

fn main() {
    // 固定大小的解码线程池：一次加载几百张缩略图时，
    // 不能每个请求都 spawn 一个线程（会瞬间冒出几百个线程），
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
            pick_folder,
            // 查询
            commands::ping,
            commands::get_stats,
            commands::get_photos,
            commands::get_photo,
            commands::get_tags,
            commands::get_timeline,
            commands::get_places,
            commands::get_events,
            commands::get_duplicates,
            commands::get_roots,
            commands::get_settings,
            commands::get_restore_presets,
            commands::get_scan_progress,
            commands::get_startup_state,
            // 写操作
            commands::save_settings,
            commands::add_root,
            commands::remove_root,
            commands::scan_start,
            commands::scan_sync,
            commands::set_favorite,
            commands::reclassify,
            commands::backfill_places,
            commands::reset_index,
            commands::clean_duplicates,
            commands::delete_photos,
            // 修复
            commands::restore_one,
            commands::restore_apply,
            commands::restore_batch,
            // 整理
            commands::organize,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
