mod audio;
mod profile;
mod ui;

use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::error;

use audio::{AudioEngine, InitialParams};
use ui::{
    bind_adjustment, bind_toggle, path_from_settings, restore_window_state, save_window_state,
    setup_eq_position, setup_file_picker_row, setup_preset_actions, setup_reset_button,
};

const APP_ID: &str = "io.github.hedgieinsocks.Namplay";
const UI: &str = include_str!(concat!(env!("OUT_DIR"), "/window.ui"));

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
        profile_path: path_from_settings(&settings, "amp-profile-path"),
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
            settings.connect_changed(None, move |s, key| match key {
                "pedal-profile-path" => engine.load_pedal_profile(path_from_settings(s, key)),
                "pedal-profile-input" => engine.set_pedal_in_gain_db(s.double(key) as f32),
                "pedal-profile-output" => engine.set_pedal_out_gain_db(s.double(key) as f32),
                "amp-profile-path" => engine.load_amp_profile(path_from_settings(s, key)),
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
    app.add_action_entries([browse_action, usage_action]);

    let about_action = gio::ActionEntry::builder("about")
        .activate(|app: &adw::Application, _, _| {
            let about = adw::AboutWindow::builder()
                .application_name("Namplay")
                .application_icon(APP_ID)
                .version("0.2.2")
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
    app.add_action_entries([about_action]);

    setup_preset_actions(&builder, &win, &settings, app);

    win.present();
}
