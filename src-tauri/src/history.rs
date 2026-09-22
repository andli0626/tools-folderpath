//! 最近路径：内存列表 + JSON 落盘 + 托盘子菜单刷新。

use std::path::{Path, PathBuf};

use tauri::menu::{MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Manager};

use crate::clipboard;
use crate::notify;
use crate::AppState;

pub const HISTORY_MAX: usize = 10;

/// 包一层对象，将来加别的设置项不必迁移文件格式。
#[derive(Default, serde::Serialize, serde::Deserialize)]
struct HistoryFile {
    items: Vec<PathBuf>,
}

fn file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("history.json")
}

pub fn load(config_dir: &Path) -> Vec<PathBuf> {
    std::fs::read(file_path(config_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<HistoryFile>(&bytes).ok())
        .map(|f| {
            f.items
                .into_iter()
                .filter(|p| p.is_absolute())
                .take(HISTORY_MAX)
                .collect()
        })
        .unwrap_or_default()
}

fn save(config_dir: &Path, items: &[PathBuf]) {
    if std::fs::create_dir_all(config_dir).is_err() {
        return;
    }
    let json = serde_json::to_vec_pretty(&HistoryFile {
        items: items.to_vec(),
    })
    .unwrap_or_default();
    let _ = std::fs::write(file_path(config_dir), json);
}

/// 新→旧，去重后落盘并刷新菜单。必须在主线程调用。
pub fn push(app: &AppHandle, paths: &[PathBuf]) {
    if paths.is_empty() {
        return;
    }
    let (config_dir, snapshot) = {
        let state = app.state::<AppState>();
        let snapshot = {
            let mut h = state.history.lock().unwrap();
            for p in paths.iter().rev() {
                h.retain(|x| x != p);
                h.insert(0, p.clone());
            }
            h.truncate(HISTORY_MAX);
            h.clone()
        };
        (state.config_dir.clone(), snapshot)
    };

    save(&config_dir, &snapshot);
    refresh_menu(app);
}

/// 清空必须同时落盘，否则重启后旧历史又回来了。
pub fn clear(app: &AppHandle) {
    let config_dir = {
        let state = app.state::<AppState>();
        state.history.lock().unwrap().clear();
        state.config_dir.clone()
    };
    save(&config_dir, &[]);
    refresh_menu(app);
}

pub fn copy_index(app: &AppHandle, index: usize) {
    let target = app
        .state::<AppState>()
        .history
        .lock()
        .unwrap()
        .get(index)
        .cloned();
    match target {
        None => notify::error("历史条目已失效", "请重新获取路径"),
        Some(p) => {
            let text = p.display().to_string();
            match clipboard::write_text(&text) {
                Ok(()) => notify::info("已复制历史路径", &text),
                Err(e) => notify::error("写入剪贴板失败", &e),
            }
        }
    }
}

/// 菜单里放完整路径太长，只留末两级。剪贴板与通知始终用完整路径。
pub fn label_for(path: &Path) -> String {
    let comps: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
            _ => None,
        })
        .collect();
    match comps.len() {
        0 => path.display().to_string(),
        n if n <= 3 => comps.join(" › "),
        n => format!("… › {}", comps[n - 2..].join(" › ")),
    }
}

/// 重建整个子菜单内容（动态项 + 分隔符 + 「清空历史」）。
/// 必须在主线程调用：构造 MenuItem 内部会走 run_main_thread!，
/// 已在主线程时该宏会内联执行，不会自死锁。
pub fn refresh_menu(app: &AppHandle) {
    let Some(sub) = app.state::<AppState>().history_menu.lock().unwrap().clone() else {
        return;
    };

    // remove_at 返回 Ok(None) 表示下标越界，据此把旧内容清空。
    while let Ok(Some(_)) = sub.remove_at(0) {}

    let handle = sub.app_handle().clone();
    let items = app.state::<AppState>().history.lock().unwrap().clone();

    if items.is_empty() {
        if let Ok(item) =
            MenuItem::with_id(&handle, "history:none", "（暂无历史）", false, None::<&str>)
        {
            let _ = sub.append(&item);
        }
    } else {
        for (i, p) in items.iter().enumerate() {
            let id = format!("history:{i}");
            if let Ok(item) = MenuItem::with_id(&handle, id, label_for(p), true, None::<&str>) {
                let _ = sub.append(&item);
            }
        }
        if let Ok(sep) = PredefinedMenuItem::separator(&handle) {
            let _ = sub.append(&sep);
        }
        if let Ok(clear) = MenuItem::with_id(&handle, "history:clear", "清空历史", true, None::<&str>)
        {
            let _ = sub.append(&clear);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_paths_keep_only_the_last_two_components() {
        let p = Path::new("/Volumes/SanDis1TB/WK-Tools/tools-folderpath");
        assert_eq!(label_for(p), "… › WK-Tools › tools-folderpath");
    }

    #[test]
    fn short_paths_are_shown_in_full() {
        assert_eq!(label_for(Path::new("/Users/lilin/Downloads")), "Users › lilin › Downloads");
        assert_eq!(label_for(Path::new("/Volumes")), "Volumes");
    }

    #[test]
    fn history_file_roundtrips_and_drops_relative_entries() {
        let dir = std::env::temp_dir().join(format!("folderpath-hist-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            file_path(&dir),
            r#"{"items":["/Volumes/A","relative/path","/Volumes/B"]}"#,
        )
        .unwrap();
        assert_eq!(load(&dir), vec![PathBuf::from("/Volumes/A"), PathBuf::from("/Volumes/B")]);

        std::fs::write(file_path(&dir), b"not json").unwrap();
        assert!(load(&dir).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
