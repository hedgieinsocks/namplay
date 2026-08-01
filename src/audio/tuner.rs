use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

use futures_channel::mpsc::UnboundedSender;
use pitch_detection::detector::{mcleod::McLeodDetector, PitchDetector};

const DETECTION_SIZE: usize = 2048;
const DETECTION_PADDING: usize = 256;
const POWER_THRESHOLD: f32 = 0.005;
const CLARITY_THRESHOLD: f32 = 0.70;

pub(super) const SAMPLE_BUFFER_MAX: usize = DETECTION_SIZE * 4;

pub(super) fn spawn(
    samples: Arc<Mutex<VecDeque<f32>>>,
    enabled: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    sample_rate: u32,
    hz_tx: UnboundedSender<f32>,
) {
    std::thread::Builder::new()
        .name("tuner".into())
        .spawn(move || {
            let mut detector = McLeodDetector::new(DETECTION_SIZE, DETECTION_PADDING);
            let mut was_enabled = false;
            loop {
                std::thread::sleep(std::time::Duration::from_millis(80));

                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                if !enabled.load(Ordering::Relaxed) {
                    if was_enabled {
                        let _ = hz_tx.unbounded_send(0.0);
                    }
                    was_enabled = false;
                    continue;
                }
                was_enabled = true;

                let window: Option<Vec<f32>> = {
                    let mut guard = samples.lock().unwrap();
                    if guard.len() >= DETECTION_SIZE {
                        let slice = guard.make_contiguous();
                        let start = slice.len() - DETECTION_SIZE;
                        Some(slice[start..].to_vec())
                    } else {
                        None
                    }
                };

                let Some(window) = window else { continue };

                let detected = detector
                    .get_pitch(
                        &window,
                        sample_rate as usize,
                        POWER_THRESHOLD,
                        CLARITY_THRESHOLD,
                    )
                    .map(|p| p.frequency)
                    .unwrap_or(0.0);

                let _ = hz_tx.unbounded_send(detected);
            }
        })
        .expect("tuner thread spawn failed");
}

pub(crate) fn hz_to_note(hz: f32) -> Option<(String, f32)> {
    if !(20.0..=8000.0).contains(&hz) {
        return None;
    }
    let midi_float = 69.0 + 12.0 * (hz / 440.0).log2();
    let midi_round = midi_float.round();
    let cents = (midi_float - midi_round) * 100.0;
    let midi_int = midi_round as i32;
    if !(21..=108).contains(&midi_int) {
        return None;
    }
    const NAMES: &[&str] = &[
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
    ];
    let octave = (midi_int / 12) - 1;
    let name = format!("{}{}", NAMES[(midi_int % 12) as usize], octave);
    Some((name, cents))
}
