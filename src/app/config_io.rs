use crate::app::strings;
use crate::model::config::AppConfig;

pub(super) fn export_config(config: &AppConfig) {
    let json = match serde_json::to_string_pretty(config) {
        Ok(json) => json,
        Err(_) => return,
    };
    if let Some(path) = rfd::FileDialog::new()
        .set_title(strings::export_config_dialog_title())
        .add_filter(strings::json_config_filter(), &["json"])
        .set_file_name("filesync_config.json")
        .save_file()
    {
        let _ = std::fs::write(path, json);
    }
}

pub(super) fn import_config() -> Option<AppConfig> {
    let path = rfd::FileDialog::new()
        .set_title(strings::import_config_dialog_title())
        .add_filter(strings::json_config_filter(), &["json"])
        .pick_file()?;
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}
