use gio::prelude::*;
use gtk4::prelude::*;
use libadwaita::{self as adw, prelude::*};
use log::{debug, error};

use super::show_persistent_toast;
use crate::keys::*;
use crate::APP_ID;

const EXPANDER_ROW_IDS: &[&str] = &["gate_row", "eq_row", "pedal_row", "amp_row", "cab_row"];

pub fn setup_primary_menu(
    app: &adw::Application,
    builder: &gtk4::Builder,
    settings: &gio::Settings,
    toast_overlay: &adw::ToastOverlay,
) {
    app.add_action(&settings.create_action(COLLAPSE_ON_LAUNCH));
    app.add_action(&settings.create_action(RUN_IN_BACKGROUND));
    app.add_action(&settings.create_action(NORMALIZE_OUTPUT));

    if settings.boolean(COLLAPSE_ON_LAUNCH) {
        for id in EXPANDER_ROW_IDS {
            let row: adw::ExpanderRow = builder.object(*id).expect(id);
            row.set_expanded(false);
        }
    }

    if settings.boolean(RUN_IN_BACKGROUND) {
        request_background_permission(toast_overlay.clone());
    }
    settings.connect_changed(Some(RUN_IN_BACKGROUND), {
        let toast_overlay = toast_overlay.clone();
        move |s, key| {
            if s.boolean(key) {
                request_background_permission(toast_overlay.clone());
            }
        }
    });

    let audio_window: adw::Window = builder.object("audio_window").expect("audio_window");
    let settings_action = gio::ActionEntry::builder("settings")
        .activate(move |_: &adw::Application, _, _| {
            audio_window.present();
        })
        .build();

    let browse_action = gio::ActionEntry::builder("browse-profiles")
        .activate(|app: &adw::Application, _, _| {
            gtk4::UriLauncher::new("https://www.tone3000.com/search").launch(
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
                .developer_name("Run A2 Neural Amp Modeler profiles via PipeWire's JACK")
                .developers(["Claude", "hedgieinsocks", "contributors"])
                .license_type(gtk4::License::MitX11)
                .website("https://github.com/hedgieinsocks/namplay")
                .issue_url("https://github.com/hedgieinsocks/namplay/issues")
                .modal(true)
                .build();
            about.set_transient_for(app.active_window().as_ref());
            about.present();
        })
        .build();

    app.add_action_entries([settings_action, browse_action, about_action]);
}

fn request_background_permission(toast_overlay: adw::ToastOverlay) {
    glib::MainContext::default().spawn_local(async move {
        use ashpd::desktop::background::Background;
        let result = async {
            Background::request()
                .reason("Namplay keeps processing audio while the window is closed")
                .auto_start(false)
                .dbus_activatable(false)
                .send()
                .await?
                .response()
        }
        .await;
        match result {
            Ok(response) if response.run_in_background() => {
                debug!(target: "background", "state=granted");
            }
            Ok(_) => {
                error!(target: "background", "state=denied");
                show_persistent_toast(&toast_overlay, "Background: permission denied");
            }
            Err(e) => {
                error!(target: "background", "state=error reason={e}");
                show_persistent_toast(&toast_overlay, "Background: portal request failed");
            }
        }
    });
}
