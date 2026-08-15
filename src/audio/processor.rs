use std::collections::VecDeque;
use std::sync::{atomic::AtomicBool, atomic::Ordering, mpsc, Arc, Mutex};

use fft_convolver::FFTConvolver;
use jack::{AudioIn, AudioOut, Client, Control, NotificationHandler, ProcessHandler, ProcessScope};
use log::warn;
use nam_rs::Model;

use super::cab::CabConvolver;
use super::eq::{EqChannel, EqCoeffs};
use super::gate::Gate;
use super::EqPosition;

pub(super) struct Notifications;

impl NotificationHandler for Notifications {
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
    pub(super) pedal_capture_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_pedal_capture: Option<Model>,
    pub(super) amp_capture_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_amp_capture: Option<Model>,
    pub(super) cab_rx: mpsc::Receiver<Option<CabConvolver>>,
    pub(super) current_cab: Option<FFTConvolver<f32>>,
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
                apply_gain(out_l, p.pedal_input_gain);
                pedal.process_buffer(out_l);
                apply_gain(out_l, p.pedal_output_gain);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PreAmp {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        if !p.amp_bypass {
            if let Some(amp) = &mut self.current_amp_capture {
                apply_gain(out_l, p.amp_input_gain);
                amp.process_buffer(out_l);
                apply_gain(out_l, p.amp_output_gain);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PostAmp {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        let n = out_l.len().min(self.conv_buf.len());

        if !p.cab_bypass {
            if let Some(cab) = &mut self.current_cab {
                self.conv_buf[..n].copy_from_slice(&out_l[..n]);
                let _ = cab.process(&self.conv_buf[..n], &mut out_l[..n]);
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
