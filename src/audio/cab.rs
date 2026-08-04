use std::sync::mpsc;

use fft_convolver::FFTConvolver;
use futures_channel::mpsc::UnboundedSender;
use log::warn;

use super::EngineEvent;

pub(super) type CabConvolver = FFTConvolver<f32>;

pub(super) fn spawn(
    tx: mpsc::Sender<Option<CabConvolver>>,
    path: Option<String>,
    sample_rate: u32,
    block_size: usize,
    event_tx: UnboundedSender<EngineEvent>,
) {
    let event_tx_for_load = event_tx.clone();
    super::spawn_background_load(
        "cab",
        tx,
        path,
        move |p| load(p, sample_rate, block_size, &event_tx_for_load),
        || {},
        |p| format!("Cab: failed to load file: {p}"),
        event_tx,
    );
}

fn load(
    path: &str,
    sample_rate: u32,
    block_size: usize,
    event_tx: &UnboundedSender<EngineEvent>,
) -> Option<CabConvolver> {
    let samples = load_wav_samples(path, sample_rate, event_tx)?;
    let mut conv = FFTConvolver::<f32>::default();
    conv.init(block_size, &samples).ok()?;
    Some(conv)
}

fn load_wav_samples(
    path: &str,
    jack_sample_rate: u32,
    event_tx: &UnboundedSender<EngineEvent>,
) -> Option<Vec<f32>> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.channels != 1 {
        warn!(target: "cab", "file_channels={}", spec.channels);
        let detail = format!("expected mono file, got {} channels", spec.channels);
        let _ = event_tx.unbounded_send(EngineEvent::Warning(format!("Cab: {detail}")));
        return None;
    }
    if spec.sample_rate != jack_sample_rate {
        warn!(
            target: "cab",
            "file_sample_rate={}Hz jack_sample_rate={}Hz",
            spec.sample_rate, jack_sample_rate
        );
        let detail = format!(
            "file sample rate {}Hz != JACK sample rate {}Hz",
            spec.sample_rate, jack_sample_rate
        );
        let _ = event_tx.unbounded_send(EngineEvent::Warning(format!("Cab: {detail}")));
    }
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => {
            let max = (1i64 << (spec.bits_per_sample - 1)) as f32;
            reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / max)
                .collect()
        }
    };
    Some(samples)
}
