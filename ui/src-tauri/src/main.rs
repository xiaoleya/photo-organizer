#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::Child;
use tauri::Manager;

/// 后端子进程句柄（sidecar 的 CommandChild 或 std Child 二选一）
enum BackendChild {
    Sidecar(tauri_plugin_shell::process::CommandChild),
    Std(std::process::Child),
}

impl BackendChild {
    /// kill 会消费自身：sidecar 的 kill(self) 需要所有权。
    /// sidecar 的 kill 只是发信号，子进程可能还在退出中；
    /// 因此再按镜像名补一刀，确保不留残影。
    fn kill(self) {
        match self {
            BackendChild::Sidecar(c) => {
                let _ = c.kill();
            }
            BackendChild::Std(mut c) => {
                let _ = c.kill();
                let _ = c.wait();
            }
        }
        #[cfg(target_os = "windows")]
        {
            // 兜底：静默结束所有同名进程
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", "photo-backend.exe"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .creation_flags_no_window()
                .spawn();
        }
    }
}

/// 让子进程不弹黑框
trait NoWindow {
    fn creation_flags_no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn creation_flags_no_window(&mut self) -> &mut Self {
        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        self
    }
}

struct Backend(std::sync::Mutex<Option<BackendChild>>);

/// 后端 exe 查找：**优先 exe 自身所在目录**。
/// 双击启动时 current_dir 往往是 C:\Windows\System32 之类，
/// 之前用 cwd 找后端所以找不到，前端就报"后端未启动"。
/// 后端由 Tauri sidecar 机制内嵌在主程序里，
/// 运行时会自动解压到应用数据目录，路径由 shell 插件解析。
/// 这里保留一个兜底：万一 sidecar 不可用，再按文件系统找。
fn find_backend_fallback() -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(d) = exe.parent() {
            dirs.push(d.to_path_buf());
            dirs.push(d.join("binaries"));
            dirs.push(d.join("_up_"));
            if let Some(p) = d.parent() {
                dirs.push(p.to_path_buf());
            }
        }
    }
    for d in dirs {
        let p = d.join("photo-backend.exe");
        if p.exists() {
            return Some(p);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn bind_to_parent(child: &Child) {
    // 用 Job Object 把后端绑定到本进程：主进程退出时系统连带终止后端。
    use std::os::windows::io::AsRawHandle;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attrs: *mut u8, name: *const u16) -> *mut u8;
        fn SetInformationJobObject(job: *mut u8, class: u32, info: *const u8, len: u32) -> i32;
        fn AssignProcessToJobObject(job: *mut u8, process: *mut u8) -> i32;
    }

    let handle = child.as_raw_handle() as *mut u8;
    unsafe {
        let job = CreateJobObjectW(std::ptr::null_mut(), std::ptr::null());
        if !job.is_null() {
            let mut info = [0u8; 144];
            let flags: u32 = 0x2000; // KILL_ON_JOB_CLOSE
            info[16..20].copy_from_slice(&flags.to_le_bytes());
            let _ = SetInformationJobObject(job, 9, info.as_ptr(), info.len() as u32);
            let _ = AssignProcessToJobObject(job, handle);
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn bind_to_parent(_child: &Child) {}

#[tauri::command]
fn open_path(path: String) -> Result<(), String> {
    // 用系统默认程序打开文件或文件夹
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
/// Tauri 的 dialog 插件在 Windows 上默认走旧式 SHBrowseForFolder，
/// 外观是老式树形框；这里改用 COM 的 IFileOpenDialog + FOS_PICKFOLDERS，
/// 得到和「另存为」完全一致的现代资源管理器界面（带地址栏、搜索、侧边栏）。
#[tauri::command]
async fn pick_folder() -> Option<String> {
    #[cfg(target_os = "windows")]
    {
        use windows::core::{Interface, PCWSTR};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
            COINIT_APARTMENTTHREADED,
        };
        use windows::Win32::UI::Shell::{
            FileOpenDialog, IFileOpenDialog, IShellItem, FOS_FORCEFILESYSTEM,
            FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH,
        };

        unsafe {
            let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            // RPC_E_CHANGED_MODE 表示线程已初始化成别的套间模型，可继续用
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

                // 显示模态对话框
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
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .invoke_handler(tauri::generate_handler![open_path, pick_folder])
        .setup(|app| {
            // 依次尝试：sidecar（安装版/内嵌）→ 同目录 exe（绿色版）→ 二进制子目录
            use tauri_plugin_shell::ShellExt;

            let mut started = false;

            // 1) sidecar
            if let Ok(cmd) = app.shell().sidecar("photo-backend") {
                if let Ok((_rx, child)) = cmd.spawn() {
                    app.manage(Backend(std::sync::Mutex::new(
                        Some(BackendChild::Sidecar(child)))));
                    started = true;
                }
            }

            // 2) 回退：exe 同目录的 photo-backend.exe
            if !started {
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(dir) = exe.parent() {
                        let candidates = [
                            dir.join("backend").join("photo-backend.exe"),
                            dir.join("photo-backend.exe"),
                            dir.join("binaries").join("photo-backend.exe"),
                            dir.join("_up_").join("photo-backend.exe"),
                        ];
                        for bp in candidates {
                            if bp.exists() {
                                match std::process::Command::new(&bp)
                                    .current_dir(dir)
                                    .spawn()
                                {
                                    Ok(child) => {
                                        app.manage(Backend(std::sync::Mutex::new(
                                            Some(BackendChild::Std(child)))));
                                        started = true;
                                        break;
                                    }
                                    Err(_) => continue,
                                }
                            }
                        }
                    }
                }
            }

            if !started {
                eprintln!("[opo] 后端未能启动：sidecar 与本地 exe 都不可用");
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // CloseRequested 在用户点 ✕ 时最先触发，比 Destroyed 更可靠
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if let Some(state) = window.try_state::<Backend>() {
                    if let Ok(mut guard) = state.0.lock() {
                        if let Some(child) = guard.take() {
                            child.kill();
                        }
                    }
                }
            }
            if let tauri::WindowEvent::Destroyed = event {
                if let Some(state) = window.try_state::<Backend>() {
                    if let Ok(mut guard) = state.0.lock() {
                        if let Some(child) = guard.take() {
                            child.kill();
                        }
                    }
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            // RunEvent::Exit 在主进程真正结束前触发，这是最后可靠的清理点
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<Backend>() {
                    if let Ok(mut guard) = state.0.lock() {
                        if let Some(child) = guard.take() {
                            child.kill();
                        }
                    }
                }
            }
        });
}