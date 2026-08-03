use gtk4::prelude::*;
use libadwaita as adw;

use crate::audio::hz_to_note;

pub fn create_tuner_window(
    builder: &gtk4::Builder,
    mut tuner_hz_rx: futures_channel::mpsc::UnboundedReceiver<f32>,
) -> adw::Window {
    let window: adw::Window = builder.object("tuner_window").expect("tuner_window");
    let note_label: gtk4::Label = builder
        .object("tuner_note_label")
        .expect("tuner_note_label");
    let cents_label: gtk4::Label = builder
        .object("tuner_cents_label")
        .expect("tuner_cents_label");
    let hz_label: gtk4::Label = builder.object("tuner_hz_label").expect("tuner_hz_label");

    window.connect_hide({
        let note_label = note_label.clone();
        let cents_label = cents_label.clone();
        let hz_label = hz_label.clone();
        move |_| {
            note_label.set_text("--");
            cents_label.set_text("");
            hz_label.set_text("");
            note_label.remove_css_class("success");
        }
    });

    glib::MainContext::default().spawn_local(async move {
        use futures_util::StreamExt;
        while let Some(hz) = tuner_hz_rx.next().await {
            if let Some((name, cents)) = hz_to_note(hz) {
                note_label.set_text(&name);
                cents_label.set_text(&format!("{:+.0} cents", cents));
                hz_label.set_text(&format!("{hz:.1} Hz"));
                if cents.abs() <= 5.0 {
                    note_label.add_css_class("success");
                } else {
                    note_label.remove_css_class("success");
                }
            } else {
                note_label.set_text("--");
                cents_label.set_text("");
                hz_label.set_text("");
                note_label.remove_css_class("success");
            }
        }
    });

    window
}
