// 在 Release 模式下隐藏控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod fs;
mod i18n;
mod model;
mod tray;
mod ui;

fn main() -> eframe::Result<()> {
    // 单实例检测：若已有实例在运行则提示后退出
    let _mutex = match single_instance_guard() {
        Some(m) => m,
        None => return Ok(()),
    };

    let icon_data = make_icon();

    // 创建系统托盘图标（在 eframe 启动前创建，存活至程序退出）
    let tray = tray::AppTray::new(
        icon_data.rgba.clone(),
        icon_data.width,
        icon_data.height,
    );

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FileSync")
            .with_min_inner_size([800.0, 500.0])
            .with_inner_size([1000.0, 650.0])
            .with_icon(icon_data),
        ..Default::default()
    };

    eframe::run_native(
        "FileSync",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::FileSyncApp::new(cc, tray)))),
    )
}

// ─────────────────────────────────────────────────────────────────
// 单实例保护
// ─────────────────────────────────────────────────────────────────

/// 尝试创建全局命名互斥量以保证单实例运行。
///
/// - 首个实例：返回 `Some(guard)`，调用方持有 guard 直至进程退出（drop 时释放互斥量）。
/// - 已有实例：弹窗提示后返回 `None`，调用方应直接退出。
fn single_instance_guard() -> Option<Box<dyn std::any::Any>> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONINFORMATION, MB_OK, HWND_DESKTOP,
        };

        // 用带 GUID 的名称避免与其他软件冲突
        let name: Vec<u16> = "Global\\FileSync-B3C4D5E6-F7A8-9B0C-1D2E-3F4A5B6C7D8E"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let result = unsafe {
            CreateMutexW(None, false, windows::core::PCWSTR::from_raw(name.as_ptr()))
        };

        match result {
            Ok(handle) => {
                if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
                    let zh = i18n::is_zh();
                    let msg_str = if zh {
                        "FileSync 已在运行中。\n请查看系统托盘图标。"
                    } else {
                        "FileSync is already running.\nCheck the system tray icon."
                    };
                    let msg: Vec<u16> =
                        msg_str.encode_utf16().chain(std::iter::once(0)).collect();
                    let title: Vec<u16> =
                        "FileSync".encode_utf16().chain(std::iter::once(0)).collect();
                    unsafe {
                        MessageBoxW(
                            HWND_DESKTOP,
                            windows::core::PCWSTR::from_raw(msg.as_ptr()),
                            windows::core::PCWSTR::from_raw(title.as_ptr()),
                            MB_OK | MB_ICONINFORMATION,
                        );
                    }
                    None
                } else {
                    Some(Box::new(handle) as Box<dyn std::any::Any>)
                }
            }
            // 互斥量创建失败（权限问题等）：放行，避免误拒绝启动
            Err(_) => Some(Box::new(()) as Box<dyn std::any::Any>),
        }
    }
    #[cfg(not(windows))]
    {
        Some(Box::new(()) as Box<dyn std::any::Any>)
    }
}

/// 程序化生成 32×32 同步图标（蓝色圆形背景 + 白色双弧箭头）
fn make_icon() -> egui::IconData {

    const S: u32 = 32;
    let mut rgba = vec![0u8; (S * S * 4) as usize];

    for py in 0..S {
        for px in 0..S {
            let dx = px as f32 + 0.5 - 15.5;
            let dy = py as f32 + 0.5 - 15.5;
            let r = (dx * dx + dy * dy).sqrt();
            let a = dy.atan2(dx).to_degrees(); // -180..180

            let i = ((py * S + px) * 4) as usize;

            // 蓝色圆形背景
            if r <= 15.0 {
                rgba[i]     = 38;
                rgba[i + 1] = 110;
                rgba[i + 2] = 200;
                rgba[i + 3] = 255;
            }

            // 白色同步弧：半径 8.5..12.5
            // 弧 A（右侧）：-150° 到 +25°（穿过 0°/右方向）
            // 弧 B（左侧）：+155° 到 -120°（穿过 ±180°/左方向）
            if r >= 8.5 && r <= 12.5 {
                let in_a = a >= -150.0 && a <= 25.0;
                let in_b = a >= 155.0 || a <= -120.0;
                if in_a || in_b {
                    rgba[i]     = 220;
                    rgba[i + 1] = 235;
                    rgba[i + 2] = 255;
                    rgba[i + 3] = 255;
                }
            }
        }
    }

    // 弧 A 末端箭头（+25° 处，约像素 (25, 20)，箭头朝顺时针方向）
    for &(x, y) in &[
        (23u32, 19u32), (24, 19), (25, 19),
        (22, 20), (23, 20), (24, 20), (25, 20),
        (23, 21), (24, 21),
    ] {
        if x < S && y < S {
            let i = ((y * S + x) * 4) as usize;
            rgba[i]     = 220;
            rgba[i + 1] = 235;
            rgba[i + 2] = 255;
            rgba[i + 3] = 255;
        }
    }

    // 弧 B 末端箭头（-120° 处，约像素 (10, 6)，箭头朝顺时针方向）
    for &(x, y) in &[
        (8u32, 6u32), (9, 6), (10, 6),
        (7, 7), (8, 7), (9, 7), (10, 7),
        (8, 8), (9, 8),
    ] {
        if x < S && y < S {
            let i = ((y * S + x) * 4) as usize;
            rgba[i]     = 220;
            rgba[i + 1] = 235;
            rgba[i + 2] = 255;
            rgba[i + 3] = 255;
        }
    }

    egui::IconData { rgba, width: S, height: S }
}
