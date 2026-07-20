//! Pitch detection thread: feeds audio samples from the real-time callback
//! through the McLeod detector and writes the result to a shared atomic Hz value.

use std::collections::VecDeque;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};

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
) -> Arc<super::AtomicF32> {
    let hz_out = Arc::new(super::AtomicF32::new(0.0));
    let hz = Arc::clone(&hz_out);

    std::thread::Builder::new()
        .name("tuner".into())
        .spawn(move || {
            let mut detector = McLeodDetector::new(DETECTION_SIZE, DETECTION_PADDING);
            loop {
                std::thread::sleep(std::time::Duration::from_millis(80));

                if shutdown.load(Ordering::Relaxed) {
                    break;
                }

                if !enabled.load(Ordering::Relaxed) {
                    hz.set(0.0);
                    continue;
                }

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

                hz.set(detected);
            }
        })
        .expect("tuner thread spawn failed");

    hz_out
}
