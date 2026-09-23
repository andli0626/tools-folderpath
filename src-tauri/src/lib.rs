mod clipboard;
mod finder;
mod history;
pub mod notify;
mod tray;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use tauri::menu::{CheckMenuItem, Submenu};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{Builder as ShortcutBuilder, GlobalShortcutExt, ShortcutState};

/// 与 Finder 自带的「将文件拷贝为路径名」⌥⌘C 和「拷贝名称」⌘⇧C 错开，
/// 免得在访达里按下像是把系统功能顶掉了。
const SHORTCUT: &str = "Command+Control+Alt+P";

pub struct AppState {
    /// 重入保护：解析要跨进程调 osascript，连点时后一次直接丢弃。
    busy: AtomicBool,
    history: Mutex<Vec<PathBuf>>,
    history_menu: Mutex<Option<Submenu<Wry>>>,
    autostart_item: Mutex<Option<CheckMenuItem<Wry>>>,
    config_dir: PathBuf,
}

/// 两个触发口（全局快捷键、托盘菜单两项）都收敛到这里。
pub fn fetch_and_copy(app: &AppHandle, mode: finder::Mode) {
    let state = app.state::<AppState>();
    if state.busy.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    // 访达被卡住时 osascript 可能不返回，不能占用主线程。
    std::thread::spawn(move || {
        let result = finder::resolve(mode);
        let target = app.clone();
        // 收尾回主线程：改 NSMenu 结构和发通知都走主线程最稳。
        if let Err(e) = app.run_on_main_thread(move || finish(&target, result)) {
            eprintln!("无法调度回主线程：{e}");
            app.state::<AppState>().busy.store(false, Ordering::SeqCst);
        }
    });
}

fn finish(app: &AppHandle, result: Result<Vec<PathBuf>, finder::FinderError>) {
    match result {
        Ok(paths) => match copy_and_notify(&paths, false) {
            Ok(()) => history::push(app, &paths),
            Err(e) => notify::error("写入剪贴板失败", &e),
        },
        Err(e) => notify::error("获取失败", &e.to_string()),
    }
    app.state::<AppState>().busy.store(false, Ordering::SeqCst);
}

/// 写剪贴板并发通知反馈。快捷键、托盘菜单与工具栏 helper 共用这一份语义。
/// `wait_notify` 为 true 时同步等通知发出——短命进程不等就会整条丢失。
fn copy_and_notify(paths: &[PathBuf], wait_notify: bool) -> Result<(), String> {
    let text = paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join("\n");
    clipboard::write_text(&text)?;
    let title = if paths.len() == 1 {
        "已复制路径".to_string()
    } else {
        format!("已复制 {} 个路径", paths.len())
    };
    if wait_notify {
        notify::show_and_wait(&title, &text);
    } else {
        notify::info(&title, &text);
    }
    Ok(())
}

/// `CopyPath Toolbar.app` 的唯一动作：解析当前文件夹 → 复制 → 通知，随即退出。
/// 不进最近历史（那是托盘应用的运行时状态），错误同时打 stderr 便于排查。
pub fn toolbar_copy() -> i32 {
    match finder::resolve(finder::Mode::FolderPath) {
        Ok(paths) => match copy_and_notify(&paths, true) {
            Ok(()) => 0,
            Err(e) => {
                notify::show_and_wait("写入剪贴板失败", &e);
                eprintln!("{e}");
                1
            }
        },
        Err(e) => {
            notify::show_and_wait("获取失败", &e.to_string());
            eprintln!("{e}");
            1
        }
    }
}

/// 供 `--probe` 子命令使用：不启界面，直接把解析结果打到 stdout，便于逐项验证。
pub fn probe(arg: Option<&str>) -> i32 {
    let mode = match arg {
        Some("item") => finder::Mode::ItemPath,
        _ => finder::Mode::FolderPath,
    };
    match finder::resolve(mode) {
        Ok(paths) => {
            for p in &paths {
                println!("{}", p.display());
            }
            eprintln!("(共 {} 条，模式 {:?})", paths.len(), mode);
            0
        }
        Err(e) => {
            eprintln!("{e}");
            1
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            ShortcutBuilder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        fetch_and_copy(app, finder::Mode::FolderPath);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let handle = app.handle();

            // 打包后靠 Info.plist 的 LSUIElement 隐藏 Dock；`cargo tauri dev` 跑的是
            // 裸二进制、不在 .app 里，那条 key 不生效，需要这句运行时兜底。
            #[cfg(target_os = "macos")]
            {
                let _ = handle.set_dock_visibility(false);
            }

            let config_dir = handle.path().app_config_dir()?;
            #[cfg(target_os = "macos")]
            notify::set_bundle(&handle.config().identifier.clone());
            handle.manage(AppState {
                busy: AtomicBool::new(false),
                history: Mutex::new(history::load(&config_dir)),
                history_menu: Mutex::new(None),
                autostart_item: Mutex::new(None),
                config_dir,
            });

            tray::setup(handle)?;

            // 注册失败（被别的应用占用）只降级，托盘两条入口仍然可用。
            if let Err(e) = handle.global_shortcut().register(SHORTCUT) {
                eprintln!("全局快捷键 {SHORTCUT} 注册失败（托盘与菜单仍可用）：{e}");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("运行 CopyPath 失败");
}
