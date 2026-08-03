use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_channel::mpsc::UnboundedSender;
use ksni::blocking::{Handle, TrayMethods};
use ksni::menu::StandardItem;
use ksni::{MenuItem, Tray};
use log::error;

use crate::APP_ID;

pub enum TrayCommand {
    ToggleWindow,
    ToggleMute,
    Quit,
}

pub struct TrayState {
    pub window_visible: Arc<AtomicBool>,
    pub mute: Arc<AtomicBool>,
    pub cmd_tx: UnboundedSender<TrayCommand>,
}

impl Tray for TrayState {
    const MENU_ON_ACTIVATE: bool = true;

    fn id(&self) -> String {
        APP_ID.into()
    }

    fn icon_name(&self) -> String {
        APP_ID.into()
    }

    fn title(&self) -> String {
        "Namplay".into()
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        let visible = self.window_visible.load(Ordering::Relaxed);
        let muted = self.mute.load(Ordering::Relaxed);
        vec![
            StandardItem {
                label: if visible { "Hide" } else { "Show" }.into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.cmd_tx.unbounded_send(TrayCommand::ToggleWindow);
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: if muted { "Unmute" } else { "Mute" }.into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.cmd_tx.unbounded_send(TrayCommand::ToggleMute);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|this: &mut Self| {
                    let _ = this.cmd_tx.unbounded_send(TrayCommand::Quit);
                }),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn spawn(state: TrayState) -> Option<Handle<TrayState>> {
    match state.disable_dbus_name(true).spawn() {
        Ok(handle) => Some(handle),
        Err(e) => {
            error!(target: "tray", "state=error reason={e}");
            None
        }
    }
}
