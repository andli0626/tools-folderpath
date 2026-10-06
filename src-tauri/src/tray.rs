//! macOS 菜单栏托盘：点击图标展开菜单。

use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt as AutostartExt;

use crate::finder::Mode;
use crate::history;
use crate::notify;
use crate::AppState;

pub const TRAY_ID: &str = "main";

/// 托盘图必须是纯黑字形 + 透明背景，配合 icon_as_template 才能由系统在深浅色
/// 菜单栏下自动反色。全彩图标在其中一种外观下会看不清。
const TRAY_ICON: tauri::image::Image<'static> =
    tauri::include_image!("../assets/tray-template@2x.png");

pub fn setup(app: &AppHandle) -> tauri::Result<()> {
    let fetch_folder = MenuItem::with_id(app, "fetch-folder", "立即获取文件夹路径", true, None::<&str>)?;
    let fetch_item = MenuItem::with_id(app, "fetch-item", "复制选中项自身路径", true, None::<&str>)?;
    let history_sub = Submenu::with_id(app, "history", "最近路径", true)?;
    let autostart = CheckMenuItem::with_id(app, "autostart", "开机自启", true, false, None::<&str>)?;
    // 初始勾选来自落盘设置；面板此刻还没建，不能拿 is_visible() 当真值。
    #[cfg(target_os = "macos")]
    let float_btn = CheckMenuItem::with_id(
        app,
        "float-button",
        "悬浮按钮",
        true,
        app.state::<AppState>().float_settings.lock().unwrap().enabled,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "退出 CopyPath", true, None::<&str>)?;
    // 版本号只读，禁用态；真源是 tauri.conf.json，别在这里写死。
    let version = MenuItem::with_id(
        app,
        "version",
        format!("版本 {}", app.package_info().version),
        false,
        None::<&str>,
    )?;

    {
        let state = app.state::<AppState>();
        *state.history_menu.lock().unwrap() = Some(history_sub.clone());
        *state.autostart_item.lock().unwrap() = Some(autostart.clone());
        // 赋值表达式语句上直接挂 #[cfg] 会按「表达式属性」解析（E0658），包一层块。
        #[cfg(target_os = "macos")]
        {
            *state.float_item.lock().unwrap() = Some(float_btn.clone());
        }
    }
    history::refresh_menu(app);

    // 菜单项列表用 Vec 拼：数组字面量元素上挂不了 #[cfg]（expression attributes 未稳定）。
    let sep_after_actions = PredefinedMenuItem::separator(app)?;
    let sep_before_toggles = PredefinedMenuItem::separator(app)?;
    let mut items: Vec<&dyn tauri::menu::IsMenuItem<tauri::Wry>> = vec![
        &fetch_folder,
        &fetch_item,
        &sep_after_actions,
        &history_sub,
        &sep_before_toggles,
    ];
    #[cfg(target_os = "macos")]
    {
        items.push(&float_btn);
    }
    // 逐个 push：数组字面量会把元素类型统一成第一项的具体类型，混装过不了推断。
    items.push(&autostart);
    items.push(&quit);
    items.push(&version);
    let menu = Menu::with_items(app, &items)?;

    if app.autolaunch().is_enabled().unwrap_or(false) {
        let _ = autostart.set_checked(true);
    }

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(TRAY_ICON)
        .icon_as_template(true)
        .tooltip("CopyPath · 点击展开菜单，全局快捷键 ⌃⌥⌘P 直接复制")
        // 挂了菜单的 NSStatusItem 会在 mouseDown 里自己把菜单弹出来，tray-icon 铺在
        // 图标上的事件覆盖层因此永远收不到左键（实测右键收得到、左键收不到）。
        // 所以"左键直接取路径"与"菜单"在同一个图标上不可兼得，一键交给全局快捷键。
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "fetch-folder" => crate::fetch_and_copy(app, Mode::FolderPath),
            "fetch-item" => crate::fetch_and_copy(app, Mode::ItemPath),
            #[cfg(target_os = "macos")]
            "float-button" => crate::float_button::toggle(app),
            "quit" => app.exit(0),
            "autostart" => toggle_autostart(app),
            "history:clear" => history::clear(app),
            id => {
                if let Some(rest) = id.strip_prefix("history:") {
                    if let Ok(index) = rest.parse::<usize>() {
                        history::copy_index(app, index);
                    }
                }
            }
        })
        .build(app)?;
    Ok(())
}

fn toggle_autostart(app: &AppHandle) {
    // muda 在发出菜单事件前已经自己把勾翻过一次（muda-0.19.3 macos/mod.rs:1125），
    // 所以这里既不能读 is_checked()（会双重取反），也不能再 set_checked()。
    // 真实状态以 LaunchAgent plist 是否存在为准。
    let want = !app.autolaunch().is_enabled().unwrap_or(false);
    let result = if want {
        app.autolaunch().enable()
    } else {
        app.autolaunch().disable()
    };
    if let Err(e) = result {
        let state = app.state::<AppState>();
        if let Some(item) = state.autostart_item.lock().unwrap().as_ref() {
            let _ = item.set_checked(!want);
        }
        notify::error("开机自启设置失败", &e.to_string());
        return;
    }
    notify::info(
        if want { "已开启开机自启" } else { "已关闭开机自启" },
        "登录项指向当前 .app 所在位置",
    );
}
