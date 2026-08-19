//! The ship's voice: pure DSP, no samples, no files (P2 applied to sound).
//!
//! Everything here is a function of sample rate, a seed, and the live
//! [`Levels`] driven from sim state — which makes the whole synthesiser
//! unit-testable without an audio device, the same way the sim is testable
//! without a window.
//!
//! Character brief: SID-chip grit, deep and guttural, metal cage not moving
//! house. Three voices and a hiss:
//!
//! - **engine** — saw/pulse sub-oscillator with pulse-width modulation and an
//!   amplitude growl. The pitch ladder is quantised to semitones with a short
//!   portamento: ramps *step*, chip-arpeggio style, instead of sliding like a
//!   car engine. That quantisation is the "bitesized" in the brief.
//! - **wind** — filtered noise driven by real dynamic pressure (½ρv² from the
//!   sim's own atmosphere), so it roars low-and-fast, thins with altitude, and
//!   is exactly absent in vacuum with no crossfade logic at all.
//! - **hull** — near-silence of space: a leaky-integrated brown-noise rumble
//!   plus a ~31 Hz drone, scaled by structural load. Pressure on metal.
//! - **brake** — band-passed retro-thruster hiss.

/// Control inputs, all unitless and pre-normalised by the caller.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Levels {
    /// Main-engine demand 0..1 (boost included by the caller).
    pub effort: f32,
    /// Normalised dynamic pressure 0..1 (½ρv² against a reference).
    pub wind_q: f32,
    /// How much of "space" surrounds the hull: 0 in thick air, 1 in vacuum.
    pub vacuum: f32,
    /// Structural load in g beyond weightlessness, 0..~4.
    pub load_g: f32,
    /// Air-brake engagement 0..1.
    pub brake: f32,
    /// Attitude-thruster demand 0..1 (largest torque axis): the RCS voice.
    /// Rolling is flying too, and a silent manoeuvre reads as a broken game.
    pub rcs: f32,
    /// Master gain 0..1.
    pub master: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            effort: 0.0,
            wind_q: 0.0,
            vacuum: 1.0,
            load_g: 0.0,
            brake: 0.0,
            rcs: 0.0,
            master: 0.8,
        }
    }
}

/// One-pole smoother: reaches ~63% of a step in `tau` seconds regardless of
/// sample rate, so parameter zipper noise cannot depend on the machine.
#[derive(Clone, Copy)]
struct Smooth {
    value: f32,
    alpha: f32,
}

impl Smooth {
    fn new(sample_rate: f32, tau_s: f32) -> Self {
        Self {
            value: 0.0,
            alpha: 1.0 - (-1.0 / (sample_rate * tau_s)).exp(),
        }
    }
    fn next(&mut self, target: f32) -> f32 {
        self.value += (target - self.value) * self.alpha;
        self.value
    }
}

/// xorshift32: deterministic, allocation-free noise. Audio is presentation,
/// not simulation, so it never touches the sim's determinism contract — but
/// being seeded keeps the synth's own tests exact.
#[derive(Clone, Copy)]
struct Rng(u32);

impl Rng {
    fn white(&mut self) -> f32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        (x as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

pub struct Synth {
    rate: f32,
    // Engine.
    eng_phase: f32,
    eng_freq: Smooth,
    eng_amp: Smooth,
    growl_phase: f32,
    pwm_phase: f32,
    eng_lp: f32,
    // Wind.
    wind_q: Smooth,
    wind_lp_l: f32,
    wind_lp_r: f32,
    wind_hp_l: f32,
    wind_hp_r: f32,
    rng_l: Rng,
    rng_r: Rng,
    // Hull.
    vacuum: Smooth,
    load: Smooth,
    brown: f32,
    drone_phase: f32,
    drift_phase: f32,
    // Brake.
    brake: Smooth,
    brake_bp: f32,
    brake_hp: f32,
    // RCS thrusters.
    rcs: Smooth,
    rcs_phase: f32,
    rcs_lp: f32,
    // Hull crackle: sparse metallic pops when the airframe is loaded in
    // vacuum. One retriggered damped resonator; its own RNG so event timing
    // cannot disturb the wind noise sequence.
    crackle_rng: Rng,
    crackle_countdown: f32,
    crackle_env: f32,
    crackle_decay: f32,
    crackle_freq: f32,
    crackle_phase: f32,
    // Master.
    master: Smooth,
    dc_x: (f32, f32),
    dc_y: (f32, f32),
}

impl Synth {
    pub fn new(sample_rate: f32, seed: u32) -> Self {
        Self {
            rate: sample_rate,
            eng_phase: 0.0,
            eng_freq: Smooth::new(sample_rate, 0.012),
            eng_amp: Smooth::new(sample_rate, 0.05),
            growl_phase: 0.0,
            pwm_phase: 0.0,
            eng_lp: 0.0,
            wind_q: Smooth::new(sample_rate, 0.08),
            wind_lp_l: 0.0,
            wind_lp_r: 0.0,
            wind_hp_l: 0.0,
            wind_hp_r: 0.0,
            rng_l: Rng(seed | 1),
            rng_r: Rng(seed.rotate_left(16) | 1),
            vacuum: Smooth::new(sample_rate, 0.25),
            load: Smooth::new(sample_rate, 0.12),
            brown: 0.0,
            drone_phase: 0.0,
            drift_phase: 0.0,
            brake: Smooth::new(sample_rate, 0.04),
            brake_bp: 0.0,
            brake_hp: 0.0,
            rcs: Smooth::new(sample_rate, 0.06),
            rcs_phase: 0.0,
            rcs_lp: 0.0,
            crackle_rng: Rng(seed.wrapping_mul(2654435761) | 1),
            crackle_countdown: 0.4,
            crackle_env: 0.0,
            crackle_decay: 0.999,
            crackle_freq: 150.0,
            crackle_phase: 0.0,
            master: Smooth::new(sample_rate, 0.05),
            dc_x: (0.0, 0.0),
            dc_y: (0.0, 0.0),
        }
    }

    /// Engine pitch for a given effort, quantised to the semitone ladder.
    /// Public so the tests can assert the ladder itself.
    pub fn engine_pitch(effort: f32) -> f32 {
        // 38 Hz idle rising ~2.5 octaves at full burn — guttural end of a SID.
        let f = 38.0 * (1.0 + 4.6 * effort.clamp(0.0, 1.0));
        let steps = (12.0 * (f / 38.0).log2()).round();
        38.0 * (2.0f32).powf(steps / 12.0)
    }

    fn lp_coeff(&self, cutoff_hz: f32) -> f32 {
        1.0 - (-core::f32::consts::TAU * cutoff_hz / self.rate).exp()
    }

    /// Render one stereo frame pair (left, right).
    fn frame(&mut self, levels: &Levels) -> (f32, f32) {
        let tau = core::f32::consts::TAU;

        // Vacuum first: it muffles everything airborne. In space the only
        // path from engine to ear is conduction through the hull, so the burn
        // drops to a low structural mutter rather than a roar.
        let vac = self.vacuum.next(levels.vacuum.clamp(0.0, 1.0));
        let muffle_amp = 1.0 - 0.62 * vac;
        let muffle_cut = 1.0 - 0.55 * vac;

        // ---- engine -----------------------------------------------------
        let effort = levels.effort.clamp(0.0, 1.0);
        let freq = self.eng_freq.next(Self::engine_pitch(effort));
        let amp = self.eng_amp.next(if effort > 0.003 {
            (0.16 + 0.34 * effort.powf(0.8)) * muffle_amp
        } else {
            0.0
        });
        self.eng_phase = (self.eng_phase + freq / self.rate).fract();
        self.pwm_phase = (self.pwm_phase + 0.37 / self.rate).fract();
        self.growl_phase = (self.growl_phase + (16.0 + 14.0 * effort) / self.rate).fract();

        // Saw + pulse with slow PWM: the classic pairing.
        let saw = 2.0 * self.eng_phase - 1.0;
        let width = 0.5 + 0.32 * (tau * self.pwm_phase).sin();
        let pulse = if self.eng_phase < width { 1.0 } else { -1.0 };
        let growl = 1.0 - 0.38 * (0.5 + 0.5 * (tau * self.growl_phase).sin());
        let drive = 1.0 + 2.2 * effort;
        let mut engine = (0.6 * saw + 0.4 * pulse) * growl * amp;
        engine = (engine * drive).tanh();
        // Keep it in the chest: low-pass hard, harder still in vacuum.
        let k = self.lp_coeff((95.0 + 240.0 * effort) * muffle_cut);
        self.eng_lp += (engine - self.eng_lp) * k;
        let engine = self.eng_lp;

        // ---- wind -------------------------------------------------------
        let q = self.wind_q.next(levels.wind_q.clamp(0.0, 1.0));
        let cutoff = self.lp_coeff(140.0 + 1500.0 * q);
        let wind_amp = q.sqrt() * 0.5;
        let mut wind = (0.0, 0.0);
        if wind_amp > 1e-4 {
            let n_l = self.rng_l.white();
            let n_r = self.rng_r.white();
            self.wind_lp_l += (n_l - self.wind_lp_l) * cutoff;
            self.wind_lp_r += (n_r - self.wind_lp_r) * cutoff;
            // One-pole high-pass at ~30 Hz keeps wind from stealing the
            // engine's sub band.
            let hp = self.lp_coeff(30.0);
            self.wind_hp_l += (self.wind_lp_l - self.wind_hp_l) * hp;
            self.wind_hp_r += (self.wind_lp_r - self.wind_hp_r) * hp;
            wind = (
                (self.wind_lp_l - self.wind_hp_l) * wind_amp,
                (self.wind_lp_r - self.wind_hp_r) * wind_amp,
            );
        }

        // ---- hull, in vacuum -------------------------------------------
        let load = self.load.next(levels.load_g.clamp(0.0, 4.0));
        // Leaky integrator of white noise = brown rumble; the leak keeps it
        // bounded without a hard clamp.
        self.brown = self.brown * 0.985 + self.rng_l.white() * 0.02;
        self.drone_phase = (self.drone_phase + 31.0 / self.rate).fract();
        self.drift_phase = (self.drift_phase + 0.11 / self.rate).fract();
        let drift = 0.6 + 0.4 * (tau * self.drift_phase).sin();
        // "Almost muted": the resting rumble sits just above perception, so
        // the crackle and the muffled burn read against near-silence.
        let hull_amp = vac * (0.016 + 0.045 * load) * drift;
        let hull = (self.brown * 2.2 + 0.5 * (tau * self.drone_phase).sin()) * hull_amp;

        // ---- hull crackle ----------------------------------------------
        // Sparse Poisson-ish pops: metal relieving stress. Rate rises with
        // load, exists only in vacuum — in air the wind owns this band and
        // real airframes groan under aero load anyway, which drag supplies.
        let crackle_rate = vac * (0.25 + 2.8 * load);
        let mut crackle = 0.0;
        if crackle_rate > 0.01 {
            self.crackle_countdown -= 1.0 / self.rate;
            if self.crackle_countdown <= 0.0 {
                let u = self.crackle_rng.white() * 0.5 + 0.5;
                let v = self.crackle_rng.white() * 0.5 + 0.5;
                self.crackle_countdown = (0.15 + 1.7 * u) / crackle_rate;
                // Mostly quick ticks, occasionally a low groan.
                if v > 0.85 {
                    self.crackle_freq = 52.0 + 40.0 * u;
                    self.crackle_decay = (-1.0 / (self.rate * 0.45)).exp();
                    self.crackle_env = 0.22 + 0.18 * v;
                } else {
                    self.crackle_freq = 95.0 + 240.0 * u;
                    self.crackle_decay = (-1.0 / (self.rate * (0.03 + 0.09 * v))).exp();
                    self.crackle_env = 0.12 + 0.22 * v;
                }
            }
        }
        if self.crackle_env > 1e-4 {
            self.crackle_phase = (self.crackle_phase + self.crackle_freq / self.rate).fract();
            crackle = (tau * self.crackle_phase).sin() * self.crackle_env * vac;
            self.crackle_env *= self.crackle_decay;
        }

        // ---- rcs thrusters ---------------------------------------------
        // Attitude jets: a small motor whine plus valve noise, an octave-ish
        // above the main engine and far quieter — and muffled in vacuum the
        // same way, since it reaches the ear through the same hull.
        let rcs = self.rcs.next(levels.rcs.clamp(0.0, 1.0));
        let mut rcs_out = 0.0;
        if rcs > 0.004 {
            self.rcs_phase = (self.rcs_phase + 142.0 / self.rate).fract();
            let tri = 2.0 * (2.0 * (self.rcs_phase - 0.5)).abs() - 1.0;
            let n = self.rng_r.white();
            let lp = self.lp_coeff(420.0 * muffle_cut);
            self.rcs_lp += (n - self.rcs_lp) * lp;
            rcs_out = (tri * 0.45 + self.rcs_lp * 0.8) * rcs * 0.11 * muffle_amp;
        }

        // ---- brake ------------------------------------------------------
        let brake = self.brake.next(levels.brake.clamp(0.0, 1.0));
        let mut hiss = 0.0;
        if brake > 1e-3 {
            let n = self.rng_r.white();
            let lp = self.lp_coeff(2400.0);
            self.brake_bp += (n - self.brake_bp) * lp;
            let hp = self.lp_coeff(350.0);
            self.brake_hp += (self.brake_bp - self.brake_hp) * hp;
            hiss = (self.brake_bp - self.brake_hp) * 0.30 * brake;
        }

        // ---- mix --------------------------------------------------------
        let master = self.master.next(levels.master.clamp(0.0, 1.0));
        let mono = engine + hull + hiss + crackle + rcs_out;
        let l = ((mono + wind.0) * master).tanh();
        let r = ((mono + wind.1) * master).tanh();

        // DC block: the brown rumble and asymmetric pulse both bias the mean,
        // and a DC offset is inaudible right up until it thumps on stop.
        let out_l = l - self.dc_x.0 + 0.995 * self.dc_y.0;
        self.dc_x.0 = l;
        self.dc_y.0 = out_l;
        let out_r = r - self.dc_x.1 + 0.995 * self.dc_y.1;
        self.dc_x.1 = r;
        self.dc_y.1 = out_r;

        (out_l, out_r)
    }

    /// Fill an interleaved stereo buffer.
    pub fn render(&mut self, levels: &Levels, out: &mut [f32]) {
        for pair in out.chunks_exact_mut(2) {
            let (l, r) = self.frame(levels);
            pair[0] = l;
            pair[1] = r;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rms(buf: &[f32]) -> f32 {
        (buf.iter().map(|s| s * s).sum::<f32>() / buf.len() as f32).sqrt()
    }

    fn render_secs(levels: Levels, secs: f32) -> Vec<f32> {
        let mut synth = Synth::new(48_000.0, 0xC0FFEE);
        let mut buf = vec![0.0f32; (48_000.0 * secs) as usize * 2];
        // Warm-up so smoothers settle before measurement.
        let mut warm = vec![0.0f32; 9_600];
        synth.render(&levels, &mut warm);
        synth.render(&levels, &mut buf);
        buf
    }

    /// Whatever the inputs — including absurd ones — the output is finite and
    /// inside [-1, 1]. An audio glitch is unpleasant; a NaN reaching the DAC
    /// is a scream.
    #[test]
    fn output_is_always_finite_and_bounded() {
        let cases = [
            Levels::default(),
            Levels {
                effort: 1.0,
                wind_q: 1.0,
                vacuum: 0.0,
                load_g: 4.0,
                brake: 1.0,
                rcs: 1.0,
                master: 1.0,
            },
            Levels {
                effort: 55.0,
                wind_q: -3.0,
                vacuum: 9.0,
                load_g: 1e9,
                brake: 2.0,
                rcs: 44.0,
                master: 5.0,
            },
        ];
        for levels in cases {
            let buf = render_secs(levels, 0.5);
            for s in buf {
                assert!(s.is_finite(), "non-finite sample for {levels:?}");
                assert!(s.abs() <= 1.0, "sample {s} out of range for {levels:?}");
            }
        }
    }

    /// Space is almost muted — but punctuated. A flat RMS cannot tell "quiet
    /// with pops" from "loud", so this measures the shape of the brief
    /// directly: the FLOOR between events (median of short chunks) is a
    /// whisper against an in-air burn, while the loudest chunk stands well
    /// off that floor — the crackle exists, in silence.
    #[test]
    fn vacuum_idle_is_near_silence_with_events() {
        let idle = render_secs(Levels::default(), 2.0);
        let burn = rms(&render_secs(
            Levels {
                effort: 1.0,
                vacuum: 0.0,
                ..Default::default()
            },
            0.5,
        ));
        let chunk = 4800; // 50 ms of stereo
        let mut chunks: Vec<f32> = idle.chunks(chunk).map(rms).collect();
        chunks.sort_by(f32::total_cmp);
        let floor = chunks[chunks.len() / 2];
        let peak = chunks[chunks.len() - 1];
        assert!(floor > 1e-5, "hull presence missing entirely: {floor:e}");
        assert!(
            floor < burn * 0.12,
            "space floor not quiet: {floor:.4} vs burn {burn:.4}"
        );
        assert!(
            peak > floor * 2.0,
            "no events above the floor: peak {peak:.4} floor {floor:.4}"
        );
    }

    /// Wind grows with dynamic pressure — the sim's q drives it directly, so
    /// monotonicity here is what makes descent audibly build.
    #[test]
    fn wind_grows_with_dynamic_pressure() {
        let at = |q: f32| {
            rms(&render_secs(
                Levels {
                    wind_q: q,
                    vacuum: 0.0,
                    ..Default::default()
                },
                0.4,
            ))
        };
        let (w1, w2, w3) = (at(0.05), at(0.35), at(1.0));
        assert!(
            w1 < w2 && w2 < w3,
            "wind not monotone: {w1:.4} {w2:.4} {w3:.4}"
        );
        assert!(w3 > 4.0 * w1, "wind dynamic range too flat");
    }

    /// The engine ladder is quantised: every pitch lands on a semitone of the
    /// 38 Hz root, and a full ramp climbs well over an octave in discrete
    /// steps. This is the SID bite — remove the quantisation and this fails.
    #[test]
    fn engine_pitch_ladder_is_semitone_quantised() {
        let mut last = 0.0;
        let mut distinct = 0;
        for i in 0..=100 {
            let f = Synth::engine_pitch(i as f32 / 100.0);
            let steps = 12.0 * (f / 38.0).log2();
            assert!(
                (steps - steps.round()).abs() < 1e-3,
                "pitch {f} Hz is off the semitone ladder"
            );
            if (f - last).abs() > 0.01 {
                distinct += 1;
                last = f;
            }
        }
        assert!(distinct > 12, "ladder too coarse: {distinct} steps");
        assert!(
            Synth::engine_pitch(1.0) > Synth::engine_pitch(0.0) * 4.0,
            "full burn should sit >2 octaves above idle"
        );
    }

    /// A hard step in effort must not click: smoothing keeps sample-to-sample
    /// jumps small through the transition.
    #[test]
    fn effort_step_does_not_click() {
        let mut synth = Synth::new(48_000.0, 7);
        let mut a = vec![0.0f32; 4800];
        synth.render(&Levels::default(), &mut a);
        let mut b = vec![0.0f32; 9600];
        synth.render(
            &Levels {
                effort: 1.0,
                ..Default::default()
            },
            &mut b,
        );
        let mut max_jump = 0.0f32;
        for w in b.chunks_exact(2).collect::<Vec<_>>().windows(2) {
            max_jump = max_jump.max((w[1][0] - w[0][0]).abs());
        }
        assert!(max_jump < 0.30, "click on effort step: jump {max_jump:.3}");
    }

    /// In vacuum the burn is a structural mutter, not a roar: same effort,
    /// clearly quieter than in air. The engine reaches the ear only through
    /// the hull.
    #[test]
    fn engine_is_muffled_in_vacuum() {
        let in_air = rms(&render_secs(
            Levels {
                effort: 1.0,
                vacuum: 0.0,
                ..Default::default()
            },
            0.5,
        ));
        let in_space = rms(&render_secs(
            Levels {
                effort: 1.0,
                vacuum: 1.0,
                ..Default::default()
            },
            0.5,
        ));
        assert!(
            in_space < in_air * 0.62,
            "vacuum burn not muffled: {in_space:.4} vs {in_air:.4}"
        );
        assert!(in_space > 0.005, "vacuum burn should still mutter");
    }

    /// A loaded hull in vacuum crackles: load makes noise beyond the drone.
    #[test]
    fn hull_crackles_under_load_in_vacuum() {
        let unloaded = rms(&render_secs(
            Levels {
                vacuum: 1.0,
                load_g: 0.0,
                ..Default::default()
            },
            2.0,
        ));
        let loaded = rms(&render_secs(
            Levels {
                vacuum: 1.0,
                load_g: 3.0,
                ..Default::default()
            },
            2.0,
        ));
        assert!(
            loaded > unloaded * 1.25,
            "no crackle under load: {loaded:.4} vs {unloaded:.4}"
        );
    }

    /// Rolling is flying: torque alone must make sound.
    #[test]
    fn rcs_torque_is_audible() {
        let quiet = rms(&render_secs(
            Levels {
                vacuum: 0.0,
                ..Default::default()
            },
            0.4,
        ));
        let rolling = rms(&render_secs(
            Levels {
                vacuum: 0.0,
                rcs: 1.0,
                ..Default::default()
            },
            0.4,
        ));
        assert!(
            rolling > quiet * 1.5,
            "rcs inaudible: {rolling:.5} vs {quiet:.5}"
        );
    }

    /// Same seed, same inputs, same samples — the synth's own determinism,
    /// so a future golden-audio test is possible at all.
    #[test]
    fn rendering_is_deterministic() {
        let levels = Levels {
            effort: 0.6,
            wind_q: 0.3,
            vacuum: 0.2,
            load_g: 1.0,
            brake: 0.5,
            rcs: 0.4,
            master: 0.9,
        };
        let a = render_secs(levels, 0.25);
        let b = render_secs(levels, 0.25);
        assert_eq!(a, b);
    }

    /// Sustained output must not drift off centre: the DC blocker is load-
    /// bearing because the brown rumble and asymmetric pulse both bias the
    /// mean, and DC thumps when the stream stops.
    #[test]
    fn output_has_no_dc_offset() {
        let buf = render_secs(
            Levels {
                effort: 0.8,
                vacuum: 0.5,
                load_g: 2.0,
                ..Default::default()
            },
            1.0,
        );
        let mean = buf.iter().sum::<f32>() / buf.len() as f32;
        assert!(mean.abs() < 0.01, "DC offset {mean:.4}");
    }
}
