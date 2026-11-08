//! macOS 应用 UI（Dock 常驻 + 菜单栏托盘 + 主窗口）。
//!
//! - Dock 图标：Regular 激活策略（Info.plist 已无 LSUIElement），点击 Dock 重开主窗口
//!   （NSApplicationDelegate.applicationShouldHandleReopen）；
//! - 主窗口：WKWebView 内嵌承载本机控制台页（/admin），连接/配对/设备/配置全部在应用内
//!   完成，不打开外部浏览器；
//! - 菜单栏托盘：tray-icon + 菜单（muda），状态徽标每 2s 刷新；
//! - 线程模型：AppKit/WebKit 对象只在主线程触碰；菜单事件线程把动作 dispatch 回主队列；
//!   tokio 服务（HTTP/WS/mDNS）在后台线程。

use std::sync::{Arc, Mutex};
use std::time::Duration;

use objc2::rc::{Allocated, Retained};
use objc2::runtime::NSObject;
use objc2::DeclaredClass;
use objc2::runtime::ProtocolObject;
use objc2::{class, declare_class, msg_send_id, ClassType};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSAutoresizingMaskOptions, NSWindow, NSWindowStyleMask, NSScreen,
};
use objc2_foundation::{
    MainThreadMarker, NSPoint, NSRect, NSSize, NSString, NSURL, NSURLRequest, NSObjectProtocol,
};
use objc2_web_kit::{WKWebView, WKWebViewConfiguration};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{TrayIcon, TrayIconBuilder};

use crate::ws::Hub;

const ICON_PNG: &[u8] = include_bytes!("../../../web/public/icons/icon-192.png");

/// 仅在主线程触碰；存 &'static MainWindow 的裸指针（usize，满足 static 的 Send+Sync）。
static MAIN_WINDOW: Mutex<Option<usize>> = Mutex::new(None);
/// 委托回调（Dock 点击重开）需要的 hub 引用。
static DELEGATE_HUB: Mutex<Option<Arc<Hub>>> = Mutex::new(None);

struct MainWindow {
    window: Retained<NSWindow>,
}

fn admin_url(hub: &Arc<Hub>) -> String {
    let port = hub.cfg.lock().unwrap().server.port;
    format!("http://127.0.0.1:{port}/admin")
}

/// 主线程：确保主窗口存在并前置显示。
fn show_window(mtm: MainThreadMarker, hub: &Arc<Hub>) {
    let mw = {
        let mut slot = MAIN_WINDOW.lock().unwrap();
        match *slot {
            Some(ptr) => unsafe { &*(ptr as *const MainWindow) },
            None => {
                let mw: &'static MainWindow = Box::leak(Box::new(build_window(mtm, hub)));
                *slot = Some(mw as *const MainWindow as usize);
                mw
            }
        }
    };
    unsafe {
        mw.window.makeKeyAndOrderFront(None);
        mw.window.orderFrontRegardless();
    }
}

fn build_window(mtm: MainThreadMarker, hub: &Arc<Hub>) -> MainWindow {
    let (w, h) = (430.0, 700.0);
    let style = NSWindowStyleMask::Titled
        | NSWindowStyleMask::Closable
        | NSWindowStyleMask::Miniaturizable
        | NSWindowStyleMask::Resizable;
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h));
    let window = unsafe {
        let allocated: Allocated<NSWindow> = msg_send_id![class!(NSWindow), alloc];
        NSWindow::initWithContentRect_styleMask_backing_defer(
            allocated,
            rect,
            style,
            NSBackingStoreType::NSBackingStoreBuffered,
            false,
        )
    };
    unsafe {
        window.setTitle(&NSString::from_str("Palm — 连接与配置"));
        window.setReleasedWhenClosed(false); // 关闭仅隐藏，Dock/菜单可再次打开
        window.setContentMinSize(NSSize::new(340.0, 460.0)); // 可拉伸/压缩的下限
    }

    // WKWebView 填充内容，加载本机控制台页（进程内 HTTP，不打开外部浏览器）
    let web = unsafe {
        let config: Retained<WKWebViewConfiguration> =
            msg_send_id![WKWebViewConfiguration::class(), new];
        let allocated: Allocated<WKWebView> = msg_send_id![WKWebView::class(), alloc];
        WKWebView::initWithFrame_configuration(
            allocated,
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(w, h)),
            &config,
        )
    };
    unsafe {
        if let Some(url) = NSURL::URLWithString(&NSString::from_str(&admin_url(hub))) {
            let req = NSURLRequest::requestWithURL(&url);
            web.loadRequest(&req);
        }
    }
    let content = window.contentView().unwrap();
    unsafe {
        // 宽高随窗口伸缩（拉伸/压缩时自动跟随）
        web.setAutoresizingMask(
            NSAutoresizingMaskOptions::NSViewWidthSizable
                | NSAutoresizingMaskOptions::NSViewHeightSizable,
        );
        content.addSubview(&web);
        // 启动即占满一屏：取主屏可用区域（自动避开菜单栏/Dock）。
        // 必须在 addSubview 之后执行，webview 的 autoresizing 才会跟随放大。
        if let Some(screen) = NSScreen::mainScreen(mtm) {
            window.setFrame_display(screen.visibleFrame(), true);
        }
    };

    MainWindow { window }
}

// ---------------------------------------------------------------------------
// NSApplicationDelegate：Dock 点击重开主窗口
// ---------------------------------------------------------------------------

declare_class!(
    struct AppDelegate;

    unsafe impl ClassType for AppDelegate {
        type Super = NSObject;
        type Mutability = objc2::mutability::MainThreadOnly;
        const NAME: &'static str = "PalmAppDelegate";
    }

    impl DeclaredClass for AppDelegate {}

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        #[allow(non_snake_case)]
        #[method(applicationShouldHandleReopen:hasVisibleWindows:)]
        unsafe fn applicationShouldHandleReopen_hasVisibleWindows(
            &self,
            _sender: &NSApplication,
            _has_visible_windows: bool,
        ) -> bool {
            let mtm = MainThreadMarker::new().expect("delegate runs on main thread");
            if let Some(hub) = DELEGATE_HUB.lock().unwrap().clone() {
                show_window(mtm, &hub);
                true
            } else {
                false
            }
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = mtm.alloc().set_ivars(());
        unsafe { msg_send_id![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// 菜单栏（托盘）
// ---------------------------------------------------------------------------

/// 阻塞主线程运行应用事件循环（NSApp.run 不返回；退出走进程退出）。
pub fn run(hub: Arc<Hub>, _port: u16) -> anyhow::Result<()> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| anyhow::anyhow!("app UI must run on the main thread"))?;
    let app = NSApplication::sharedApplication(mtm);
    // Regular：Dock 常驻图标（点击 Dock 重开主窗口，由 AppDelegate 处理）
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

    // 委托：注册 Dock 重开回调
    *DELEGATE_HUB.lock().unwrap() = Some(hub.clone());
    let delegate: &'static AppDelegate =
        unsafe { &*Retained::into_raw(AppDelegate::new(mtm)) };
    app.setDelegate(Some(ProtocolObject::from_ref(delegate)));

    // —— 托盘菜单 ——
    let menu = Menu::new();
    let status_item = MenuItem::new("○ 等待手机接入", false, None);
    menu.append(&status_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    let open_item = MenuItem::new("打开连接与配置…", true, None);
    menu.append(&open_item)?;
    let trust_item = MenuItem::new("辅助功能权限设置…", true, None);
    menu.append(&trust_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    let quit_item = MenuItem::new("退出 Palm", true, None);
    menu.append(&quit_item)?;

    let tray = TrayIconBuilder::new()
        .with_id("palm")
        .with_icon(load_icon()?)
        .with_tooltip("Palm — 手机触控板服务")
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(true)
        .with_title("")
        .build()?;

    // 泄漏保活；跨线程只传裸指针（解引用只在主线程）
    let tray: &'static TrayIcon = Box::leak(Box::new(tray));
    let status_item: &'static MenuItem = Box::leak(Box::new(status_item));
    let tray_ptr = tray as *const TrayIcon as usize;
    let status_ptr = status_item as *const MenuItem as usize;

    // —— 菜单点击（muda 事件线程）→ 主线程套用 ——
    let open_id = open_item.id().clone();
    let trust_id = trust_item.id().clone();
    let quit_id = quit_item.id().clone();
    let hub_menu = hub.clone();
    let rt = tokio::runtime::Handle::current();
    let queue = dispatch2::DispatchQueue::main();
    std::thread::spawn(move || {
        for ev in MenuEvent::receiver() {
            if ev.id == open_id {
                let hub = hub_menu.clone();
                queue.exec_async(move || {
                    let mtm = MainThreadMarker::new().expect("main queue runs on main thread");
                    show_window(mtm, &hub);
                });
            } else if ev.id == trust_id {
                // 触发系统授权弹窗（AXIsProcessTrustedWithOptions ask=true），
                // 内部失败时自行回退深链
                let hub = hub_menu.clone();
                rt.spawn(async move {
                    hub.backend.lock().await.request_trust();
                });
            } else if ev.id == quit_id {
                std::process::exit(0);
            }
        }
    });

    // —— 状态轮询：tokio 算文本 → 主队列刷托盘 ——
    let hub_poll = hub.clone();
    tokio::spawn(async move {
        let hub = hub_poll;
        loop {
            let status = status_text(&hub).await;
            let badge = if status.starts_with('⚠') {
                "⚠"
            } else if status.starts_with('●') {
                "●"
            } else {
                ""
            };
            queue.exec_async(move || {
                unsafe {
                    (*(tray_ptr as *const TrayIcon)).set_title(Some(badge));
                    (*(status_ptr as *const MenuItem)).set_text(&status);
                }
            });
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    // 首次使用（还没有任何已配对设备）自动打开主窗口完成引导
    if hub.cfg.lock().unwrap().device.is_empty() {
        show_window(mtm, &hub);
    }

    unsafe { app.run() }; // 阻塞直至退出
    Ok(())
}

async fn status_text(hub: &Arc<Hub>) -> String {
    let trusted = hub.backend.lock().await.trusted();
    if !trusted {
        return "⚠ Palm 需要辅助功能权限".to_string();
    }
    let name = hub.active_name.lock().unwrap().clone();
    match name {
        Some(n) if !n.is_empty() => format!("● 已连接 {n}"),
        _ => "○ 等待手机接入".to_string(),
    }
}

fn load_icon() -> anyhow::Result<tray_icon::Icon> {
    let img = image::load_from_memory(ICON_PNG)?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Ok(tray_icon::Icon::from_rgba(rgba.into_raw(), w, h)?)
}
