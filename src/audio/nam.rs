use std::sync::{mpsc, Arc, Mutex};

use futures_channel::mpsc::UnboundedSender;
use log::warn;
use nam_rs::{Model, NamModel};

use super::EngineEvent;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ProfileKind {
    Pedal,
    Amp,
}

impl ProfileKind {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Pedal => "Pedal",
            Self::Amp => "Amp",
        }
    }

    pub(super) fn target(self) -> &'static str {
        match self {
            Self::Pedal => "pedal",
            Self::Amp => "amp",
        }
    }
}

pub(super) fn load(
    kind: ProfileKind,
    tx: mpsc::Sender<Option<Model>>,
    path: Option<String>,
    sample_rate: u32,
    loudness_out: Arc<Mutex<Option<f32>>>,
    event_tx: UnboundedSender<EngineEvent>,
) {
    let target = kind.target();
    let label = kind.label();
    let event_tx_for_load = event_tx.clone();
    let loudness_for_load = Arc::clone(&loudness_out);
    super::spawn_background_load(
        target,
        tx,
        path,
        move |p| {
            NamModel::from_file(p).ok().and_then(|nm| {
                let model_sr = nm.expected_sample_rate() as u32;
                if model_sr != sample_rate {
                    warn!(
                        target: target,
                        "model_sample_rate={model_sr}Hz jack_sample_rate={sample_rate}Hz"
                    );
                    let detail = format!(
                        "NAM profile sample rate {model_sr}Hz != JACK sample rate {sample_rate}Hz"
                    );
                    let _ = event_tx_for_load
                        .unbounded_send(EngineEvent::Warning(format!("{label}: {detail}")));
                }
                let loudness = nm.loudness();
                *loudness_for_load.lock().unwrap() = loudness;
                if loudness.is_some() {
                    let _ = event_tx_for_load.unbounded_send(EngineEvent::ProfileLoaded(kind));
                }
                Model::from_nam(&nm).ok()
            })
        },
        move || *loudness_out.lock().unwrap() = None,
        move |p| format!("{label}: failed to load NAM profile: {p}"),
        event_tx,
    );
}
