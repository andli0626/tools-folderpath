//! 全局悬浮按钮：一枚常驻屏幕、可拖动的胶囊小按钮（非激活 NSPanel）。
//!
//! 关键约束是**点击不得改变前置应用**：NSPanel 挂上 `NonactivatingPanel` 掩码后，
//! 点击它时 Finder 仍保持前置，`finder::resolve` 读到的「选中项 / 前置窗口」与
//! 托盘菜单语义完全一致。这也是它不走 Tauri 窗口的原因——NSWindow 点击必然
//! 激活自身（会抢走 Finder 前置身份），且为一个按钮拉起 WebView 进程不值得。
//!
//! 交互：左键 = 复制选中项自身路径（与托盘同语义、进最近路径）；按住拖动 =
//! 移动位置（落盘记忆）；右键 = 两种语义切换 + 隐藏入口；托盘菜单「悬浮按钮」
//! = 显隐开关。开关与位置存在 `float-button.json`。

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{
    define_class, msg_send, sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSFont, NSImage, NSImageView,
    NSLineCapStyle, NSFloatingWindowLevel, NSMenu, NSMenuItem, NSPanel, NSScreen, NSTextField,
    NSView, NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{ns_string, NSArray, NSString, NSPoint, NSRect, NSSize};
use tauri::{AppHandle, Manager};

use crate::finder::Mode;
use crate::AppState;

/// 布局常量（点）。圆角半径取高度一半，胶囊造型。
const HEIGHT: f64 = 34.0;
const PADDING: f64 = 14.0;
const ICON: f64 = 15.0;
const GAP: f64 = 5.0;
/// 右侧 × 关闭区：间隙 + 点击区 + 右缘留白。整个右区都在关闭热区内。
const CLOSE_GAP: f64 = 5.0;
const CLOSE_ZONE: f64 = 16.0;
const CLOSE_PAD_R: f64 = 2.0;
/// 位移超过这个值才算拖动，否则松手仍按点击处理（容忍手抖）。
const DRAG_THRESHOLD: f64 = 4.0;
/// 默认停靠位置离屏幕右缘的距离。
const EDGE_MARGIN: f64 = 16.0;

/// 悬浮按钮的运行时设置：显隐开关 + 记住的位置（窗口左上角，全局屏幕坐标）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settings {
    pub enabled: bool,
    pub pos: Option<(f64, f64)>,
}

/// 包一层对象，将来加别的设置项不必迁移文件格式。
#[derive(serde::Serialize, serde::Deserialize)]
struct SettingsFile {
    enabled: bool,
    pos: Option<(f64, f64)>,
}

fn file_path(config_dir: &Path) -> PathBuf {
    config_dir.join("float-button.json")
}

/// 无配置文件（首次使用）默认开启；文件损坏同样回落默认，绝不因此挡启动。
pub fn load(config_dir: &Path) -> Settings {
    std::fs::read(file_path(config_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<SettingsFile>(&bytes).ok())
        .map(|f| Settings {
            enabled: f.enabled,
            pos: f.pos,
        })
        .unwrap_or(Settings {
            enabled: true,
            pos: None,
        })
}

fn save(config_dir: &Path, settings: &Settings) {
    if std::fs::create_dir_all(config_dir).is_err() {
        return;
    }
    let json = serde_json::to_vec_pretty(&SettingsFile {
        enabled: settings.enabled,
        pos: settings.pos,
    })
    .unwrap_or_default();
    let _ = std::fs::write(file_path(config_dir), json);
}

thread_local! {
    /// NSPanel 非 Send，而托盘、菜单与鼠标事件全部发生在主线程，thread_local 即可持有。
    static PANEL: RefCell<Option<Retained<NSPanel>>> = const { RefCell::new(None) };
}

pub fn is_visible() -> bool {
    if MainThreadMarker::new().is_none() {
        return false;
    }
    PANEL.with_borrow(|panel| panel.as_ref().is_some_and(|w| w.isVisible()))
}

/// 启动时调用：上次开启着就直接显示。
pub fn init(app: &AppHandle) {
    if app.state::<AppState>().float_settings.lock().unwrap().enabled {
        show(app);
    }
}

/// 托盘「悬浮按钮」开关：以面板实际可见性为准翻转。
pub fn toggle(app: &AppHandle) {
    if is_visible() {
        persist_settings(app, |s| s.enabled = false);
        hide();
    } else {
        persist_settings(app, |s| s.enabled = true);
        show(app);
    }
    sync_tray_item(app);
}

/// 右键菜单「隐藏悬浮按钮」走的入口，与托盘开关共用同一份落盘状态。
pub fn hide_and_disable(app: &AppHandle) {
    persist_settings(app, |s| s.enabled = false);
    hide();
    sync_tray_item(app);
}

pub fn show(app: &AppHandle) {
    let Some(mtm) = MainThreadMarker::new() else {
        eprintln!("悬浮按钮必须在主线程显示，本次忽略");
        return;
    };
    PANEL.with_borrow_mut(|slot| {
        match slot {
            // orderFrontRegardless：显示但不激活应用（不抢 Finder 前置身份）。
            Some(panel) => panel.orderFrontRegardless(),
            None => {
                let panel = create_panel(app, mtm);
                panel.orderFrontRegardless();
                *slot = Some(panel);
            }
        }
    });
}

fn hide() {
    PANEL.with_borrow_mut(|slot| {
        if let Some(panel) = slot {
            panel.orderOut(None);
        }
    });
}

/// 托盘勾选状态以面板实际可见性为准（muda 发事件前会自己翻一次勾，不能信它）。
fn sync_tray_item(app: &AppHandle) {
    let state = app.state::<AppState>();
    let float_item = state.float_item.lock().unwrap();
    if let Some(item) = float_item.as_ref() {
        let _ = item.set_checked(is_visible());
    }
}

fn persist_settings(app: &AppHandle, f: impl FnOnce(&mut Settings)) {
    let (config_dir, snapshot) = {
        let state = app.state::<AppState>();
        let mut settings = state.float_settings.lock().unwrap();
        f(&mut settings);
        (state.config_dir.clone(), *settings)
    };
    save(&config_dir, &snapshot);
}

/// 拖动结束后记下窗口左上角。NSWindow 的 origin 是左下角，换算成左上角落盘，
/// 与屏幕高度无关，重读时再转回来。
fn persist_position(app: &AppHandle, frame: NSRect) {
    persist_settings(app, |s| {
        s.pos = Some((frame.origin.x, frame.origin.y + frame.size.height));
    });
}

fn create_panel(app: &AppHandle, mtm: MainThreadMarker) -> Retained<NSPanel> {
    // 先建文字视图量宽度，面板宽度按实际渲染收口（写死会跟系统字体度量打架）。
    let label = NSTextField::labelWithString(ns_string!("复制路径"), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(12.5)));
    label.setTextColor(Some(&NSColor::whiteColor()));
    label.sizeToFit();
    let label_size = label.frame().size;
    let size = NSSize::new(
        (PADDING + ICON + GAP + label_size.width + CLOSE_GAP + CLOSE_ZONE + CLOSE_PAD_R).ceil(),
        HEIGHT,
    );

    let saved = app.state::<AppState>().float_settings.lock().unwrap().pos;
    let screens = visible_frames(mtm);
    let frame = saved
        .and_then(|top_left| frame_at(top_left, size, &screens))
        .unwrap_or_else(|| NSRect::new(default_origin(size, mtm), size));

    let panel = NSPanel::initWithContentRect_styleMask_backing_defer(
        NSPanel::alloc(mtm),
        frame,
        NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
        NSBackingStoreType::Buffered,
        false,
    );
    panel.setLevel(NSFloatingWindowLevel);
    panel.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    // NSPanel 默认随应用失活而隐藏，悬浮按钮必须常驻所有空间与前台应用。
    panel.setHidesOnDeactivate(false);
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setAcceptsMouseMovedEvents(true);

    let view = FloatButtonView::new_with_frame(app.clone(), frame, mtm);

    if let Some(icon) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        ns_string!("doc.on.doc"),
        None,
    ) {
        icon.setSize(NSSize::new(ICON, ICON));
        let icon_view = NSImageView::imageViewWithImage(&icon, mtm);
        icon_view.setFrameOrigin(NSPoint::new(PADDING, (HEIGHT - ICON) / 2.0));
        icon_view.setContentTintColor(Some(&NSColor::whiteColor()));
        view.addSubview(&icon_view);
    }

    label.setFrameOrigin(NSPoint::new(
        PADDING + ICON + GAP,
        (HEIGHT - label_size.height) / 2.0,
    ));
    view.addSubview(&label);
    view
        .as_super()
        .setToolTip(Some(ns_string!("CopyPath · 左键复制选中项自身路径；× 关闭悬浮按钮；拖动移动；右键更多")));

    panel.setContentView(Some(view.as_super()));
    panel
}

fn visible_frames(mtm: MainThreadMarker) -> Vec<NSRect> {
    let screens: Retained<NSArray<NSScreen>> = NSScreen::screens(mtm);
    (0..screens.len())
        .map(|i| screens.objectAtIndex(i).visibleFrame())
        .collect()
}

/// 把记住的左上角坐标还原成窗口 frame；不与任何屏幕相交则返回 None（回落默认位置，
/// 覆盖外接屏被拔掉、分辨率变了等场景）。
fn frame_at(top_left: (f64, f64), size: NSSize, screens: &[NSRect]) -> Option<NSRect> {
    let frame = NSRect::new(
        NSPoint::new(top_left.0, top_left.1 - size.height),
        size,
    );
    screens
        .iter()
        .copied()
        .any(|screen| intersects(frame, screen))
        .then_some(frame)
}

fn intersects(a: NSRect, b: NSRect) -> bool {
    a.origin.x < b.origin.x + b.size.width
        && b.origin.x < a.origin.x + a.size.width
        && a.origin.y < b.origin.y + b.size.height
        && b.origin.y < a.origin.y + a.size.height
}

fn default_origin(size: NSSize, mtm: MainThreadMarker) -> NSPoint {
    match NSScreen::mainScreen(mtm) {
        Some(screen) => {
            let vf = screen.visibleFrame();
            NSPoint::new(
                vf.origin.x + vf.size.width - size.width - EDGE_MARGIN,
                vf.origin.y + (vf.size.height - size.height) / 2.0,
            )
        }
        None => NSPoint::new(100.0, 100.0),
    }
}

struct ButtonIvars {
    app: AppHandle,
    /// 拖动标记：mouseUp 据此区分「点击」与「拖完松手」。
    dragging: Cell<bool>,
    /// mouseDown 落在右侧 × 关闭区：拖动被禁用，松手即关闭。
    down_in_close: Cell<bool>,
    /// mouseDown 时刻的指针位置（窗口坐标系），拖动阈值与位移换算的基准。
    start: Cell<NSPoint>,
    /// mouseDown 时刻的窗口原点（屏幕坐标），拖动时在其上加位移。
    base_origin: Cell<NSPoint>,
}

define_class!(
    /// 按钮整块视图：接管全部鼠标事件，子视图（图标/文字）纯展示。
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "CopyPathFloatButtonView"]
    #[ivars = ButtonIvars]
    struct FloatButtonView;

    impl FloatButtonView {
        // 返回保留对象的方法要用 method_id 形式（define_class! 不支持 method_family）。
        #[unsafe(method_id(hitTest:))]
        unsafe fn hit_test(&self, _point: NSPoint) -> Option<Retained<NSView>> {
            // 子视图不参与命中，整块都算自己——图标和文字上按下也要能点击/拖动。
            let retained = unsafe { Retained::retain(self as *const Self as *mut Self) };
            retained.map(|view| view.into_super())
        }

        #[unsafe(method(drawRect:))]
        unsafe fn draw_rect(&self, _dirty: NSRect) {
            let bounds = self.as_super().bounds();
            let radius = bounds.size.height / 2.0;
            let path =
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(bounds, radius, radius);
            NSColor::colorWithWhite_alpha(0.10, 0.86).setFill();
            path.fill();
            NSColor::colorWithWhite_alpha(1.0, 0.22).setStroke();
            path.setLineWidth(1.0);
            path.stroke();

            // × 关闭钮：常驻胶囊右侧，点一下等同右键菜单的「隐藏悬浮按钮」。
            let cx = bounds.size.width - CLOSE_PAD_R - CLOSE_ZONE / 2.0;
            let cy = bounds.size.height / 2.0;
            let r = 4.0;
            let cross = NSBezierPath::bezierPath();
            cross.setLineWidth(1.6);
            cross.setLineCapStyle(NSLineCapStyle::Round);
            cross.moveToPoint(NSPoint::new(cx - r, cy + r));
            cross.lineToPoint(NSPoint::new(cx + r, cy - r));
            cross.moveToPoint(NSPoint::new(cx - r, cy - r));
            cross.lineToPoint(NSPoint::new(cx + r, cy + r));
            NSColor::colorWithWhite_alpha(1.0, 0.55).setStroke();
            cross.stroke();
        }

        #[unsafe(method(mouseDown:))]
        unsafe fn mouse_down(&self, event: &NSEvent) {
            let ivars = self.ivars();
            ivars.dragging.set(false);
            let point = event.locationInWindow();
            ivars.start.set(point);
            // 按在 × 区：不启动拖动，松手即关闭。
            let width = self.as_super().bounds().size.width;
            ivars
                .down_in_close
                .set(point.x >= width - CLOSE_ZONE - CLOSE_PAD_R);
            if let Some(window) = self.as_super().window() {
                ivars.base_origin.set(window.frame().origin);
            }
        }

        #[unsafe(method(mouseDragged:))]
        unsafe fn mouse_dragged(&self, event: &NSEvent) {
            let ivars = self.ivars();
            if ivars.down_in_close.get() {
                return;
            }
            let start = ivars.start.get();
            let current = event.locationInWindow();
            let dx = current.x - start.x;
            let dy = current.y - start.y;
            if !ivars.dragging.get() && (dx * dx + dy * dy).sqrt() > DRAG_THRESHOLD {
                ivars.dragging.set(true);
            }
            if ivars.dragging.get() {
                // 直接跟手挪窗口：drag 事件足够密，不必用 performWindowDrag 的模态循环，
                // 松手时 frameOrigin 就是最终位置，持久化无须额外等待。
                let base = ivars.base_origin.get();
                if let Some(window) = self.as_super().window() {
                    window.setFrameOrigin(NSPoint::new(base.x + dx, base.y + dy));
                }
            }
        }

        #[unsafe(method(mouseUp:))]
        unsafe fn mouse_up(&self, _event: &NSEvent) {
            let ivars = self.ivars();
            if ivars.dragging.get() {
                ivars.dragging.set(false);
                if let Some(window) = self.as_super().window() {
                    persist_position(&ivars.app, window.frame());
                }
                return;
            }
            if ivars.down_in_close.get() {
                // 与右键菜单「隐藏悬浮按钮」同一条落盘关闭路径。
                hide_and_disable(&ivars.app);
                return;
            }
            // 与托盘「复制选中项自身路径」同一条代码路径（含最近历史与重入保护）。
            crate::fetch_and_copy(&ivars.app, Mode::ItemPath);
        }

        #[unsafe(method(rightMouseDown:))]
        unsafe fn right_mouse_down(&self, event: &NSEvent) {
            let menu = self.build_menu();
            // 菜单跟踪自己跑模态循环，期间不激活应用，Finder 仍是前置。
            menu.popUpMenuPositioningItem_atLocation_inView(
                None,
                event.locationInWindow(),
                Some(self.as_super()),
            );
        }

        #[unsafe(method(copyItemPath:))]
        unsafe fn copy_item_path(&self, _sender: Option<&AnyObject>) {
            crate::fetch_and_copy(&self.ivars().app, Mode::ItemPath);
        }

        #[unsafe(method(copyFolderPath:))]
        unsafe fn copy_folder_path(&self, _sender: Option<&AnyObject>) {
            crate::fetch_and_copy(&self.ivars().app, Mode::FolderPath);
        }

        #[unsafe(method(hidePanel:))]
        unsafe fn hide_panel(&self, _sender: Option<&AnyObject>) {
            hide_and_disable(&self.ivars().app);
        }
    }
);

impl FloatButtonView {
    fn new_with_frame(app: AppHandle, frame: NSRect, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ButtonIvars {
            app,
            dragging: Cell::new(false),
            down_in_close: Cell::new(false),
            start: Cell::new(NSPoint::new(0.0, 0.0)),
            base_origin: Cell::new(NSPoint::new(0.0, 0.0)),
        });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// 右键小菜单：两种复制语义 + 隐藏入口。
    fn build_menu(&self) -> Retained<NSMenu> {
        let mtm = MainThreadMarker::new().expect("右键菜单必须在主线程构建");
        let menu = NSMenu::new(mtm);
        // 自定义 target 不实现菜单校验协议，关掉自动启用免得菜单项被置灰。
        menu.setAutoenablesItems(false);
        let this: Retained<AnyObject> = unsafe {
            // 纯上溯到根类，任何 ObjC 对象都是合法的 AnyObject。
            Retained::cast_unchecked(
                Retained::retain(self as *const Self as *mut Self).expect("self 必须存活"),
            )
        };

        let add_item = |title: &str, action: objc2::runtime::Sel| {
            let item = NSMenuItem::new(mtm);
            item.setTitle(&NSString::from_str(title));
            unsafe {
                item.setTarget(Some(&this));
                item.setAction(Some(action));
            }
            menu.addItem(&item);
        };
        add_item("复制选中项自身路径", sel!(copyItemPath:));
        add_item("立即获取文件夹路径", sel!(copyFolderPath:));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_item("隐藏悬浮按钮", sel!(hidePanel:));
        menu
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn settings_file_roundtrips() {
        let dir = std::env::temp_dir().join(format!("folderpath-float-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        save(&dir, &Settings { enabled: false, pos: Some((120.5, 200.0)) });
        assert_eq!(
            load(&dir),
            Settings { enabled: false, pos: Some((120.5, 200.0)) }
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_or_garbage_file_defaults_to_enabled() {
        let dir = std::env::temp_dir().join(format!("folderpath-float-none-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(load(&dir), Settings { enabled: true, pos: None });

        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(file_path(&dir), b"not json").unwrap();
        assert_eq!(load(&dir), Settings { enabled: true, pos: None });
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn frame_at_converts_top_left_back_to_bottom_left_origin() {
        let screens = [screen(0.0, 0.0, 1440.0, 900.0)];
        let frame = frame_at((100.0, 134.0), NSSize::new(100.0, 34.0), &screens).unwrap();
        assert_eq!(frame.origin, NSPoint::new(100.0, 100.0));
        assert_eq!(frame.size, NSSize::new(100.0, 34.0));
    }

    #[test]
    fn frame_at_rejects_positions_off_every_screen() {
        let screens = [screen(0.0, 0.0, 1440.0, 900.0)];
        assert!(frame_at((9000.0, 9000.0), NSSize::new(100.0, 34.0), &screens).is_none());
        // 只压着屏幕边缘 1px 不算在屏上，避免还原出一条贴缝的不可见窗口。
        assert!(frame_at((1440.0, 900.0), NSSize::new(100.0, 34.0), &screens).is_none());
    }

    #[test]
    fn frame_at_accepts_secondary_screen() {
        let screens = [
            screen(0.0, 0.0, 1440.0, 900.0),
            screen(1440.0, 0.0, 1920.0, 1080.0),
        ];
        let frame = frame_at((1500.0, 534.0), NSSize::new(100.0, 34.0), &screens).unwrap();
        assert_eq!(frame.origin, NSPoint::new(1500.0, 500.0));
    }
}
