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
    NSBackingStoreType, NSBezierPath, NSColor, NSEvent, NSFont, NSGlassEffectView,
    NSGlassEffectViewStyle, NSImage, NSImageView, NSLineCapStyle, NSFloatingWindowLevel, NSMenu,
    NSMenuItem, NSPanel, NSScreen, NSTextField, NSTrackingArea, NSTrackingAreaOptions, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{
    ns_string, NSArray, NSDate, NSNumber, NSString, NSTimer, NSPoint, NSRect, NSSize,
};
use tauri::{AppHandle, Manager};

use crate::finder::Mode;
use crate::AppState;

/// 布局常量（点）。见 docs/核心功能/悬浮按钮-双语义与视觉重构-技术方案.md §4。
const HEIGHT: f64 = 36.0;
/// 左内边距。
const PAD_L: f64 = 12.0;
const ICON: f64 = 14.0;
/// 图标与文字的间距。
const GAP: f64 = 4.0;
const FONT_SIZE: f64 = 12.0;
/// 每段文字右缘到分隔线（或下一段）的留白。
const SEG_PAD_R: f64 = 10.0;
/// 段间分隔线宽。0.5 是 macOS 分段控件的标志性取值——通高会显得像表格线。
const DIVIDER_W: f64 = 0.5;
/// 分隔线上下内缩量，故线高 = HEIGHT − 2 × DIVIDER_INSET = 16。
const DIVIDER_INSET: f64 = 10.0;
/// 圆角半径。**不是** HEIGHT/2 的全胶囊：宽度涨到 162.5 后全胶囊会变成一根
/// 「药丸」，与 macOS 原生分段控件不像。10 对齐 macOS 26+ 控件圆角量级。
const RADIUS: f64 = 10.0;
/// × 关闭区：间隙 + 点击区 + 右缘留白。整个右区都在关闭热区内。
const CLOSE_GAP: f64 = 4.0;
const CLOSE_ZONE: f64 = 18.0;
const CLOSE_PAD_R: f64 = 2.0;
/// 位移超过这个值才算拖动，否则松手仍按点击处理（容忍手抖）。
const DRAG_THRESHOLD: f64 = 4.0;
/// 默认停靠位置离屏幕右缘的距离。
const EDGE_MARGIN: f64 = 16.0;
/// 复制成功的确认态持续时长（秒）。
const FLASH_SECS: f64 = 0.7;

/// 两段文字在 12pt 系统字体下的实测宽度，供宽度公式与命中区共用同一套数，
/// 保证「算出来的宽」与「画出来的分区」不会漂移。
const LABEL_FOLDER_W: f64 = 36.0;
const LABEL_FILE_W: f64 = 24.0;

/// 按钮的三个命中区。分隔线那 0.5pt 归入左侧可拖区，不单独成区。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    /// 复制文件夹路径。
    Folder = 0,
    /// 复制文件路径。
    File = 1,
    /// 右侧 × 关闭钮。
    Close = 2,
    /// 拖动把手：按下后位移超阈值就移动窗口，松手不触发任何复制。
    Drag = 3,
}

/// 材质档位。L1/L2 由系统渲染材质，L3 是手绘兜底。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Material {
    /// macOS 26+ 的原生 Liquid Glass（`NSGlassEffectView`）。
    Glass,
    /// macOS 10.14+ 的经典 vibrancy（`NSVisualEffectView` + `.hudWindow`）。
    Vibrancy,
    /// macOS 12–13 的手绘深色实底。
    Solid,
}

/// 运行时选档：问系统 `NSGlassEffectView` 在不在，不比较版本号。
///
/// 版本号比较要维护白名单，且对未来大版本会失效；类存在性两者都不需要。
/// `NSVisualEffectView`（10.14+）同理，作为第二档。
pub fn material_tier() -> Material {
    if objc2::runtime::AnyClass::get(c"NSGlassEffectView").is_some() {
        Material::Glass
    } else if objc2::runtime::AnyClass::get(c"NSVisualEffectView").is_some() {
        Material::Vibrancy
    } else {
        Material::Solid
    }
}

/// 胶囊总宽。**面板必须严格等于这个值**——比可见胶囊大出来的透明区域会
/// 吃掉下面应用的点击（现有 hitTest 对任何点都返回自身，不做形状判定）。
/// 投影交给系统窗口阴影，macOS 对 borderless 窗口的阴影画在 frame 之外且
/// 不计入命中区。见技术方案 §3.3。
pub fn capsule_width() -> f64 {
    (PAD_L
        + ICON
        + GAP
        + LABEL_FOLDER_W
        + SEG_PAD_R
        + DIVIDER_W
        + SEG_PAD_R
        + ICON
        + GAP
        + LABEL_FILE_W
        + SEG_PAD_R
        + CLOSE_GAP
        + CLOSE_ZONE
        + CLOSE_PAD_R)
        .ceil()
}

/// 各区的 x 边界（右开）。索引 0 为 0，索引 4 为总宽。
fn zone_bounds() -> [f64; 5] {
    let d0 = PAD_L + ICON + GAP + LABEL_FOLDER_W + SEG_PAD_R;
    let d1 = d0 + DIVIDER_W + SEG_PAD_R + ICON + GAP + LABEL_FILE_W + SEG_PAD_R;
    let d2 = d1 + CLOSE_GAP;
    [0.0, d0, d1, d2, capsule_width()]
}

/// 纯函数：x 落在哪个区。宽度与分区共用同一套边界，「算出来的」与
/// 「画出来的」不会漂移。
pub fn zone_at(x: f64) -> Zone {
    let b = zone_bounds();
    if x >= b[3] {
        Zone::Close
    } else if x >= b[2] {
        Zone::Drag
    } else if x >= b[1] {
        Zone::File
    } else {
        Zone::Folder
    }
}

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
    // 宽度按固定公式算（zone_bounds 与之共用同一套边界），不再靠 sizeToFit 收口。
    let size = NSSize::new(capsule_width(), HEIGHT);
    let tier = material_tier();
    let b = zone_bounds();

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

    let view = FloatButtonView::new_with_frame(app.clone(), frame, tier, mtm);

    // 两段文字：左「文件夹」右「文件」。位置与 zone_bounds 同源，保证文字
    // 落在各自命中区内。
    let ink = match tier {
        // L1/L3 底色确定，文字取确定色值；L2 交给系统 labelColor 自适应，
        // 免得在浅材质上压白字（那正是 1.2.1 的老毛病）。
        Material::Glass | Material::Vibrancy => NSColor::labelColor(),
        Material::Solid => NSColor::whiteColor(),
    };
    for (text, x0) in [("文件夹", PAD_L), ("文件", b[1] + SEG_PAD_R)] {
        // ns_string! 只吃字面量，循环里的 text 要自己建 NSString。
        let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
        label.setFont(Some(&NSFont::systemFontOfSize(FONT_SIZE)));
        label.setTextColor(Some(&ink));
        label.sizeToFit();
        label.setFrameOrigin(NSPoint::new(
            x0 + ICON + GAP,
            (HEIGHT - label.frame().size.height) / 2.0,
        ));
        view.addSubview(&label);
    }

    // 材质容器：L1/L2 由系统视图渲染，L3 走 drawRect 手绘。
    let content: Retained<NSView> = match tier {
        Material::Glass => install_glass(&view, frame),
        Material::Vibrancy => install_vibrancy(&view, frame),
        Material::Solid => view.as_super().clone(),
    };

    view.as_super().setToolTip(Some(ns_string!(
        "CopyPath · 左段复制文件夹路径；中段复制文件路径；× 关闭；拖动移动；右键更多"
    )));
    panel.setContentView(Some(&content));
    panel
}

/// L1：macOS 26+ 原生 Liquid Glass。圆角交给玻璃自己（`cornerRadius`），
/// drawRect 因此**不得**再画圆角，否则两者曲率不同会把边缘高光切断。
fn install_glass(view: &Retained<FloatButtonView>, frame: NSRect) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().expect("面板在主线程创建");
    let glass = NSGlassEffectView::initWithFrame(NSGlassEffectView::alloc(mtm), frame);
    glass.setStyle(NSGlassEffectViewStyle::Regular);
    glass.setCornerRadius(RADIUS);
    // 交互形变：让玻璃自带 hover/按下反馈（macOS 27 专有，当前 objc2 绑定尚未
    // 收录该属性，故走 msg_send 设 KVC）。**KVC 可能静默失败**——失败时由
    // drawRect 的手写三态兜底。走查项 W-46 专验这条。
    unsafe {
        let any: &AnyObject = glass.as_super();
        let flag: &AnyObject = &*NSNumber::numberWithBool(true);
        let _: () = msg_send![any, setValue: flag, forKey: ns_string!("effectIsInteractive")];
    }
    glass.setContentView(Some(view.as_super()));
    // 玻璃的直接父类是 NSResponder，contentView 要的是 NSView，故显式转一层。
    let as_view: Retained<NSView> = unsafe { Retained::cast(glass.into_super()) };
    as_view
}

/// L2：macOS 10.14+ 经典 vibrancy。`.hudWindow` 在浅色底上比 `.menu`/`.popover`
/// 更实，够用作兜底。
fn install_vibrancy(view: &Retained<FloatButtonView>, frame: NSRect) -> Retained<NSView> {
    let mtm = MainThreadMarker::new().expect("面板在主线程创建");
    let fx = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), frame);
    fx.setMaterial(NSVisualEffectMaterial::HUDWindow);
    fx.setBlendingMode(NSVisualEffectBlendingMode::WithinWindow);
    fx.setState(NSVisualEffectState::Active);
    fx.setWantsLayer(true);
    fx.layer().setCornerRadius(RADIUS);
    fx.layer().setMasksToBounds(true);
    fx.setContentView(Some(view.as_super()));
    let as_view: Retained<NSView> = unsafe { Retained::cast(fx.into_super()) };
    as_view
}

fn visible_frames(mtm: MainThreadMarker) -> Vec<NSRect> {
    let screens: Retained<NSArray<NSScreen>> = NSScreen::screens(mtm);
    (0..screens.len())
        .map(|i| screens.objectAtIndex(i).visibleFrame())
        .collect()
}

/// 把记住的左上角坐标还原成窗口 frame；不与任何屏幕相交则返回 None（回落默认位置，
/// 覆盖外接屏被拔掉、分辨率变了等场景）。
///
/// 命中某块屏幕后还要**夹紧**到它的可见区内：按钮从 107pt 加宽到 162.5pt 之后，
/// 之前停在屏幕右缘的用户若只按「相交」判定，会有一半按钮落在屏幕外
/// （走查项 W-32）。`default_origin` 本来就会算边距，保存位置的还原路径原先漏了。
fn frame_at(top_left: (f64, f64), size: NSSize, screens: &[NSRect]) -> Option<NSRect> {
    let raw = NSRect::new(
        NSPoint::new(top_left.0, top_left.1 - size.height),
        size,
    );
    let vf = screens.iter().copied().find(|s| intersects(raw, *s))?;
    // 取左缘与右缘都留得住的位置：优先贴住记忆位置，越界才推。
    let x = raw
        .origin
        .x
        .clamp(vf.origin.x, (vf.origin.x + vf.size.width - size.width).max(vf.origin.x));
    // 底部同理。顶部方向越界就交给 max 兜底，min 侧因 visibleFrame 通常不从屏幕顶
    // 开始，这里仍按同一逻辑夹一次，避免多屏不同缩放时出现负值。
    let y = raw
        .origin
        .y
        .clamp(
            vf.origin.y,
            (vf.origin.y + vf.size.height - size.height).max(vf.origin.y),
        );
    Some(NSRect::new(NSPoint::new(x, y), size))
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
    /// 材质档位。drawRect 据此决定要不要手绘圆角与背景（L1 不画，见注释）。
    tier: Cell<Material>,
    /// 拖动标记：mouseUp 据此区分「点击」与「拖完松手」。
    dragging: Cell<bool>,
    /// mouseDown 命中的区：松手时按它分发到复制 / 关闭 / 忽略。
    down_zone: Cell<Zone>,
    /// 指针当前悬停的区，仅作高亮装饰。-1 表示不在任何复制段上。
    hover_zone: Cell<i8>,
    /// 复制成功确认态命中的区，-1 表示无。命中时该段显示 ✓ 并泛绿。
    flash_zone: Cell<i8>,
    /// 确认态结束时刻（`NSDate` 时间戳，0 表示无）。用墙钟而非帧计数，
    /// 定时器被节流时也不会把确认态拖长。
    flash_until: Cell<f64>,
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
            let ivars = self.ivars();
            let bounds = self.as_super().bounds();
            let b = zone_bounds();
            let dark_ink = ivars.tier.get() == Material::Solid;

            // L1 不画圆角：玻璃自带 cornerRadius 且会对内容做折射裁切，两者叠加
            // 会因曲率算法不同而切断边缘高光（走查项 W-49）。L2 的圆角画在
            // CALayer 上，这里同样不画。只有 L3 是纯手绘。
            if ivars.tier.get() == Material::Solid {
                let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                    bounds, RADIUS, RADIUS,
                );
                // 深色实底：白字约 14:1，任何壁纸上都达标。刻意不用 1.2.1 的
                // white@0.10——那在浅底上只有 1.2:1（走查项 W-52）。
                NSColor::colorWithCalibrationRed_green_blue_alpha(0.122, 0.125, 0.157, 0.92)
                    .setFill();
                path.fill();
                NSColor::colorWithWhite_alpha(0.10, 1.0).setStroke();
                path.setLineWidth(1.0);
                path.stroke();
            }

            // 段间分隔线：上下各内缩 DIVIDER_INSET，不通高。
            let line = NSBezierPath::bezierPath();
            line.setLineWidth(DIVIDER_W);
            NSColor::colorWithWhite_alpha(if dark_ink { 0.16 } else { 0.20 }, 1.0).setStroke();
            let mid = bounds.size.height / 2.0;
            let half = bounds.size.height / 2.0 - DIVIDER_INSET;
            line.moveToPoint(NSPoint::new(b[1], mid - half));
            line.lineToPoint(NSPoint::new(b[1], mid + half));
            line.stroke();

            // hover / press 高亮。L1 的原生形变可能没生效（KVC 静默失败），
            // 所以手写这层始终画，作为兜底（走查项 W-46）。
            let mut paint_seg = |x0: f64, x1: f64, flash: bool, hovered: bool, pressed: bool| {
                if !flash && !hovered && !pressed {
                    return;
                }
                let r = NSRect::new(
                    NSPoint::new(x0, 3.0),
                    NSSize::new(x1 - x0, bounds.size.height - 6.0),
                );
                let p = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(r, 7.0, 7.0);
                if flash {
                    NSColor::colorWithCalibrationRed_green_blue_alpha(0.11, 0.55, 0.33, 0.26)
                        .setFill();
                } else if pressed {
                    NSColor::colorWithWhite_alpha(0.16, 1.0).setFill();
                } else {
                    NSColor::colorWithWhite_alpha(0.10, 1.0).setFill();
                }
                p.fill();
            };
            let flash = ivars.flash_zone.get();
            let hover = ivars.hover_zone.get();
            let pressed = !ivars.dragging.get() && ivars.down_zone.get() as i8 != -1;
            paint_seg(
                2.0,
                b[1],
                flash == Zone::Folder as i8,
                hover == Zone::Folder as i8,
                pressed && ivars.down_zone.get() == Zone::Folder,
            );
            paint_seg(
                b[1] + DIVIDER_W,
                b[2],
                flash == Zone::File as i8,
                hover == Zone::File as i8,
                pressed && ivars.down_zone.get() == Zone::File,
            );

            // 两段图标：成功确认态换成 ✓，其余显示各自的 SF Symbol。
            for (zone, x0, name, alt) in [
                (Zone::Folder, PAD_L, "folder", "folder.fill"),
                (Zone::File, b[1] + SEG_PAD_R, "doc", "checkmark"),
            ] {
                let is_flash = flash == zone as i8;
                let symbol = if is_flash { alt } else { name };
                let ink = if is_flash {
                    NSColor::systemGreenColor()
                } else if dark_ink {
                    NSColor::colorWithWhite_alpha(0.95, 1.0)
                } else {
                    NSColor::labelColor()
                };
                if let Some(img) = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                    ns_string!(symbol),
                    None,
                ) {
                    img.setSize(NSSize::new(ICON, ICON));
                    let iv = NSImageView::imageViewWithImage(&img, mtm());
                    iv.setFrameOrigin(NSPoint::new(x0, (HEIGHT - ICON) / 2.0));
                    iv.setContentTintColor(Some(&ink));
                    self.as_super().addSubview(&iv);
                }
            }

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
            NSColor::colorWithWhite_alpha(if dark_ink { 0.55 } else { 0.45 }, 1.0).setStroke();
            cross.stroke();
        }

        /// 鼠标在视图内移动：只更新高亮，不改语义。
        #[unsafe(method(mouseMoved:))]
        unsafe fn mouse_moved(&self, event: &NSEvent) {
            let ivars = self.ivars();
            let z = zone_at(event.locationInWindow().x);
            let next = match z {
                Zone::Folder => Zone::Folder as i8,
                Zone::File => Zone::File as i8,
                _ => -1,
            };
            if next != ivars.hover_zone.get() {
                ivars.hover_zone.set(next);
                self.as_super().setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseExited:))]
        unsafe fn mouse_exited(&self) {
            let ivars = self.ivars();
            if ivars.hover_zone.get() != -1 {
                ivars.hover_zone.set(-1);
                self.as_super().setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseDown:))]
        unsafe fn mouse_down(&self, event: &NSEvent) {
            let ivars = self.ivars();
            ivars.dragging.set(false);
            let point = event.locationInWindow();
            ivars.start.set(point);
            ivars.down_zone.set(zone_at(point.x));
            if let Some(window) = self.as_super().window() {
                ivars.base_origin.set(window.frame().origin);
            }
            self.as_super().setNeedsDisplay(true);
        }

        #[unsafe(method(mouseDragged:))]
        unsafe fn mouse_dragged(&self, event: &NSEvent) {
            let ivars = self.ivars();
            // × 区禁拖动（1.2.1 已定的规则）：避免「想关按钮却拖动了窗口」。
            if ivars.down_zone.get() == Zone::Close {
                return;
            }
            let start = ivars.start.get();
            let current = event.locationInWindow();
            let dx = current.x - start.x;
            let dy = current.y - start.y;
            if !ivars.dragging.get() && (dx * dx + dy * dy).sqrt() > DRAG_THRESHOLD {
                ivars.dragging.set(true);
                self.as_super().setNeedsDisplay(true);
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
                self.as_super().setNeedsDisplay(true);
                return;
            }
            match ivars.down_zone.get() {
                Zone::Close => {
                    // 与右键菜单「隐藏悬浮按钮」同一条落盘关闭路径。
                    hide_and_disable(&ivars.app);
                }
                Zone::Folder => {
                    // 与托盘「立即获取文件夹路径」同一条代码路径（含最近历史与重入保护）。
                    start_flash(self, Zone::Folder);
                    crate::fetch_and_copy(&ivars.app, Mode::FolderPath);
                }
                Zone::File => {
                    start_flash(self, Zone::File);
                    crate::fetch_and_copy(&ivars.app, Mode::FilePath);
                }
                // 两段之间的留白：按下是为了拖动，松手不在任何段上，什么都不做。
                Zone::Drag => {}
            }
            self.as_super().setNeedsDisplay(true);
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
            start_flash(self, Zone::Folder);
            crate::fetch_and_copy(&self.ivars().app, Mode::FolderPath);
        }

        #[unsafe(method(copyFilePath:))]
        unsafe fn copy_file_path(&self, _sender: Option<&AnyObject>) {
            start_flash(self, Zone::File);
            crate::fetch_and_copy(&self.ivars().app, Mode::FilePath);
        }

        #[unsafe(method(hidePanel:))]
        unsafe fn hide_panel(&self, _sender: Option<&AnyObject>) {
            hide_and_disable(&self.ivars().app);
        }
    }
);

impl FloatButtonView {
    fn new_with_frame(
        app: AppHandle,
        frame: NSRect,
        tier: Material,
        mtm: MainThreadMarker,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ButtonIvars {
            app,
            tier: Cell::new(tier),
            dragging: Cell::new(false),
            down_zone: Cell::new(Zone::Drag),
            hover_zone: Cell::new(-1),
            flash_zone: Cell::new(-1),
            flash_until: Cell::new(0.0),
            start: Cell::new(NSPoint::new(0.0, 0.0)),
            base_origin: Cell::new(NSPoint::new(0.0, 0.0)),
        });
        let view: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };

        // hover 高亮需要一个 tracking area；触摸板不移动也能触发 mouseMoved。
        // 只作装饰——两个复制段常驻可点，不依赖 hover 才有功能。
        let opts = NSTrackingAreaOptions::MouseMoved
            | NSTrackingAreaOptions::ActiveInKeyWindow
            | NSTrackingAreaOptions::InVisibleRect;
        let area = NSTrackingArea::initWithFrame_options_owner(
            NSTrackingArea::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), frame.size),
            opts,
            Some(view.as_super()),
        );
        view.as_super().addTrackingArea(&area);
        view
    }

    /// 右键小菜单：三种复制语义 + 隐藏入口。
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
        add_item("立即获取文件夹路径", sel!(copyFolderPath:));
        add_item("复制选中项自身路径", sel!(copyItemPath:));
        add_item("复制文件路径", sel!(copyFilePath:));
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        add_item("隐藏悬浮按钮", sel!(hidePanel:));
        menu
    }
}

/// 主线程标记的便捷取值。drawRect 之类被系统回调的方法里没有 mtm 参数，
/// 而 AppKit 视图方法必然跑在主线程，这里直接 unwrap。
fn mtm() -> MainThreadMarker {
    MainThreadMarker::new().expect("AppKit 视图方法必然在主线程")
}

/// 让某一段进入复制成功的确认态，并在 FLASH_SECS 后复原。
///
/// 拆成两步而不是靠定时器反复轮询：一次性 `repeats: false` 的定时器在
/// 0.7s 后只做「复原」这一件事，中途不需要重绘（flash_until 存的是墙钟，
/// 万一定时器被系统推迟了，首次触发时会立刻判超时并复原，不会拖长）。
///
/// 定时器不持有视图的强引用：block 里只按窗口可见性做防御性检查。
/// NSTimer 挂上 run loop 后即由 run loop 持有，故不需要额外保活。
fn start_flash(view: &FloatButtonView, zone: Zone) {
    let ivars = view.ivars();
    ivars.flash_zone.set(zone as i8);
    ivars.flash_until.set(now_secs() + FLASH_SECS);
    view.as_super().setNeedsDisplay(true);

    unsafe {
        let block = block2::RcBlock::new(move |_timer: core::ptr::NonNull<NSTimer>| {
            // AppKit 定时器在主线程触发，但面板可能已被 × 隐藏。
            if MainThreadMarker::new().is_none() {
                return;
            }
            let ivars = view.ivars();
            if now_secs() >= ivars.flash_until.get() {
                ivars.flash_zone.set(-1);
                ivars.flash_until.set(0.0);
            }
            view.as_super().setNeedsDisplay(true);
        });
        NSTimer::scheduledTimerWithTimeInterval_repeats_block(
            FLASH_SECS,
            false,
            &block,
        );
    }
}

/// 墙钟秒数。用绝对时间而非帧计数：定时器被系统节流时确认态不会被拖长。
fn now_secs() -> f64 {
    let t = NSDate::timeIntervalSinceReferenceDate(NSDate::new());
    t as f64
}

#[cfg(test)]
mod zone_tests {
    use super::*;

    /// 宽度公式的回归基线（走查 W-44 口径：数值与视觉稿一致）。
    #[test]
    fn capsule_width_matches_the_design_spec() {
        assert_eq!(capsule_width(), 162.5);
    }

    #[test]
    fn divider_height_is_sixteen() {
        assert_eq!(HEIGHT - DIVIDER_INSET * 2.0, 16.0);
    }

    /// 三个区的边界值。**逐个打在边界上与边界两侧**，避免只测中间值。
    #[test]
    fn zone_boundaries_are_exact() {
        let b = zone_bounds();
        // 分隔线：0.5pt 归入左段，右段从分隔线之后开始。
        assert_eq!(zone_at(0.0), Zone::Folder);
        assert_eq!(zone_at(b[1] - 0.1), Zone::Folder);
        assert_eq!(zone_at(b[1]), Zone::Folder, "分隔线左缘仍是文件夹段");
        assert_eq!(zone_at(b[1] + DIVIDER_W + 0.1), Zone::File);
        assert_eq!(zone_at(b[2] - 0.1), Zone::File);
        // 文件段与拖动留白之间。
        assert_eq!(zone_at(b[2]), Zone::Drag);
        assert_eq!(zone_at(b[3] - 0.1), Zone::Drag);
        // × 区从 b[3] 开始直到右缘。
        assert_eq!(zone_at(b[3]), Zone::Close);
        assert_eq!(zone_at(capsule_width() - 0.1), Zone::Close);
    }

    /// 两段都够宽，图标与文字不会被挤到相邻段里。
    #[test]
    fn both_copy_segments_are_wide_enough() {
        let b = zone_bounds();
        assert!(
            b[1] - 0.0 > 60.0,
            "文件夹段应有足够宽度，实测 {}",
            b[1]
        );
        assert!(b[2] - b[1] > 50.0, "文件段应有足够宽度，实测 {}", b[2] - b[1]);
        assert!(b[3] - b[2] <= 6.0, "拖动留白不应大到吞掉视觉分界");
    }

    /// 圆角不是全胶囊：宽度远大于高度，若半径等于高度一半就会变药丸。
    #[test]
    fn radius_is_not_a_full_pill() {
        assert_ne!(RADIUS, HEIGHT / 2.0);
        assert!(capsule_width() / HEIGHT > 4.0);
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
    fn frame_at_clamps_wide_button_back_inside_the_screen() {
        // 按钮加宽到 162.5pt 后，旧版本「只要相交就认」的判定会让停在
        // 1440pt 右缘的按钮有一半落在屏幕外。夹紧后右缘应正好贴住屏幕右缘。
        let screens = [screen(0.0, 0.0, 1440.0, 900.0)];
        let size = NSSize::new(162.5, 36.0);
        let frame = frame_at((1440.0, 134.0), size, &screens).unwrap();
        assert_eq!(frame.origin.x, 1440.0 - 162.5);
        assert_eq!(frame.origin.y, 98.0);
    }

    #[test]
    fn frame_at_clamps_below_the_bottom_edge() {
        let screens = [screen(0.0, 0.0, 1440.0, 900.0)];
        let size = NSSize::new(162.5, 36.0);
        // top_left.y = -20 表示按钮几乎整个在屏幕下方。
        let frame = frame_at((100.0, -20.0), size, &screens).unwrap();
        assert_eq!(frame.origin.y, 0.0);
    }

    #[test]
    fn frame_at_leaves_an_onscreen_position_untouched() {
        // 已在屏内的位置不该被夹紧挪动。
        let screens = [screen(0.0, 0.0, 1440.0, 900.0)];
        let size = NSSize::new(162.5, 36.0);
        let frame = frame_at((100.0, 500.0), size, &screens).unwrap();
        assert_eq!(frame.origin, NSPoint::new(100.0, 464.0));
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
