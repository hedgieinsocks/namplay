//! Impulse response loading: WAV decode and FFT convolver setup.

use fft_convolver::FFTConvolver;
use futures_channel::mpsc::UnboundedSender;
use log::warn;

/// Left convolver, plus a right one when the IR file is stereo.
pub(super) type IrConvolvers = (FFTConvolver<f32>, Option<FFTConvolver<f32>>);

pub(super) fn load(
    path: &str,
    sample_rate: u32,
    block_size: usize,
    warning_tx: &UnboundedSender<String>,
) -> Option<IrConvolvers> {
    let (left, right) = load_wav_channels(path, sample_rate, warning_tx)?;
    let mut conv_l = FFTConvolver::<f32>::default();
    conv_l.init(block_size, &left).ok()?;
    let conv_r = right.and_then(|r| {
        let mut c = FFTConvolver::<f32>::default();
        c.init(block_size, &r).ok().map(|_| c)
    });
    Some((conv_l, conv_r))
}

fn load_wav_channels(
    path: &str,
    jack_sample_rate: u32,
    warning_tx: &UnboundedSender<String>,
) -> Option<(Vec<f32>, Option<Vec<f32>>)> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate != jack_sample_rate {
        let detail = format!(
            "file sample rate {}Hz != JACK sample rate {}Hz",
            spec.sample_rate, jack_sample_rate
        );
        warn!(target: "ir", "{detail}");
        let _ = warning_tx.unbounded_send(format!("IR: {detail}"));
    }
    let channels = spec.channels as usize;
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
    if channels == 1 {
        Some((samples, None))
    } else {
        let left: Vec<f32> = samples.chunks(channels).map(|c| c[0]).collect();
        let right: Vec<f32> = samples.chunks(channels).map(|c| c[1]).collect();
        Some((left, Some(right)))
    }
}
