//! 原生通知。通知只是反馈，任何失败都不影响已经完成的复制。

/// 必须在任何通知之前调用，用已知的 bundle id 直接占位。
///
/// notify-rust 的 macOS 后端没被显式设置时会自己跑一段 AppleScript
/// `get id of application "use_default"` 去反查宿主 bundle id；那句在无人值守的
/// 机器上解析不出来，AppleScript 组件会弹出「选择应用」的模态询问，把调用它的
/// 主线程永久挂住（第一次复制之后整个应用就不再响应任何触发）。这里提前把
/// Once 占掉，既避开挂死，也让通知正确归属到本应用而不是终端。
#[cfg(target_os = "macos")]
pub fn set_bundle(bundle_id: &str) {
    if let Err(e) = mac_notification_sys::set_application(bundle_id) {
        eprintln!("通知归属设置失败（不影响复制）：{e}");
    }
}

fn build(title: &str, body: &str) -> notify_rust::Notification {
    let mut n = notify_rust::Notification::new();
    n.appname("CopyPath").summary(title);
    if !body.is_empty() {
        n.body(body);
    }
    n
}

/// 通知走独立线程：后端一旦表现异常（本机上它就曾把主线程挂死），也不能拖住托盘和菜单。
pub fn show(title: &str, body: &str) {
    let title = title.to_string();
    let body = body.to_string();
    std::thread::spawn(move || {
        if let Err(e) = build(&title, &body).show() {
            eprintln!("通知发送失败（不影响已复制的内容）：{e}");
        }
    });
}

/// 同 `show`，但阻塞到后端返回。短命的工具栏 helper 必须用它：
/// 异步线程还没发出去进程就退出，通知会整条丢失。
pub fn show_and_wait(title: &str, body: &str) {
    if let Err(e) = build(title, body).show() {
        eprintln!("通知发送失败（不影响已复制的内容）：{e}");
    }
}

pub fn info(title: &str, body: &str) {
    show(title, body);
}

pub fn error(title: &str, body: &str) {
    show(title, body);
}
