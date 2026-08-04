use std::collections::VecDeque;
use std::sync::{atomic::AtomicBool, atomic::Ordering, mpsc, Arc, Mutex};

use fft_convolver::FFTConvolver;
use jack::{AudioIn, AudioOut, Client, Control, ProcessHandler, ProcessScope};
use nam_rs::Model;

use super::cab::CabConvolver;
use super::eq::{EqChannel, EqCoeffs};
use super::gate::Gate;
use super::EqPosition;

#[derive(Clone, Copy)]
pub(crate) struct Params {
    pub gate_enabled: bool,
    pub gate_threshold_db: f32,
    pub pedal_in_gain: f32,
    pub pedal_out_gain: f32,
    pub amp_in_gain: f32,
    pub amp_out_gain: f32,
    pub cab_level: f32,
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
    pub(super) pedal_profile_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_pedal_profile: Option<Model>,
    pub(super) pedal_bypass: Arc<AtomicBool>,
    pub(super) amp_profile_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_amp_profile: Option<Model>,
    pub(super) amp_bypass: Arc<AtomicBool>,
    pub(super) cab_rx: mpsc::Receiver<Option<CabConvolver>>,
    pub(super) current_cab: Option<FFTConvolver<f32>>,
    pub(super) cab_bypass: Arc<AtomicBool>,
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
        while let Ok(new_profile) = self.pedal_profile_rx.try_recv() {
            self.current_pedal_profile = new_profile;
        }
        while let Ok(new_profile) = self.amp_profile_rx.try_recv() {
            self.current_amp_profile = new_profile;
        }
        while let Ok(new_cab) = self.cab_rx.try_recv() {
            self.current_cab = new_cab;
        }

        let muted = self.mute.load(Ordering::Relaxed);

        if self.tuner_enabled.load(Ordering::Relaxed) {
            let input = self.in_port.as_slice(ps);
            if let Ok(mut guard) = self.tuner_samples.try_lock() {
                guard.extend(input.iter().copied());
                const MAX: usize = super::tuner::SAMPLE_BUFFER_MAX;
                if guard.len() > MAX {
                    let excess = guard.len() - MAX;
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

        let pedal_bypass = self.pedal_bypass.load(Ordering::Relaxed);
        let amp_bypass = self.amp_bypass.load(Ordering::Relaxed);
        let cab_bypass = self.cab_bypass.load(Ordering::Relaxed);

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

        if !pedal_bypass {
            if let Some(pedal) = &mut self.current_pedal_profile {
                apply_gain(out_l, p.pedal_in_gain);
                pedal.process_buffer(out_l);
                apply_gain(out_l, p.pedal_out_gain);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PreAmp {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        if !amp_bypass {
            if let Some(amp) = &mut self.current_amp_profile {
                apply_gain(out_l, p.amp_in_gain);
                amp.process_buffer(out_l);
                apply_gain(out_l, p.amp_out_gain);
            }
        }

        let n = out_l.len().min(self.conv_buf.len());

        if !cab_bypass {
            if let Some(cab) = &mut self.current_cab {
                self.conv_buf[..n].copy_from_slice(&out_l[..n]);
                let _ = cab.process(&self.conv_buf[..n], &mut out_l[..n]);
                apply_gain(&mut out_l[..n], p.cab_level);
            }
        }

        if p.eq_enabled && p.eq_pos == EqPosition::PostCab {
            self.eq.process_buffer(out_l, &self.eq_coeffs);
        }

        out_r.copy_from_slice(out_l);

        Control::Continue
    }
}
