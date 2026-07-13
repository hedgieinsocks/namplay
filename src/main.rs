mod audio;

use std::path::Path;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};

use audio::{AudioEngine, InitialParams};

const APP_ID: &str = "io.github.hedgieinsocks.Namplay";
const UI: &str = include_str!(concat!(env!("OUT_DIR"), "/window.ui"));

fn main() {
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

    setup_file_picker_row(
        &builder,
        &win,
        &settings,
        "pedal_profile_row",
        "pedal_profile_button",
        "pedal_profile_clear_button",
        "pedal-profile-path",
        "Choose Pedal Profile",
        "NAM Profiles",
        "nam",
    );

    setup_file_picker_row(
        &builder,
        &win,
        &settings,
        "amp_profile_row",
        "amp_profile_button",
        "amp_profile_clear_button",
        "amp-profile-path",
        "Choose Amp Profile",
        "NAM Profiles",
        "nam",
    );

    setup_file_picker_row(
        &builder,
        &win,
        &settings,
        "ir_row",
        "ir_button",
        "ir_clear_button",
        "ir-path",
        "Choose Impulse Response",
        "WAV Files",
        "wav",
    );

    bind_toggle(&builder, &settings, "noise_gate_row", "noise-gate-enabled");
    bind_adjustment(
        &builder,
        &settings,
        "noise_gate_threshold_adjustment",
        "noise-gate-threshold",
    );
    bind_adjustment(
        &builder,
        &settings,
        "pedal_profile_input_adjustment",
        "pedal-profile-input",
    );
    bind_adjustment(
        &builder,
        &settings,
        "pedal_profile_output_adjustment",
        "pedal-profile-output",
    );
    bind_adjustment(
        &builder,
        &settings,
        "amp_profile_input_adjustment",
        "amp-profile-input",
    );
    bind_adjustment(
        &builder,
        &settings,
        "amp_profile_output_adjustment",
        "amp-profile-output",
    );
    bind_adjustment(&builder, &settings, "ir_level_adjustment", "ir-level");

    setup_reset_button(
        &builder,
        &settings,
        "noise_gate_threshold_reset_button",
        "noise-gate-threshold",
    );
    setup_reset_button(
        &builder,
        &settings,
        "pedal_profile_input_reset_button",
        "pedal-profile-input",
    );
    setup_reset_button(
        &builder,
        &settings,
        "pedal_profile_output_reset_button",
        "pedal-profile-output",
    );
    setup_reset_button(
        &builder,
        &settings,
        "amp_profile_input_reset_button",
        "amp-profile-input",
    );
    setup_reset_button(
        &builder,
        &settings,
        "amp_profile_output_reset_button",
        "amp-profile-output",
    );
    setup_reset_button(&builder, &settings, "ir_level_reset_button", "ir-level");

    bind_toggle(&builder, &settings, "eq_row", "eq-enabled");
    bind_adjustment(&builder, &settings, "eq_hp_adjustment", "eq-hp");
    bind_adjustment(&builder, &settings, "eq_low_adjustment", "eq-low");
    bind_adjustment(&builder, &settings, "eq_mid_adjustment", "eq-mid");
    bind_adjustment(&builder, &settings, "eq_high_adjustment", "eq-high");
    bind_adjustment(&builder, &settings, "eq_lp_adjustment", "eq-lp");
    setup_reset_button(&builder, &settings, "eq_hp_reset_button", "eq-hp");
    setup_reset_button(&builder, &settings, "eq_low_reset_button", "eq-low");
    setup_reset_button(&builder, &settings, "eq_mid_reset_button", "eq-mid");
    setup_reset_button(&builder, &settings, "eq_high_reset_button", "eq-high");
    setup_reset_button(&builder, &settings, "eq_lp_reset_button", "eq-lp");
    setup_eq_position(&builder, &settings);

    match AudioEngine::new(InitialParams {
        gate_enabled: settings.boolean("noise-gate-enabled"),
        gate_threshold_db: settings.double("noise-gate-threshold") as f32,
        pedal_path: path_from_settings(&settings, "pedal-profile-path"),
        pedal_in_gain_db: settings.double("pedal-profile-input") as f32,
        pedal_out_gain_db: settings.double("pedal-profile-output") as f32,
        in_gain_db: settings.double("amp-profile-input") as f32,
        out_gain_db: settings.double("amp-profile-output") as f32,
        model_path: path_from_settings(&settings, "amp-profile-path"),
        ir_path: path_from_settings(&settings, "ir-path"),
        ir_level_db: settings.double("ir-level") as f32,
        eq_enabled: settings.boolean("eq-enabled"),
        eq_pos: match settings.string("eq-position").as_str() {
            "pre-pedal" => 0,
            "post-ir" => 2,
            _ => 1,
        },
        eq_low_db: settings.double("eq-low") as f32,
        eq_mid_db: settings.double("eq-mid") as f32,
        eq_high_db: settings.double("eq-high") as f32,
        eq_hp_freq: settings.double("eq-hp") as f32,
        eq_lp_freq: settings.double("eq-lp") as f32,
    }) {
        Ok(engine) => {
            // Keep engine alive inside the 'static closure owned by GSettings
            settings.connect_changed(None, move |s, key| match key {
                "pedal-profile-path" => engine.load_pedal_model(path_from_settings(s, key)),
                "pedal-profile-input" => engine.set_pedal_in_gain_db(s.double(key) as f32),
                "pedal-profile-output" => engine.set_pedal_out_gain_db(s.double(key) as f32),
                "amp-profile-path" => engine.load_model(path_from_settings(s, key)),
                "ir-path" => engine.load_ir(path_from_settings(s, key)),
                "ir-level" => engine.set_ir_level_db(s.double(key) as f32),
                "amp-profile-input" => engine.set_in_gain_db(s.double(key) as f32),
                "amp-profile-output" => engine.set_out_gain_db(s.double(key) as f32),
                "noise-gate-enabled" => engine.set_gate_enabled(s.boolean(key)),
                "noise-gate-threshold" => engine.set_gate_threshold_db(s.double(key) as f32),
                "eq-enabled" => engine.set_eq_enabled(s.boolean(key)),
                "eq-position" => engine.set_eq_pos(match s.string(key).as_str() {
                    "pre-pedal" => 0,
                    "post-ir" => 2,
                    _ => 1,
                }),
                "eq-low" => engine.set_eq_low_db(s.double(key) as f32),
                "eq-mid" => engine.set_eq_mid_db(s.double(key) as f32),
                "eq-high" => engine.set_eq_high_db(s.double(key) as f32),
                "eq-hp" => engine.set_eq_hp_freq(s.double(key) as f32),
                "eq-lp" => engine.set_eq_lp_freq(s.double(key) as f32),
                _ => {}
            });
        }
        Err(e) => eprintln!("audio unavailable: {e}"),
    }

    let settings_clone = settings.clone();
    win.connect_close_request(move |w| {
        save_window_state(w, &settings_clone);
        glib::Propagation::Proceed
    });

    app.add_action(&settings.create_action("collapse-on-launch"));

    if settings.boolean("collapse-on-launch") {
        for id in &[
            "noise_gate_row",
            "eq_row",
            "pedal_profile_row",
            "amp_profile_row",
            "ir_row",
        ] {
            let row: adw::ExpanderRow = builder.object(*id).expect(*id);
            row.set_expanded(false);
        }
    }

    let about_action = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| {
            let about = adw::AboutWindow::builder()
                .application_name("Namplay")
                .application_icon(APP_ID)
                .version("0.1.0")
                .developers(["hedgieinsocks"])
                .license_type(gtk4::License::MitX11)
                .website("https://github.com/hedgieinsocks/namplay")
                .issue_url("https://github.com/hedgieinsocks/namplay/issues")
                .modal(true)
                .build();
            about.set_transient_for(app.active_window().as_ref());
            about.present();
        })
        .build();
    app.add_action_entries([about_action]);

    win.present();
}

fn restore_window_state(win: &adw::ApplicationWindow, settings: &gio::Settings) {
    win.set_default_size(settings.int("window-width"), settings.int("window-height"));
    if settings.boolean("window-maximized") {
        win.maximize();
    }
}

fn save_window_state(win: &adw::ApplicationWindow, settings: &gio::Settings) {
    let _ = settings.set_boolean("window-maximized", win.is_maximized());
    if !win.is_maximized() {
        let (width, height) = win.default_size();
        let _ = settings.set_int("window-width", width);
        let _ = settings.set_int("window-height", height);
    }
}

fn path_from_settings(settings: &gio::Settings, key: &str) -> Option<String> {
    let p = settings.string(key);
    if p.is_empty() {
        None
    } else {
        Some(p.to_string())
    }
}

fn setup_file_picker_row(
    builder: &gtk4::Builder,
    win: &adw::ApplicationWindow,
    settings: &gio::Settings,
    row_id: &str,
    button_id: &str,
    clear_id: &str,
    path_key: &str,
    title: &str,
    filter_name: &str,
    filter_suffix: &str,
) {
    let row: adw::ExpanderRow = builder.object(row_id).expect(row_id);
    let button: gtk4::Button = builder.object(button_id).expect(button_id);
    let clear_button: gtk4::Button = builder.object(clear_id).expect(clear_id);

    let stored = settings.string(path_key);
    if !stored.is_empty() {
        let name = Path::new(stored.as_str())
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(stored.as_str());
        row.set_subtitle(name);
        row.set_enable_expansion(true);
        row.set_expanded(true);
    }

    let filter = gtk4::FileFilter::new();
    filter.set_name(Some(filter_name));
    filter.add_suffix(filter_suffix);

    let filters = gio::ListStore::new::<gtk4::FileFilter>();
    filters.append(&filter);

    let row_c = row.clone();
    let settings_c = settings.clone();
    let win_c = win.clone();
    let path_key_c = path_key.to_owned();
    let title_c = title.to_owned();
    let filter_c = filter.clone();
    let filters_c = filters.clone();

    button.connect_clicked(move |_| {
        let dialog = gtk4::FileDialog::new();
        dialog.set_title(&title_c);
        dialog.set_filters(Some(&filters_c));
        dialog.set_default_filter(Some(&filter_c));

        let row = row_c.clone();
        let settings = settings_c.clone();
        let path_key = path_key_c.clone();

        dialog.open(Some(&win_c), None::<&gio::Cancellable>, move |result| {
            if let Ok(file) = result {
                if let Some(path) = file.path() {
                    let _ = settings.set_string(&path_key, path.to_str().unwrap_or(""));
                    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    row.set_subtitle(name);
                    row.set_enable_expansion(true);
                    row.set_expanded(true);
                }
            }
        });
    });

    let path_key_c = path_key.to_owned();
    let settings_c = settings.clone();
    let row_c = row.clone();

    clear_button.connect_clicked(move |_| {
        settings_c.reset(&path_key_c);
        row_c.set_subtitle("No file selected");
        row_c.set_enable_expansion(false);
    });
}

fn bind_adjustment(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &str) {
    let adj: gtk4::Adjustment = builder.object(id).expect(id);
    settings.bind(key, &adj, "value").build();
}

fn bind_toggle(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &str) {
    let row: adw::ExpanderRow = builder.object(id).expect(id);
    settings.bind(key, &row, "enable-expansion").build();
}

fn setup_reset_button(builder: &gtk4::Builder, settings: &gio::Settings, id: &str, key: &str) {
    let btn: gtk4::Button = builder.object(id).expect(id);
    let settings = settings.clone();
    let key = key.to_owned();
    btn.connect_clicked(move |_| {
        settings.reset(&key);
    });
}

fn setup_eq_position(builder: &gtk4::Builder, settings: &gio::Settings) {
    let pre_pedal_btn: gtk4::ToggleButton = builder
        .object("eq_pre_pedal_button")
        .expect("eq_pre_pedal_button");
    let pre_amp_btn: gtk4::ToggleButton = builder
        .object("eq_pre_amp_button")
        .expect("eq_pre_amp_button");
    let post_ir_btn: gtk4::ToggleButton = builder
        .object("eq_post_ir_button")
        .expect("eq_post_ir_button");

    match settings.string("eq-position").as_str() {
        "pre-pedal" => pre_pedal_btn.set_active(true),
        "post-ir" => post_ir_btn.set_active(true),
        _ => pre_amp_btn.set_active(true),
    }

    let settings_c = settings.clone();
    pre_pedal_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = settings_c.set_string("eq-position", "pre-pedal");
        }
    });

    let settings_c = settings.clone();
    pre_amp_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = settings_c.set_string("eq-position", "pre-amp");
        }
    });

    let settings_c = settings.clone();
    post_ir_btn.connect_toggled(move |btn| {
        if btn.is_active() {
            let _ = settings_c.set_string("eq-position", "post-ir");
        }
    });
}
