// 在 Release 模式下隐藏控制台窗口
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod config;
mod engine;
mod fs;
mod i18n;
mod log;
mod model;
mod tray;
mod ui;

use crate::log::{app_log, LogLevel};

/// SEH exception handler — catches access violations, stack overflows, etc.
/// Uses raw FFI to avoid pulling in heavy windows crate features.
#[cfg(windows)]
fn install_seh_handler() {
    type ExceptionFilterFn =
        unsafe extern "system" fn(*const EXCEPTION_POINTERS) -> i32;

    type SetUnhandledExceptionFilterFn =
        unsafe extern "system" fn(ExceptionFilterFn) -> ExceptionFilterFn;

    #[repr(C)]
    struct EXCEPTION_POINTERS {
        exception_record: *const EXCEPTION_RECORD,
        context_record: *const std::ffi::c_void,
    }

    #[repr(C)]
    struct EXCEPTION_RECORD {
        exception_code: u32,
        exception_flags: u32,
        exception_record: *const EXCEPTION_RECORD,
        exception_address: *const std::ffi::c_void,
        number_parameters: u32,
        __alignment: u32,
        exception_information: [usize; 15],
    }

    unsafe extern "system" fn seh_handler(
        info: *const EXCEPTION_POINTERS,
    ) -> i32 {
        let rec = &*(*info).exception_record;
        let code = rec.exception_code;
        let addr = rec.exception_address as usize;

        let code_name = match code {
            0xC0000005 => "ACCESS_VIOLATION",
            0xC0000094 => "INTEGER_DIVIDE_BY_ZERO",
            0xC0000096 => "PRIVILEGED_INSTRUCTION",
            0xC00000FD => "STACK_OVERFLOW",
            0xC0000135 => "DLL_NOT_FOUND",
            0xC0000006 => "IN_PAGE_ERROR",
            _ => "UNKNOWN",
        };

        let mut detail = format!("SEH EXCEPTION: {} (0x{:08X})\n  Address: 0x{:016X}", code_name, code, addr);

        if code == 0xC0000005 && rec.number_parameters >= 2 {
            let op = rec.exception_information[0];
            let fault_addr = rec.exception_information[1];
            let op_str = if op == 0 { "READ" } else if op == 1 { "WRITE" } else if op == 8 { "DEP" } else { "EXEC" };
            detail.push_str(&format!("\n  Fault: {} at 0x{:016X}", op_str, fault_addr));
        }

        app_log(&detail, LogLevel::Error);
        eprintln!("{}", detail);

        // Write a minidump via dbghelp.dll (loaded at runtime)
        write_minidump_runtime(info as *const std::ffi::c_void);

        // Show error dialog
        let msg = format!(
            "FileSync crashed.\n\n{}\nAddress: 0x{:016X}",
            code_name, addr
        );
        show_error_dialog(&msg);

        1 // EXCEPTION_EXECUTE_HANDLER
    }

    unsafe {
        let kernel32 = windows::Win32::System::LibraryLoader::GetModuleHandleW(
            windows::core::PCWSTR::null(),
        ).ok();

        if let Some(kernel32) = kernel32 {
            let name = std::ffi::CString::new("SetUnhandledExceptionFilter").unwrap();
            let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
                kernel32,
                windows::core::PCSTR::from_raw(name.as_ptr() as *const u8),
            );

            if let Some(proc) = proc {
                let set_filter: SetUnhandledExceptionFilterFn =
                    std::mem::transmute(proc);
                set_filter(seh_handler);
                app_log("SEH exception handler installed", LogLevel::Info);
            } else {
                app_log("SetUnhandledExceptionFilter not found", LogLevel::Error);
            }
        }
    }
}

#[cfg(not(windows))]
fn install_seh_handler() {}

/// Write a minidump by loading dbghelp.dll at runtime
#[cfg(windows)]
fn write_minidump_runtime(exception_info: *const std::ffi::c_void) {
    type MiniDumpWriteDumpFn = unsafe extern "system" fn(
        *mut std::ffi::c_void, u32, *mut std::ffi::c_void, u32,
        *const std::ffi::c_void, *const std::ffi::c_void, *const std::ffi::c_void,
    ) -> i32;

    let dll_name: Vec<u16> = "dbghelp.dll\0".encode_utf16().collect();
    let dbghelp = unsafe {
        windows::Win32::System::LibraryLoader::LoadLibraryW(
            windows::core::PCWSTR::from_raw(dll_name.as_ptr()),
        )
    };

    if let Ok(dbghelp) = dbghelp {
        let proc_name_c = std::ffi::CString::new("MiniDumpWriteDump").unwrap();
        let proc = unsafe {
            windows::Win32::System::LibraryLoader::GetProcAddress(
                dbghelp,
                windows::core::PCSTR::from_raw(proc_name_c.as_ptr() as *const u8),
            )
        };

        if let Some(proc) = proc {
            let write_dump: MiniDumpWriteDumpFn = unsafe { std::mem::transmute(proc) };
            let process = unsafe {
                windows::Win32::System::Threading::GetCurrentProcess()
            };
            let pid = std::process::id();

            // Open dump file via std::fs (simpler than CreateFileW)
            let dump_dir = std::env::var("LOCALAPPDATA")
                .map(|p| std::path::PathBuf::from(p).join("FileSync"))
                .unwrap_or_else(|_| std::path::PathBuf::from("."));
            let _ = std::fs::create_dir_all(&dump_dir);
            let dump_path = dump_dir.join(format!("crash_{}.dmp", pid));

            // Use raw CreateFileW to get a HANDLE
            use windows::Win32::Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, CREATE_ALWAYS,
            };
            let path_wide: Vec<u16> = dump_path
                .to_str()
                .unwrap_or("crash.dmp")
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            let file = unsafe {
                CreateFileW(
                    windows::core::PCWSTR::from_raw(path_wide.as_ptr()),
                    0xC0000000, // GENERIC_READ | GENERIC_WRITE
                    FILE_SHARE_READ,
                    None,
                    CREATE_ALWAYS,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                )
            };

            if let Ok(file) = file {
                let ret = unsafe {
                    write_dump(
                        process.0 as *mut std::ffi::c_void,
                        pid,
                        file.0 as *mut std::ffi::c_void,
                        0, // MiniDumpNormal
                        exception_info,
                        std::ptr::null(),
                        std::ptr::null(),
                    )
                };
                use windows::Win32::Foundation::CloseHandle;
                unsafe { let _ = CloseHandle(file); }

                if ret != 0 {
                    app_log(&format!("minidump written to: {}", dump_path.display()), LogLevel::Info);
                } else {
                    app_log("MiniDumpWriteDump failed", LogLevel::Error);
                }
            }
        } else {
            app_log("MiniDumpWriteDump not found in dbghelp.dll", LogLevel::Error);
        }
    } else {
        app_log("dbghelp.dll not available for minidump", LogLevel::Error);
    }
}

#[cfg(not(windows))]
fn write_minidump_runtime(_: *const std::ffi::c_void) {}

fn main() {
    // Install SEH handler FIRST — before any Windows API calls that might fault
    install_seh_handler();

    // Custom panic hook: log panics to file even in release builds (no console).
    // Without this, panics in the winit/eframe thread are invisible in release mode.
    std::panic::set_hook(Box::new(|info| {
        let msg = if let Some(s) = info.payload().downcast_ref::<&str>() {
            (*s).to_string()
        } else if let Some(s) = info.payload().downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        let loc = info
            .location()
            .map(|l| format!(" at {}:{}", l.file(), l.line()))
            .unwrap_or_default();
        crate::log::app_log(
            &format!("PANIC: {}{}", msg, loc),
            crate::log::LogLevel::Error,
        );
        // Also print to stderr so cmd.exe can capture it
        eprintln!("PANIC: {}{}", msg, loc);
    }));

    app_log("===== FileSync starting =====", LogLevel::Info);
    eprintln!("===== FileSync starting =====");

    let result = run();

    // In dev mode, pause before exit so user can read console errors
    #[cfg(debug_assertions)]
    if result.is_err() {
        use std::io::{stdin, stdout, Write};
        eprintln!("\n=== Press Enter to exit ===");
        let _ = stdout().flush();
        let mut buf = String::new();
        let _ = stdin().read_line(&mut buf);
    }

    // Ignore result; error already logged in run()
    let _ = result;
}

fn run() -> eframe::Result<()> {

    // Detect Safe Mode early
    let safe_mode = is_safe_mode();
    app_log(&format!("safe mode: {}", safe_mode), LogLevel::Info);
    eprintln!("safe mode: {}", safe_mode);

    // Single instance check
    app_log("checking single instance...", LogLevel::Info);
    let _mutex = match single_instance_guard() {
        Some(m) => {
            app_log("single instance check passed", LogLevel::Info);
            m
        }
        None => {
            app_log("another instance is running, exiting", LogLevel::Info);
            return Ok(());
        }
    };

    app_log("generating app icon...", LogLevel::Info);
    let icon_data = make_icon();

    // Safe mode compatibility:
    // wgpu requires DX12/Vulkan GPU drivers, unavailable in Safe Mode.
    // In safe mode, wgpu's DX12 backend may appear to initialize but then crashes
    // hard during the first present (GPU driver is VGA fallback), bypassing both
    // the Rust panic hook and SEH handler.  Skip wgpu entirely in safe mode.
    // glow requires OpenGL 2.0+; Safe Mode only provides OpenGL 1.1 natively.
    // Fallback: if Mesa's opengl32.dll + libgallium_wgl.dll are placed next to
    // the exe, use them for software rendering; otherwise try native glow.

    // Check if Mesa DLL is next to the exe (needed for safe mode and wgpu fallback)
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let mesa_dll = exe_dir.as_ref().map(|d| d.join("opengl32.dll"));
    let mesa_available = mesa_dll.as_ref().map_or(false, |d| d.exists());

    // Try wgpu only in normal mode — safe mode skips it to avoid a hard crash
    let wgpu_err_opt: Option<eframe::Error> = if !safe_mode {
        app_log("trying wgpu renderer...", LogLevel::Info);
        let native_options = eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_title("FileSync")
                .with_min_inner_size([800.0, 500.0])
                .with_inner_size([1000.0, 650.0])
                .with_icon(icon_data.clone()),
            ..Default::default()
        };
        match run_with_renderer(native_options, icon_data.clone(), safe_mode) {
            Ok(()) => {
                app_log("eframe exited normally", LogLevel::Info);
                return Ok(());
            }
            Err(e) => {
                app_log(&format!("wgpu failed: {}", e), LogLevel::Error);
                Some(e)
            }
        }
    } else {
        app_log("safe mode: skipping wgpu, using software renderer", LogLevel::Info);
        None
    };

    if mesa_available {
        let dll = mesa_dll.unwrap();
        app_log(
            &format!("Mesa DLL found at {}, using glow+mesa...", dll.display()),
            LogLevel::Info,
        );
        // Point glutin to Mesa's opengl32.dll. Since it's next to the exe,
        // LoadLibrary will also find libgallium_wgl.dll in the same directory.
        std::env::set_var("GALLIUM_DRIVER", "llvmpipe");
        std::env::set_var(
            "GLUTIN_WGL_OPENGL_DLL",
            dll.to_str().unwrap_or("opengl32.dll"),
        );
        let mesa_options = eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            viewport: egui::ViewportBuilder::default()
                .with_title("FileSync")
                .with_min_inner_size([800.0, 500.0])
                .with_inner_size([1000.0, 650.0])
                .with_icon(icon_data),
            ..Default::default()
        };
        match run_with_renderer(mesa_options, make_icon(), safe_mode) {
            Ok(()) => {
                app_log("glow+mesa exited normally", LogLevel::Info);
                Ok(())
            }
            Err(mesa_err) => {
                app_log(&format!("glow+mesa failed: {}", mesa_err), LogLevel::Error);
                let msg = match &wgpu_err_opt {
                    Some(wgpu_err) => format!(
                        "{}\n\nwgpu: {}\nmesa: {}",
                        i18n::t(
                            "图形渲染初始化失败。\nMesa 软件渲染器也无法启动。",
                            "Failed to initialize graphics rendering.\nMesa software renderer also failed."
                        ),
                        wgpu_err, mesa_err
                    ),
                    None => format!(
                        "{}\n\nmesa: {}",
                        i18n::t(
                            "图形渲染初始化失败。\nMesa 软件渲染器无法启动。",
                            "Failed to initialize graphics rendering.\nMesa software renderer failed."
                        ),
                        mesa_err
                    ),
                };
                app_log(&msg, LogLevel::Error);
                show_error_dialog(&msg);
                Err(mesa_err)
            }
        }
    } else {
        // No Mesa available — try native glow as last resort
        app_log("falling back to glow (native OpenGL)...", LogLevel::Info);
        let glow_options = eframe::NativeOptions {
            renderer: eframe::Renderer::Glow,
            viewport: egui::ViewportBuilder::default()
                .with_title("FileSync")
                .with_min_inner_size([800.0, 500.0])
                .with_inner_size([1000.0, 650.0])
                .with_icon(icon_data.clone()),
            ..Default::default()
        };
        match run_with_renderer(glow_options, make_icon(), safe_mode) {
            Ok(()) => {
                app_log("glow (native) exited normally", LogLevel::Info);
                Ok(())
            }
            Err(glow_err) => {
                app_log(&format!("glow (native) failed: {}", glow_err), LogLevel::Error);
                let msg = match &wgpu_err_opt {
                    Some(wgpu_err) => format!(
                        "{}\n\nwgpu: {}\nglow: {}",
                        i18n::t(
                            "图形渲染初始化失败。\n请检查 GPU 驱动或尝试在安全模式下运行。",
                            "Failed to initialize graphics rendering.\nCheck GPU drivers or try running in Safe Mode."
                        ),
                        wgpu_err, glow_err
                    ),
                    None => format!(
                        "{}\n\nglow: {}",
                        i18n::t(
                            "图形渲染初始化失败（安全模式）。\n请将 Mesa opengl32.dll 放置在程序目录以启用软件渲染。",
                            "Failed to initialize graphics rendering (Safe Mode).\nPlace Mesa opengl32.dll next to the exe to enable software rendering."
                        ),
                        glow_err
                    ),
                };
                app_log(&msg, LogLevel::Error);
                show_error_dialog(&msg);
                Err(glow_err)
            }
        }
    }
}

fn run_with_renderer(
    options: eframe::NativeOptions,
    icon_data: egui::IconData,
    safe_mode: bool,
) -> eframe::Result<()> {
    let renderer_name = match options.renderer {
        eframe::Renderer::Glow => "glow",
        eframe::Renderer::Wgpu => "wgpu",
    };
    app_log(&format!("run_with_renderer: {}", renderer_name), LogLevel::Info);

    let result = eframe::run_native(
        "FileSync",
        options,
        Box::new(move |cc| {
            app_log(&format!("{} app creator invoked", renderer_name), LogLevel::Info);
            let tray = if safe_mode {
                app_log("safe mode detected, skipping system tray", LogLevel::Info);
                None
            } else {
                let t = tray::AppTray::new(icon_data.rgba, icon_data.width, icon_data.height);
                if t.is_some() {
                    app_log("tray icon created successfully", LogLevel::Info);
                } else {
                    app_log("tray icon creation returned None", LogLevel::Info);
                }
                t
            };
            Ok(Box::new(app::FileSyncApp::new(cc, tray)))
        }),
    );

    match &result {
        Ok(()) => app_log(&format!("{} exited normally", renderer_name), LogLevel::Info),
        Err(e) => app_log(&format!("{} error: {}", renderer_name, e), LogLevel::Error),
    }
    result
}

#[cfg(windows)]
fn show_error_dialog(msg: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{
        MessageBoxW, MB_ICONERROR, MB_OK, HWND_DESKTOP,
    };
    let wide: Vec<u16> = msg.encode_utf16().chain(std::iter::once(0)).collect();
    let title: Vec<u16> = "FileSync"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        MessageBoxW(
            HWND_DESKTOP,
            windows::core::PCWSTR::from_raw(wide.as_ptr()),
            windows::core::PCWSTR::from_raw(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

// ─────────────────────────────────────────────────────────────────
// Single instance guard
// ─────────────────────────────────────────────────────────────────

fn single_instance_guard() -> Option<Box<dyn std::any::Any>> {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
        use windows::Win32::System::Threading::CreateMutexW;
        use windows::Win32::UI::WindowsAndMessaging::{
            MessageBoxW, MB_ICONINFORMATION, MB_OK, HWND_DESKTOP,
        };

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
                    let msg_str = i18n::t(
                        "FileSync 已在运行中。\n请查看系统托盘图标。",
                        "FileSync is already running.\nCheck the system tray icon.",
                    );
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
            Err(e) => {
                app_log(&format!("CreateMutexW failed: {}, allowing startup", e), LogLevel::Error);
                Some(Box::new(()) as Box<dyn std::any::Any>)
            }
        }
    }
    #[cfg(not(windows))]
    {
        Some(Box::new(()) as Box<dyn std::any::Any>)
    }
}

/// Programmatically generate 32x32 sync icon (blue circle + white arc arrows)
fn make_icon() -> egui::IconData {

    const S: u32 = 32;
    let mut rgba = vec![0u8; (S * S * 4) as usize];

    for py in 0..S {
        for px in 0..S {
            let dx = px as f32 + 0.5 - 15.5;
            let dy = py as f32 + 0.5 - 15.5;
            let r = (dx * dx + dy * dy).sqrt();
            let a = dy.atan2(dx).to_degrees();

            let i = ((py * S + px) * 4) as usize;

            // Blue circle background
            if r <= 15.0 {
                rgba[i]     = 38;
                rgba[i + 1] = 110;
                rgba[i + 2] = 200;
                rgba[i + 3] = 255;
            }

            // White sync arcs: radius 8.5..12.5
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

    // Arc A arrow tip (+25 deg)
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

    // Arc B arrow tip (-120 deg)
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

/// Detect Windows Safe Mode by checking `GetSystemMetrics(SM_CLEANBOOT)`.
/// Returns true in Safe Mode (1 = Safe Mode, 2 = Safe Mode with Networking).
#[cfg(windows)]
fn is_safe_mode() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SYSTEM_METRICS_INDEX};
    unsafe { GetSystemMetrics(SYSTEM_METRICS_INDEX(67)) != 0 }
}

#[cfg(not(windows))]
fn is_safe_mode() -> bool {
    false
}
