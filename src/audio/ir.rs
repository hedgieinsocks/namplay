use log::warn;

pub(super) fn load_wav_channels(
    path: &str,
    jack_sample_rate: u32,
) -> Option<(Vec<f32>, Option<Vec<f32>>)> {
    let mut reader = hound::WavReader::open(path).ok()?;
    let spec = reader.spec();
    if spec.sample_rate != jack_sample_rate {
        warn!(
            "IR sample rate {} != JACK rate {}, pitch may differ",
            spec.sample_rate, jack_sample_rate
        );
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
