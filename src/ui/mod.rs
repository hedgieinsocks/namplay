mod bindings;
mod file_picker;
mod menu;
mod preset_actions;
mod settings;
mod tuner;
mod window;

pub use bindings::{bind_adjustment, bind_toggle, setup_eq_position, setup_reset_button};
pub use file_picker::{setup_file_picker_row, FilePickerSpec};
pub use menu::setup_primary_menu;
pub use preset_actions::setup_preset_actions;
pub use settings::{setup_buffer_size_dropdown, setup_settings_window};
pub use tuner::setup_tuner_window;
pub use window::{restore_window_state, save_window_state};

use gio::prelude::*;
use libadwaita as adw;

pub fn show_persistent_toast(toast_overlay: &adw::ToastOverlay, msg: &str) {
    let toast = adw::Toast::new(msg);
    toast.set_timeout(0);
    toast_overlay.add_toast(toast);
}

pub fn path_from_settings(settings: &gio::Settings, key: &str) -> Option<String> {
    let p = settings.string(key);
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}
