//! 系统托盘支持
//!
//! 窗口隐藏方案：Win32 ShowWindow(SW_HIDE)，eframe 不感知，update() 持续运行。
//!
//! 恢复/退出的可靠性问题：
//!   隐藏后 WM_PAINT 被系统抑制，ctx.request_repaint() 无法唤醒 update()。
//!   因此两个操作都直接通过 Win32 触发：
//!
//!   - Show：relay 线程直接调用 SW_SHOW → 窗口可见 → WM_PAINT 正常触发 → update() 运行
//!   - Quit：relay 线程设置 force_quit 标志 + PostMessageW(WM_CLOSE) →
//!           WM_CLOSE 对隐藏窗口照常投递 → eframe 的关闭处理器运行
//!
//! wndproc 钩子安装：
//!   SetWindowLongPtr(GWLP_WNDPROC) 必须在窗口所属线程（主线程）上调用才可靠。
//!   因此由 install_close_hook_once() 在 update() 第一帧中安装，而非 relay 线程。
//!   钩子拦截 SC_CLOSE 后调用已保存的 egui Context::request_repaint()，
//!   确保 update() 能在同帧内立即被调用并处理 close_button_clicked() 标志。

use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
use std::sync::{Arc, OnceLock};

/// X 按钮被点击时由 wndproc 钩子置 true
static CLOSE_BUTTON_CLICKED: AtomicBool = AtomicBool::new(false);
/// 原始 WNDPROC 指针（替换前保存，用于转发其他消息）
#[cfg(windows)]
static ORIGINAL_WNDPROC: AtomicIsize = AtomicIsize::new(0);
/// wndproc 钩子是否已安装
static HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);
/// egui 上下文（供 wndproc 钩子调用 request_repaint）
static EGUI_CTX: OnceLock<egui::Context> = OnceLock::new();

pub fn close_button_clicked() -> bool {
    CLOSE_BUTTON_CLICKED.load(Ordering::SeqCst)
}

pub fn reset_close_button() {
    CLOSE_BUTTON_CLICKED.store(false, Ordering::SeqCst);
}

/// 保存 egui 上下文供 wndproc 钩子使用，在 FileSyncApp::new() 中调用一次。
pub fn set_egui_ctx(ctx: egui::Context) {
    let _ = EGUI_CTX.set(ctx);
}

/// 从主线程安装 WM_SYSCOMMAND(SC_CLOSE) 拦截器（幂等，首次调用后不重复安装）。
/// 必须在窗口所属线程（即 update() 所在的主线程）调用，SetWindowLongPtr(GWLP_WNDPROC)
/// 从其他线程调用可能静默失效。
pub fn install_close_hook_once() {
    #[cfg(windows)]
    if !HOOK_INSTALLED.load(Ordering::SeqCst) {
        if let Some(hwnd) = find_main_window() {
            install_close_hook(hwnd);
            HOOK_INSTALLED.store(true, Ordering::SeqCst);
        }
    }
}

/// 替换主窗口 WNDPROC，拦截 WM_SYSCOMMAND(SC_CLOSE)（X 按钮 / Alt+F4）。
/// 调用点：relay 后台线程找到主窗口后执行一次。
#[cfg(windows)]
pub fn install_close_hook(hwnd: windows::Win32::Foundation::HWND) {
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, WINDOW_LONG_PTR_INDEX,
    };
    const GWLP_WNDPROC: WINDOW_LONG_PTR_INDEX = WINDOW_LONG_PTR_INDEX(-4);
    unsafe {
        let original = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
        ORIGINAL_WNDPROC.store(original, Ordering::SeqCst);
        SetWindowLongPtrW(hwnd, GWLP_WNDPROC, close_wndproc as *const () as isize);
    }
}

#[cfg(windows)]
unsafe extern "system" fn close_wndproc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    // WM_SYSCOMMAND = 0x0112, SC_CLOSE = 0xF060, WM_CLOSE = 0x0010
    // WM_SYSCOMMAND(SC_CLOSE)：X 按钮 / Alt+F4
    if msg == 0x0112 && (wparam.0 & 0xFFF0) == 0xF060 {
        CLOSE_BUTTON_CLICKED.store(true, Ordering::SeqCst);
        // 通知 eframe 立即渲染下一帧以运行 update()，检查 close_button_clicked()。
        // 若不调用此函数，eframe 在无待处理重绘时不会调用 update()，导致关闭无响应。
        if let Some(ctx) = EGUI_CTX.get() {
            ctx.request_repaint();
        }
        // 吃掉 SC_CLOSE：不转给 DefWindowProc，也不 PostMessage WM_CLOSE。
        // 所有关闭逻辑由 update() 中的 close_button_clicked() 处理，
        // 避免 eframe 收到 WM_CLOSE 后在 update() 能响应前就自动关闭窗口。
        return LRESULT(0);
    }
    // WM_CLOSE：若钩子已安装，此消息只会来自托盘"退出"的 post_close_message()，
    // 此时 force_quit=true，update() 会处理退出；吃掉避免 eframe 提前关闭。
    if msg == 0x0010 {
        return LRESULT(0);
    }
    let original = ORIGINAL_WNDPROC.load(Ordering::SeqCst);
    if original != 0 {
        use windows::Win32::UI::WindowsAndMessaging::CallWindowProcW;
        let proc: windows::Win32::UI::WindowsAndMessaging::WNDPROC =
            std::mem::transmute(original);
        CallWindowProcW(proc, hwnd, msg, wparam, lparam)
    } else {
        LRESULT(0)
    }
}

use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::i18n::is_zh;

/// 托盘图标持有者（保持 TrayIcon 存活即可维持系统图标）
pub struct AppTray {
    #[allow(dead_code)]
    tray: TrayIcon,
    show_id: MenuId,
    quit_id: MenuId,
    /// 由托盘"退出"操作置 true，供关闭处理器跳过"最小化到托盘"逻辑
    pub force_quit: Arc<AtomicBool>,
}

impl AppTray {
    /// 从 egui 图标 RGBA 数据创建系统托盘图标。
    /// 失败时返回 `None`（无系统托盘的环境下安全降级）。
    pub fn new(rgba: Vec<u8>, width: u32, height: u32) -> Option<Self> {
        let icon = Icon::from_rgba(rgba, width, height).ok()?;

        let zh = is_zh();
        let show_label = if zh { "显示 FileSync" } else { "Show FileSync" };
        let quit_label = if zh { "退出" } else { "Quit" };

        let show_item = MenuItem::new(show_label, true, None);
        let quit_item = MenuItem::new(quit_label, true, None);
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();

        let menu = Menu::new();
        menu.append(&show_item).ok()?;
        menu.append(&quit_item).ok()?;

        let tray = TrayIconBuilder::new()
            .with_tooltip("FileSync")
            .with_icon(icon)
            .with_menu(Box::new(menu))
            .build()
            .ok()?;

        Some(Self {
            tray,
            show_id,
            quit_id,
            force_quit: Arc::new(AtomicBool::new(false)),
        })
    }

    /// 启动后台轮询线程。
    ///
    /// Show：直接调用 Win32 SW_SHOW 使窗口可见，WM_PAINT 随后触发，eframe 正常运行。
    /// Quit：设置 force_quit 并通过 PostMessageW(WM_CLOSE) 通知主窗口退出。
    ///
    /// 注意：wndproc 钩子不在此处安装，由 install_close_hook_once() 在主线程 update() 中安装。
    pub fn start_event_relay(&self) {
        let show_id = self.show_id.clone();
        let quit_id = self.quit_id.clone();
        let force_quit = self.force_quit.clone();

        std::thread::spawn(move || {
            loop {
                // 托盘图标左键单击 → 显示窗口
                while let Ok(event) = TrayIconEvent::receiver().try_recv() {
                    if let TrayIconEvent::Click {
                        button: tray_icon::MouseButton::Left,
                        button_state: tray_icon::MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_app_window();
                    }
                }

                // 右键菜单点击
                while let Ok(event) = MenuEvent::receiver().try_recv() {
                    if event.id == show_id {
                        show_app_window();
                    } else if event.id == quit_id {
                        force_quit.store(true, Ordering::SeqCst);
                        // 先使窗口可见（恢复 WM_PAINT → eframe 的 update() 能运行），
                        // 再投递 WM_CLOSE，确保 close handler 中的
                        // tray drop + process::exit(0) 被执行。
                        show_app_window();
                        // 给窗口一点时间显示，确保消息泵在运转
                        std::thread::sleep(std::time::Duration::from_millis(100));
                        post_close_message();
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(50));
            }
        });
    }
}

// ─────────────────────────────────────────────────────────────────
// Win32 辅助（pub：供 app.rs 在关闭处理器中调用 hide）
// ─────────────────────────────────────────────────────────────────

/// 通过 Win32 SW_HIDE 隐藏主窗口。eframe 不感知，update() 持续运行。
pub fn hide_app_window() {
    #[cfg(windows)]
    if let Some(hwnd) = find_main_window() {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        unsafe { let _ = ShowWindow(hwnd, SW_HIDE); }
    }
}

/// 通过 Win32 SW_SHOW / SW_RESTORE 恢复并前置主窗口。
///
/// 从 relay 线程调用：使窗口可见后 WM_PAINT 触发，eframe 恢复 update()。
pub fn show_app_window() {
    #[cfg(windows)]
    if let Some(hwnd) = find_main_window() {
        use windows::Win32::UI::WindowsAndMessaging::{
            IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
        };
        unsafe {
            let cmd = if IsIconic(hwnd).as_bool() { SW_RESTORE } else { SW_SHOW };
            let _ = ShowWindow(hwnd, cmd);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

/// 向主窗口投递 WM_CLOSE。隐藏窗口的消息队列仍正常处理 WM_CLOSE。
fn post_close_message() {
    #[cfg(windows)]
    if let Some(hwnd) = find_main_window() {
        use windows::Win32::Foundation::{LPARAM, WPARAM};
        use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        let _ = unsafe { PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

#[cfg(windows)]
fn find_main_window() -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::UI::WindowsAndMessaging::FindWindowW;
    let title: Vec<u16> = "FileSync\0".encode_utf16().collect();
    unsafe {
        FindWindowW(
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::from_raw(title.as_ptr()),
        )
    }
    .ok()
    .filter(|h| !h.is_invalid())
}
