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

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
