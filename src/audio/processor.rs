use std::sync::{
    atomic::{AtomicBool, AtomicU32, Ordering},
    mpsc, Arc,
};

use fft_convolver::FFTConvolver;
use jack::{AudioIn, AudioOut, Client, Control, ProcessHandler, ProcessScope};
use nam_rs::Model;

use super::dsp::{AtomicF32, Eq, NoiseGate};

pub(super) struct NamProcessor {
    pub(super) pedal_profile_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_pedal_profile: Option<Model>,
    pub(super) pedal_in_gain: Arc<AtomicF32>,
    pub(super) pedal_out_gain: Arc<AtomicF32>,
    pub(super) amp_profile_rx: mpsc::Receiver<Option<Model>>,
    pub(super) current_profile: Option<Model>,
    pub(super) ir_rx: mpsc::Receiver<Option<(FFTConvolver<f32>, Option<FFTConvolver<f32>>)>>,
    pub(super) current_ir_l: Option<FFTConvolver<f32>>,
    pub(super) current_ir_r: Option<FFTConvolver<f32>>,
    pub(super) conv_buf: Vec<f32>,
    pub(super) noise_gate: Option<NoiseGate>,
    pub(super) amp_in_gain: Arc<AtomicF32>,
    pub(super) amp_out_gain: Arc<AtomicF32>,
    pub(super) ir_level: Arc<AtomicF32>,
    pub(super) gate_enabled: Arc<AtomicBool>,
    pub(super) gate_threshold_db: Arc<AtomicF32>,
    pub(super) last_gate_enabled: bool,
    pub(super) last_gate_threshold_db: f32,
    pub(super) eq_l: Option<Eq>,
    pub(super) eq_r: Option<Eq>,
    pub(super) eq_enabled: Arc<AtomicBool>,
    pub(super) eq_pos: Arc<AtomicU32>,
    pub(super) eq_low_db: Arc<AtomicF32>,
    pub(super) eq_mid_db: Arc<AtomicF32>,
    pub(super) eq_high_db: Arc<AtomicF32>,
    pub(super) eq_hp_freq: Arc<AtomicF32>,
    pub(super) eq_lp_freq: Arc<AtomicF32>,
    pub(super) last_eq_enabled: bool,
    pub(super) last_eq_pos: u32,
    pub(super) last_eq_low_db: f32,
    pub(super) last_eq_mid_db: f32,
    pub(super) last_eq_high_db: f32,
    pub(super) last_eq_hp_freq: f32,
    pub(super) last_eq_lp_freq: f32,
    pub(super) sample_rate: u32,
    pub(super) in_port: jack::Port<AudioIn>,
    pub(super) out_port_l: jack::Port<AudioOut>,
    pub(super) out_port_r: jack::Port<AudioOut>,
}

impl ProcessHandler for NamProcessor {
    fn process(&mut self, _: &Client, ps: &ProcessScope) -> Control {
        while let Ok(new_profile) = self.pedal_profile_rx.try_recv() {
            self.current_pedal_profile = new_profile;
        }
        while let Ok(new_profile) = self.amp_profile_rx.try_recv() {
            self.current_profile = new_profile;
        }
        while let Ok(new_ir) = self.ir_rx.try_recv() {
            match new_ir {
                Some((l, r)) => {
                    self.current_ir_l = Some(l);
                    self.current_ir_r = r;
                }
                None => {
                    self.current_ir_l = None;
                    self.current_ir_r = None;
                }
            }
        }

        let gate_enabled = self.gate_enabled.load(Ordering::Relaxed);
        let gate_threshold_db = self.gate_threshold_db.get();
        if gate_enabled != self.last_gate_enabled
            || gate_threshold_db != self.last_gate_threshold_db
        {
            self.last_gate_enabled = gate_enabled;
            self.last_gate_threshold_db = gate_threshold_db;
            self.noise_gate =
                gate_enabled.then(|| NoiseGate::new(gate_threshold_db, self.sample_rate));
        }

        let pedal_in_gain = self.pedal_in_gain.get();
        let pedal_out_gain = self.pedal_out_gain.get();
        let in_gain = self.amp_in_gain.get();
        let out_gain = self.amp_out_gain.get();
        let ir_level = self.ir_level.get();

        let eq_enabled = self.eq_enabled.load(Ordering::Relaxed);
        let eq_pos = self.eq_pos.load(Ordering::Relaxed);
        let eq_low_db = self.eq_low_db.get();
        let eq_mid_db = self.eq_mid_db.get();
        let eq_high_db = self.eq_high_db.get();
        let eq_hp_freq = self.eq_hp_freq.get();
        let eq_lp_freq = self.eq_lp_freq.get();
        let eq_params_changed = eq_low_db != self.last_eq_low_db
            || eq_mid_db != self.last_eq_mid_db
            || eq_high_db != self.last_eq_high_db
            || eq_hp_freq != self.last_eq_hp_freq
            || eq_lp_freq != self.last_eq_lp_freq
            || eq_pos != self.last_eq_pos;
        if eq_enabled != self.last_eq_enabled || (eq_enabled && eq_params_changed) {
            self.last_eq_enabled = eq_enabled;
            self.last_eq_pos = eq_pos;
            self.last_eq_low_db = eq_low_db;
            self.last_eq_mid_db = eq_mid_db;
            self.last_eq_high_db = eq_high_db;
            self.last_eq_hp_freq = eq_hp_freq;
            self.last_eq_lp_freq = eq_lp_freq;
            if !eq_enabled {
                self.eq_l = None;
                self.eq_r = None;
            } else if let Some(eq) = &mut self.eq_l {
                eq.update(eq_low_db, eq_mid_db, eq_high_db, eq_hp_freq, eq_lp_freq);
                if let Some(eq) = &mut self.eq_r {
                    eq.update(eq_low_db, eq_mid_db, eq_high_db, eq_hp_freq, eq_lp_freq);
                }
            } else {
                self.eq_l = Some(Eq::new(
                    eq_low_db,
                    eq_mid_db,
                    eq_high_db,
                    eq_hp_freq,
                    eq_lp_freq,
                    self.sample_rate as f32,
                ));
                self.eq_r = Some(Eq::new(
                    eq_low_db,
                    eq_mid_db,
                    eq_high_db,
                    eq_hp_freq,
                    eq_lp_freq,
                    self.sample_rate as f32,
                ));
            }
        }

        let input = self.in_port.as_slice(ps);
        let out_l = self.out_port_l.as_mut_slice(ps);
        let out_r = self.out_port_r.as_mut_slice(ps);

        if self.current_pedal_profile.is_none() && self.current_profile.is_none() {
            out_l.copy_from_slice(input);
            out_r.copy_from_slice(input);
        } else {
            // Gate + optional pre-pedal EQ
            for (o, &i) in out_l.iter_mut().zip(input) {
                let mut s = match &mut self.noise_gate {
                    Some(g) => g.process_sample(i),
                    None => i,
                };
                if eq_pos == 0 {
                    if let Some(eq) = &mut self.eq_l {
                        s = eq.process_sample(s);
                    }
                }
                *o = s;
            }
            if let Some(pedal) = &mut self.current_pedal_profile {
                for s in out_l.iter_mut() {
                    *s *= pedal_in_gain;
                }
                pedal.process_buffer(out_l);
                for s in out_l.iter_mut() {
                    *s *= pedal_out_gain;
                }
            }
            // Optional pre-amp EQ
            if eq_pos == 1 {
                if let Some(eq) = &mut self.eq_l {
                    for s in out_l.iter_mut() {
                        *s = eq.process_sample(*s);
                    }
                }
            }
            if let Some(amp) = &mut self.current_profile {
                for s in out_l.iter_mut() {
                    *s *= in_gain;
                }
                amp.process_buffer(out_l);
            }
            let n = out_l.len().min(self.conv_buf.len());
            let stereo_ir = if let Some(ir_l) = &mut self.current_ir_l {
                self.conv_buf[..n].copy_from_slice(&out_l[..n]);
                let _ = ir_l.process(&self.conv_buf[..n], &mut out_l[..n]);
                for s in out_l[..n].iter_mut() {
                    *s *= ir_level;
                }
                if let Some(ir_r) = &mut self.current_ir_r {
                    let _ = ir_r.process(&self.conv_buf[..n], &mut out_r[..n]);
                    for s in out_r[..n].iter_mut() {
                        *s *= ir_level;
                    }
                    true
                } else {
                    false
                }
            } else {
                false
            };
            // Optional post-IR EQ
            if eq_pos == 2 {
                if let Some(eq) = &mut self.eq_l {
                    for s in out_l.iter_mut() {
                        *s = eq.process_sample(*s);
                    }
                }
            }
            for s in out_l.iter_mut() {
                *s *= out_gain;
            }
            if stereo_ir {
                if eq_pos == 2 {
                    if let Some(eq) = &mut self.eq_r {
                        for s in out_r.iter_mut() {
                            *s = eq.process_sample(*s);
                        }
                    }
                }
                for s in out_r.iter_mut() {
                    *s *= out_gain;
                }
            } else {
                out_r.copy_from_slice(out_l);
            }
        }

        Control::Continue
    }
}
