//! Application entry point: builds the window and wires GSettings changes
//! to the audio engine.

mod audio;
mod preset;
mod ui;

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use audio::{AudioEngine, EqPosition, InitialParams};
use ui::{
    bind_adjustment, bind_toggle, create_tuner_window, path_from_settings, restore_window_state,
    save_window_state, setup_eq_position, setup_file_picker_row, setup_preset_actions,
    setup_reset_button, FilePickerSpec,
};

const APP_ID: &str = "io.github.hedgieinsocks.Namplay";
const UI: &str = include_str!(concat!(env!("OUT_DIR"), "/window.ui"));
const TARGET_LOUDNESS_LUFS: f64 = -18.0;

/// File-backed rows; widget ids are derived from `prefix` (see `FilePickerSpec`).
const FILE_PICKERS: &[FilePickerSpec] = &[
    FilePickerSpec {
        prefix: "pedal_profile",
        key: "pedal-profile-path",
        title: "Choose Pedal Profile",
        filter_name: "NAM Profiles",
        filter_suffix: "nam",
    },
    FilePickerSpec {
        prefix: "amp_profile",
        key: "amp-profile-path",
        title: "Choose Amp Profile",
        filter_name: "NAM Profiles",
        filter_suffix: "nam",
    },
    FilePickerSpec {
        prefix: "ir",
        key: "ir-path",
        title: "Choose Impulse Response",
        filter_name: "WAV Files",
        filter_suffix: "wav",
    },
];

/// Settings keys of the sliders; the matching adjustment and reset button ids
/// are `{key with - replaced by _}_adjustment` / `..._reset_button`.
const SLIDER_KEYS: &[&str] = &[
    "noise-gate-threshold",
    "pedal-profile-input",
    "pedal-profile-output",
    "amp-profile-input",
    "amp-profile-output",
    "ir-level",
    "eq-hp",
    "eq-low",
    "eq-mid",
    "eq-high",
    "eq-lp",
];

/// ExpanderRows collapsed on launch when "collapse-on-launch" is enabled.
const EXPANDER_ROW_IDS: &[&str] = &[
    "noise_gate_row",
    "eq_row",
    "pedal_profile_row",
    "amp_profile_row",
    "ir_row",
];

fn main() {
    env_logger::init();
    let app = adw::Application::builder().application_id(APP_ID).build();

    app.connect_activate(build_ui);

    std::process::exit(app.run().into());
}

fn build_ui(app: &adw::Application) {
    if let Some(win) = app.active_window() {
        win.present();
        return;
    }

    let builder = gtk4::Builder::from_string(UI);
    let win: adw::ApplicationWindow = builder.object("window").expect("window not found");
    win.set_application(Some(app));

    let settings = gio::Settings::new(APP_ID);

    restore_window_state(&win, &settings);

    for spec in FILE_PICKERS {
        setup_file_picker_row(&builder, &win, &settings, spec);
    }

    bind_toggle(&builder, &settings, "noise_gate_row", "noise-gate-enabled");
    bind_toggle(&builder, &settings, "eq_row", "eq-enabled");

    for key in SLIDER_KEYS {
        let id_base = key.replace('-', "_");
        bind_adjustment(&builder, &settings, &format!("{id_base}_adjustment"), key);
        setup_reset_button(&builder, &settings, &format!("{id_base}_reset_button"), key);
    }

    setup_eq_position(&builder, &settings);

    match AudioEngine::new(InitialParams {
        gate_enabled: settings.boolean("noise-gate-enabled"),
        gate_threshold_db: settings.double("noise-gate-threshold") as f32,
        pedal_profile_path: path_from_settings(&settings, "pedal-profile-path"),
        pedal_in_gain_db: settings.double("pedal-profile-input") as f32,
        pedal_out_gain_db: settings.double("pedal-profile-output") as f32,
        amp_profile_path: path_from_settings(&settings, "amp-profile-path"),
        amp_in_gain_db: settings.double("amp-profile-input") as f32,
        amp_out_gain_db: settings.double("amp-profile-output") as f32,
        ir_path: path_from_settings(&settings, "ir-path"),
        ir_level_db: settings.double("ir-level") as f32,
        eq_enabled: settings.boolean("eq-enabled"),
        eq_pos: EqPosition::from_setting(settings.string("eq-position").as_str()),
        eq_low_db: settings.double("eq-low") as f32,
        eq_mid_db: settings.double("eq-mid") as f32,
        eq_high_db: settings.double("eq-high") as f32,
        eq_hp_freq: settings.double("eq-hp") as f32,
        eq_lp_freq: settings.double("eq-lp") as f32,
    }) {
        Ok(engine) => {
            wire_toggle_button(&builder, "mute_button", "MUTE", Arc::clone(&engine.mute));
            wire_toggle_button(
                &builder,
                "pedal_profile_bypass_button",
                "PEDAL",
                Arc::clone(&engine.pedal_bypass),
            );
            wire_toggle_button(
                &builder,
                "amp_profile_bypass_button",
                "AMP",
                Arc::clone(&engine.amp_bypass),
            );
            wire_toggle_button(
                &builder,
                "ir_bypass_button",
                "IR",
                Arc::clone(&engine.ir_bypass),
            );

            let tuner_window = create_tuner_window(&builder, Arc::clone(&engine.tuner_hz));
            let tuner_window_ref = tuner_window.clone();

            let tuner_enabled_btn = Arc::clone(&engine.tuner_enabled);
            let tuner_enabled_win = Arc::clone(&engine.tuner_enabled);

            let tuner_btn: gtk4::ToggleButton =
                builder.object("tuner_button").expect("tuner_button");
            let tuner_btn_ref = tuner_btn.clone();

            tuner_btn.connect_toggled(move |btn| {
                let active = btn.is_active();
                debug!("TUNER: state={}", if active { "on" } else { "off" });
                tuner_enabled_btn.store(active, Ordering::Relaxed);
                if active {
                    tuner_window.present();
                } else {
                    tuner_window.set_visible(false);
                }
            });

            tuner_window_ref.connect_visible_notify(move |win| {
                if !win.is_visible() {
                    tuner_enabled_win.store(false, Ordering::Relaxed);
                    tuner_btn_ref.set_active(false);
                }
            });

            wire_normalize_button(
                &builder,
                "pedal_profile_output_normalize_button",
                Arc::clone(&engine.pedal_loudness),
                settings.clone(),
                "pedal-profile-output",
            );
            wire_normalize_button(
                &builder,
                "amp_profile_output_normalize_button",
                Arc::clone(&engine.amp_loudness),
                settings.clone(),
                "amp-profile-output",
            );

            settings.connect_changed(None, move |s, key| match key {
                "noise-gate-enabled" => engine.set_gate_enabled(s.boolean(key)),
                "noise-gate-threshold" => engine.set_gate_threshold_db(s.double(key) as f32),
                "pedal-profile-path" => engine.load_pedal_profile(path_from_settings(s, key)),
                "pedal-profile-input" => engine.set_pedal_in_gain_db(s.double(key) as f32),
                "pedal-profile-output" => engine.set_pedal_out_gain_db(s.double(key) as f32),
                "amp-profile-path" => engine.load_amp_profile(path_from_settings(s, key)),
                "amp-profile-input" => engine.set_amp_in_gain_db(s.double(key) as f32),
                "amp-profile-output" => engine.set_amp_out_gain_db(s.double(key) as f32),
                "ir-path" => engine.load_ir(path_from_settings(s, key)),
                "ir-level" => engine.set_ir_level_db(s.double(key) as f32),
                "eq-enabled" => engine.set_eq_enabled(s.boolean(key)),
                "eq-position" => {
                    engine.set_eq_pos(EqPosition::from_setting(s.string(key).as_str()))
                }
                "eq-low" => engine.set_eq_low_db(s.double(key) as f32),
                "eq-mid" => engine.set_eq_mid_db(s.double(key) as f32),
                "eq-high" => engine.set_eq_high_db(s.double(key) as f32),
                "eq-hp" => engine.set_eq_hp_freq(s.double(key) as f32),
                "eq-lp" => engine.set_eq_lp_freq(s.double(key) as f32),
                _ => {}
            });
        }
        Err(e) => {
            error!("audio unavailable: {e}");
            let toast_overlay: adw::ToastOverlay =
                builder.object("toast_overlay").expect("toast_overlay");
            toast_overlay.add_toast(adw::Toast::new("Audio unavailable"));
        }
    }

    let settings_clone = settings.clone();
    win.connect_close_request(move |w| {
        save_window_state(w, &settings_clone);
        glib::Propagation::Proceed
    });

    app.add_action(&settings.create_action("collapse-on-launch"));

    if settings.boolean("collapse-on-launch") {
        for id in EXPANDER_ROW_IDS {
            let row: adw::ExpanderRow = builder.object(*id).expect(id);
            row.set_expanded(false);
        }
    }

    let browse_action = gio::ActionEntry::builder("browse-profiles")
        .activate(|app: &adw::Application, _, _| {
            gtk4::UriLauncher::new("https://www.tone3000.com/search").launch(
                app.active_window().as_ref(),
                None::<&gio::Cancellable>,
                |_| {},
            );
        })
        .build();

    let usage_action = gio::ActionEntry::builder("usage-guide")
        .activate(|app: &adw::Application, _, _| {
            gtk4::UriLauncher::new("https://github.com/hedgieinsocks/namplay#usage").launch(
                app.active_window().as_ref(),
                None::<&gio::Cancellable>,
                |_| {},
            );
        })
        .build();

    let about_action = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| {
            let about = adw::AboutWindow::builder()
                .application_name("Namplay")
                .application_icon(APP_ID)
                .version(env!("CARGO_PKG_VERSION"))
                .developer_name("Run A2 Neural Amp Modeler profiles via PipeWire (JACK)")
                .developers(["Claude", "hedgieinsocks", "Namplay contributors"])
                .license_type(gtk4::License::MitX11)
                .website("https://github.com/hedgieinsocks/namplay")
                .issue_url("https://github.com/hedgieinsocks/namplay/issues")
                .modal(true)
                .build();
            about.set_transient_for(app.active_window().as_ref());
            about.present();
        })
        .build();

    app.add_action_entries([browse_action, usage_action, about_action]);

    setup_preset_actions(&builder, &win, &settings, app);

    win.present();
}

fn wire_toggle_button(
    builder: &gtk4::Builder,
    id: &str,
    label: &'static str,
    flag: Arc<AtomicBool>,
) {
    let btn: gtk4::ToggleButton = builder.object(id).expect(id);
    btn.connect_toggled(move |btn| {
        let active = btn.is_active();
        debug!("{label}: {}", if active { "on" } else { "off" });
        btn.set_icon_name(if active {
            "audio-volume-muted-symbolic"
        } else {
            "audio-volume-high-symbolic"
        });
        flag.store(active, Ordering::Relaxed);
    });
}

fn wire_normalize_button(
    builder: &gtk4::Builder,
    id: &str,
    loudness: Arc<Mutex<Option<f32>>>,
    settings: gio::Settings,
    key: &'static str,
) {
    let btn: gtk4::Button = builder.object(id).expect(id);
    btn.connect_clicked(move |_| {
        if let Some(loudness) = *loudness.lock().unwrap() {
            let gain_db = (((TARGET_LOUDNESS_LUFS - loudness as f64) * 10.0).round() / 10.0)
                .clamp(-20.0, 20.0);
            let _ = settings.set_double(key, gain_db);
        }
    });
}
