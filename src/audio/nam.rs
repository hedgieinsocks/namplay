use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};

use futures_channel::mpsc::UnboundedSender;
use log::warn;
use neural_amp_modeler_rs::loader::{load_and_build_model, LoadOptions};
use neural_amp_modeler_rs::models::{NamModel, StaticModel};
use neural_amp_modeler_rs::SystemSnapshot;

use super::{EngineEvent, MAX_BLOCK_SIZE};

pub(super) type Capture = Box<StaticModel>;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CaptureKind {
    Pedal,
    Amp,
}

impl CaptureKind {
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
    kind: CaptureKind,
    tx: mpsc::Sender<Option<Capture>>,
    path: Option<String>,
    sample_rate: u32,
    loudness_out: Arc<Mutex<Option<f32>>>,
    event_tx: UnboundedSender<EngineEvent>,
) {
    let target = kind.target();
    let label = kind.label();
    let event_tx_for_load = event_tx.clone();
    let loudness_for_load = Arc::clone(&loudness_out);
    let system = SystemSnapshot::capture();
    super::spawn_background_load(
        target,
        tx,
        path,
        move |p| {
            let pair =
                load_and_build_model(Path::new(p), &system, false, LoadOptions::default()).ok()?;
            let model_sr = pair.sample_rate;
            if model_sr != sample_rate {
                warn!(
                    target: target,
                    "model_sample_rate={model_sr}Hz jack_sample_rate={sample_rate}Hz"
                );
                let detail = format!(
                    "NAM capture sample rate {model_sr}Hz != JACK sample rate {sample_rate}Hz"
                );
                let _ = event_tx_for_load
                    .unbounded_send(EngineEvent::Warning(format!("{label}: {detail}")));
            }
            let loudness = pair.loudness();
            *loudness_for_load.lock().unwrap() = loudness;
            let mut model = pair.model_l?;
            if model.reset(sample_rate, MAX_BLOCK_SIZE).is_err() {
                return None;
            }
            if loudness.is_some() {
                let _ = event_tx_for_load.unbounded_send(EngineEvent::CaptureLoaded(kind));
            }
            Some(model)
        },
        move || *loudness_out.lock().unwrap() = None,
        move |p| format!("{label}: failed to load NAM capture: {p}"),
        event_tx,
    );
}
