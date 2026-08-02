use std::sync::{mpsc, Arc, Mutex};

use futures_channel::mpsc::UnboundedSender;
use log::{debug, error, warn};
use nam_rs::{Model, NamModel};

pub(super) fn load(
    label: &'static str,
    tx: mpsc::Sender<Option<Model>>,
    path: Option<String>,
    sample_rate: u32,
    loudness_out: Arc<Mutex<Option<f32>>>,
    warning_tx: UnboundedSender<String>,
    loaded_tx: UnboundedSender<&'static str>,
) {
    std::thread::spawn(move || {
        let target = label.to_lowercase();
        let profile = match path {
            None => {
                debug!(target: &target, "NAM profile cleared");
                *loudness_out.lock().unwrap() = None;
                None
            }
            Some(p) => {
                debug!(target: &target, "loading NAM profile: {p}");
                let model = NamModel::from_file(&p).ok().and_then(|nm| {
                    let model_sr = nm.expected_sample_rate() as u32;
                    if model_sr != sample_rate {
                        let detail = format!(
                            "NAM profile sample rate {model_sr}Hz != JACK sample rate {sample_rate}Hz"
                        );
                        warn!(target: &target, "{detail}");
                        let _ = warning_tx.unbounded_send(format!("{label}: {detail}"));
                    }
                    let loudness = nm.loudness();
                    *loudness_out.lock().unwrap() = loudness;
                    if loudness.is_some() {
                        let _ = loaded_tx.unbounded_send(label);
                    }
                    Model::from_nam(&nm).ok()
                });
                if model.is_some() {
                    debug!(target: &target, "NAM profile loaded: {p}");
                } else {
                    let detail = format!("failed to load NAM profile: {p}");
                    error!(target: &target, "{detail}");
                    let _ = warning_tx.unbounded_send(format!("{label}: {detail}"));
                    *loudness_out.lock().unwrap() = None;
                }
                model
            }
        };
        let _ = tx.send(profile);
    });
}
