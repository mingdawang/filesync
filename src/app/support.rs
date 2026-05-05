use eframe::egui;

use crate::app::{strings, AppNotification, NotificationKind};

pub(super) fn open_parent_in_explorer(path: &str) {
    let p = std::path::Path::new(path);
    let dir = if p.is_dir() {
        p.to_path_buf()
    } else {
        p.parent()
            .map(|pp| pp.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    let _ = std::process::Command::new("explorer.exe").arg(dir).spawn();
}

pub(super) fn show_notification_overlay(
    ctx: &egui::Context,
    notif: &mut Option<AppNotification>,
) {
    let n = match notif {
        Some(n) => n,
        None => return,
    };

    let elapsed = n.created_at.elapsed().as_secs_f32();
    if elapsed >= 3.0 {
        *notif = None;
        return;
    }

    let remaining_secs = (3.0 - elapsed).ceil() as u32;

    let (icon, bg, accent) = match n.kind {
        NotificationKind::Success => (
            strings::notification_success_icon(),
            egui::Color32::from_rgb(25, 65, 25),
            egui::Color32::from_rgb(80, 200, 80),
        ),
        NotificationKind::Warning => (
            strings::notification_warning_icon(),
            egui::Color32::from_rgb(65, 55, 10),
            egui::Color32::from_rgb(220, 180, 40),
        ),
    };

    let title = format!("{} {}", icon, n.title);
    let body = n.body.clone();
    let mut should_dismiss = false;

    egui::Area::new("app_notification".into())
        .anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0])
        .order(egui::Order::Foreground)
        .show(ctx, |ui| {
            egui::Frame::none()
                .fill(bg)
                .rounding(egui::Rounding::same(8.0))
                .inner_margin(egui::Margin::symmetric(14.0, 10.0))
                .stroke(egui::Stroke::new(1.0, accent))
                .show(ui, |ui| {
                    ui.set_max_width(280.0);

                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&title).color(accent).strong());
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                if ui
                                    .add(
                                        egui::Label::new(
                                            egui::RichText::new(strings::close_overlay_button())
                                                .color(egui::Color32::GRAY),
                                        )
                                        .sense(egui::Sense::click()),
                                    )
                                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                                    .clicked()
                                {
                                    should_dismiss = true;
                                }
                            },
                        );
                    });

                    if !body.is_empty() {
                        ui.label(
                            egui::RichText::new(&body)
                                .small()
                                .color(egui::Color32::from_gray(200)),
                        );
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{}s", remaining_secs))
                                .small()
                                .color(egui::Color32::from_gray(150)),
                        );
                    });
                });
        });

    if should_dismiss {
        *notif = None;
    } else {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

pub(super) fn play_completion_sound() {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        unsafe extern "system" {
            fn MessageBeep(utype: u32) -> i32;
        }
        unsafe { MessageBeep(0x40) };
    }
}

pub(super) fn setup_fonts(ctx: &egui::Context) {
    const CANDIDATES: &[&str] = &[
        r"C:\Windows\Fonts\msyh.ttc",
        r"C:\Windows\Fonts\simhei.ttf",
        r"C:\Windows\Fonts\simsun.ttc",
    ];

    let mut fonts = egui::FontDefinitions::default();

    for path in CANDIDATES {
        if let Ok(data) = std::fs::read(path) {
            fonts.font_data.insert(
                "cjk_fallback".to_owned(),
                egui::FontData::from_owned(data),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("cjk_fallback".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("cjk_fallback".to_owned());
            break;
        }
    }

    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    use egui::{FontId, TextStyle};
    style.text_styles = [
        (TextStyle::Heading, FontId::proportional(24.0)),
        (TextStyle::Body, FontId::proportional(16.0)),
        (TextStyle::Monospace, FontId::monospace(15.0)),
        (TextStyle::Button, FontId::proportional(16.0)),
        (TextStyle::Small, FontId::proportional(13.0)),
    ]
    .into();
    ctx.set_style(style);
}
