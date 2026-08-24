use std::collections::VecDeque;
use std::sync::{atomic::AtomicBool, atomic::Ordering, mpsc, Arc, Mutex};

use jack::{AudioIn, AudioOut, Client, Control, NotificationHandler, ProcessHandler, ProcessScope};
use log::warn;
use neural_amp_modeler_rs::math::common::set_daz_ftz;
use neural_amp_modeler_rs::models::NamModel;

use super::cab::CabConvolver;
use super::eq::{EqChannel, EqCoeffs};
use super::gate::Gate;
use super::nam::Capture;
use super::EqPosition;

pub(super) struct Notifications;

impl NotificationHandler for Notifications {
    fn thread_init(&self, _: &Client) {
        // MXCSR is per-thread; the DSP core requires DAZ/FTZ set on every
        // audio thread that calls NamModel::process directly (bypassing its pipeline).
        unsafe { set_daz_ftz() };
    }

    fn xrun(&mut self, _: &Client) -> Control {
        warn!(target: "jack", "xrun (buffer under/overrun)");
        Control::Continue
    }
}

#[derive(Clone, Copy)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct Params {
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_input_gain: f32,
    pub pedal_output_gain: f32,
    pub pedal_bypass: bool,
    pub amp_input_gain: f32,
    pub amp_output_gain: f32,
    pub amp_bypass: bool,
    pub cab_level_gain: f32,
    pub cab_bypass: bool,
    pub eq_enabled: bool,
    pub eq_pos: EqPosition,
    pub eq_low_db: f32,
    pub eq_mid_db: f32,
    pub eq_high_db: f32,
    pub eq_hp_freq: f32,
    pub eq_lp_freq: f32,
}

pub(super) struct NamProcessor {
    pub(super) mute: Arc<AtomicBool>,
    pub(super) gate: Gate,
    pub(super) pedal_capture_rx: mpsc::Receiver<Option<Capture>>,
    pub(super) current_pedal_capture: Option<Capture>,
    pub(super) amp_capture_rx: mpsc::Receiver<Option<Capture>>,
    pub(super) current_amp_capture: Option<Capture>,
    pub(super) cab_rx: mpsc::Receiver<Option<CabConvolver>>,
    pub(super) current_cab: Option<CabConvolver>,
    pub(super) params: Arc<Mutex<Params>>,
    pub(super) last_params: Params,
    pub(super) eq_coeffs: EqCoeffs,
    pub(super) eq: EqChannel,
    pub(super) conv_buf: Vec<f32>,
    pub(super) in_port: jack::Port<AudioIn>,
    pub(super) out_port_1: jack::Port<AudioOut>,
    pub(super) out_port_2: jack::Port<AudioOut>,
    pub(super) tuner_samples: Arc<Mutex<VecDeque<f32>>>,
    pub(super) tuner_enabled: Arc<AtomicBool>,
}

fn apply_gain(buf: &mut [f32], gain: f32) {
    for s in buf {
        *s *= gain;
    }
}

fn apply_capture(
    capture: &mut Capture,
    conv_buf: &mut [f32],
    out_l: &mut [f32],
    gains: (f32, f32),
) {
    let (input_gain, output_gain) = gains;
    apply_gain(out_l, input_gain);
    let n = out_l.len().min(conv_buf.len());
    conv_buf[..n].copy_from_slice(&out_l[..n]);
    capture.process(&conv_buf[..n], &mut out_l[..n]);
    apply_gain(out_l, output_gain);
}

impl ProcessHandler for NamProcessor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        const TUNER_SAMPLE_BUFFER_MAX: usize = super::tuner::SAMPLE_BUFFER_MAX;

        while let Ok(new_capture) = self.pedal_capture_rx.try_recv() {
            self.current_pedal_capture = new_capture;
        }
        while let Ok(new_capture) = self.amp_capture_rx.try_recv() {
            self.current_amp_capture = new_capture;
        }
        while let Ok(new_cab) = self.cab_rx.try_recv() {
            self.current_cab = new_cab;
        }

        let muted = self.mute.load(Ordering::Relaxed);

        if self.tuner_enabled.load(Ordering::Relaxed) {
            let input = self.in_port.as_slice(ps);
            if let Ok(mut guard) = self.tuner_samples.try_lock() {
                guard.extend(input.iter().copied());
                if guard.len() > TUNER_SAMPLE_BUFFER_MAX {
                    let excess = guard.len() - TUNER_SAMPLE_BUFFER_MAX;
                    guard.drain(..excess);
                }
            }
            let out_l = self.out_port_1.as_mut_slice(ps);
            let out_r = self.out_port_2.as_mut_slice(ps);
            if muted {
                out_l.fill(0.0);
                out_r.fill(0.0);
            } else {
                out_l.copy_from_slice(input);
                out_r.copy_from_slice(input);
            }
            return Control::Continue;
        }

        if muted {
            self.out_port_1.as_mut_slice(ps).fill(0.0);
            self.out_port_2.as_mut_slice(ps).fill(0.0);
            return Control::Continue;
        }

        if let Ok(guard) = self.params.try_lock() {
            self.last_params = *guard;
        }
        let p = self.last_params;

        if p.gate_enabled {
            self.gate.update(p.gate_threshold_db);
        }

        if p.eq_enabled {
            self.eq_coeffs.update(
                p.eq_low_db,
                p.eq_mid_db,
                p.eq_high_db,
                p.eq_hp_freq,
                p.eq_lp_freq,
            );
        }

        let input = self.in_port.as_slice(ps);
        let out_l = self.out_port_1.as_mut_slice(ps);
        let out_r = self.out_port_2.as_mut_slice(ps);

        for (o, &i) in out_l.iter_mut().zip(input) {
            *o = if p.gate_enabled {
                self.gate.process_sample(i)
            } else {
                i
            };
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PrePedal {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        if !p.pedal_bypass {
            if let Some(pedal) = &mut self.current_pedal_capture {
                let gains = (p.pedal_input_gain, p.pedal_output_gain);
                apply_capture(pedal, &mut self.conv_buf, out_l, gains);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PreAmp {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        if !p.amp_bypass {
            if let Some(amp) = &mut self.current_amp_capture {
                let gains = (p.amp_input_gain, p.amp_output_gain);
                apply_capture(amp, &mut self.conv_buf, out_l, gains);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PostAmp {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        let n = out_l.len().min(self.conv_buf.len());

        if !p.cab_bypass {
            if let Some(cab) = &mut self.current_cab {
                self.conv_buf[..n].copy_from_slice(&out_l[..n]);
                cab.process_variable(&self.conv_buf[..n], &mut out_l[..n], None);
                apply_gain(&mut out_l[..n], p.cab_level_gain);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PostCab {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        out_r.copy_from_slice(out_l);

        Control::Continue
    }
}
