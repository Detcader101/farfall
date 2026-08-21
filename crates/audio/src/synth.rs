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
//! - **entry** — the sound of arriving: a crackle-and-roar build-up that
//!   grows with interface intensity the way the wind grows with q, and one
//!   thunderous boom fired when the ship punches through into dense air.
//!   This voice bypasses the vacuum mute — it is the border's own sound and
//!   is zero in clean space by construction (the entry level needs air).
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
    /// Air-brake engagement 0..1.
    pub brake: f32,
    /// Attitude-thruster demand 0..1 (largest torque axis): the RCS voice.
    /// Rolling is flying too, and a silent manoeuvre reads as a broken game.
    pub rcs: f32,
    /// Atmosphere-interface intensity 0..1: the build-up of arrival. Rises
    /// through the descent as thin air starts to bite, collapses once the
    /// ship is properly inside (the wind takes over), zero in clean vacuum
    /// and in settled flight. Drives the mach crackle-and-roar, and its
    /// collapse into dense air is when the boom fires. This is not sound in
    /// space; it is the sound of arriving — the one voice the vacuum mute
    /// does not touch, because it gates itself.
    pub entry: f32,
    /// 1 while the ship is supersonic INSIDE the atmosphere (dense air and
    /// past mach 1), else 0. The synth booms on the rising edge — breaking
    /// the barrier in level flight thunders just like arriving does — and
    /// the app derives the same edge for the HUD's visual alert, so what
    /// the pilot sees and what they hear cannot drift apart.
    pub supersonic: f32,
    /// How many of the path's hoops have passed the ship, as a count. The
    /// synth plays a soft "womp" on every increment — a cockpit sound, so
    /// it lives outside the vacuum mute: in space the instruments are the
    /// only thing that can make a noise, and this is the one that says
    /// "another kilometre".
    pub hoops: f32,
    /// Master gain 0..1.
    pub master: f32,
}

impl Default for Levels {
    fn default() -> Self {
        Self {
            effort: 0.0,
            wind_q: 0.0,
            vacuum: 1.0,
            brake: 0.0,
            rcs: 0.0,
            entry: 0.0,
            supersonic: 0.0,
            hoops: 0.0,
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

/// The entry boom's state machine: the build-up CHARGES it (entry intensity
/// through the descent), and punching into dense air FIRES it — so the boom
/// lands exactly where the crackle collapses and the wind takes over, the way
/// a shock wave arrives after the buffeting. An aborted entry (drifting back
/// out to vacuum before reaching air) discharges silently. Pure, so the
/// once-per-entry contract is provable in a test instead of hoped for — a
/// boom that machine-guns on a noisy interface would be the worst sound in
/// the game.
#[derive(Clone, Copy)]
pub struct BoomTrigger {
    charged: bool,
}

impl BoomTrigger {
    pub fn new() -> Self {
        Self { charged: false }
    }
    /// `entry` is interface intensity; `vacuum` is how much space surrounds
    /// the hull (1 vacuum, 0 thick air). Returns true the moment the boom
    /// should fire.
    pub fn update(&mut self, entry: f32, vacuum: f32) -> bool {
        if !self.charged {
            if entry > 0.30 {
                self.charged = true;
            }
            return false;
        }
        if vacuum < 0.35 {
            // Punched through: the shock arrives.
            self.charged = false;
            return true;
        }
        if entry < 0.03 && vacuum > 0.75 {
            // Aborted entry: back out to space, no thunder owed.
            self.charged = false;
        }
        false
    }
}

impl Default for BoomTrigger {
    fn default() -> Self {
        Self::new()
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
    // Brake.
    brake: Smooth,
    brake_bp: f32,
    brake_hp: f32,
    // Vacuum mute.
    vacuum: Smooth,
    // Entry drama.
    entry: Smooth,
    boom_trigger: BoomTrigger,
    boom_env: f32,
    boom_phase: f32,
    boom_sub_phase: f32,
    sputter_gate: f32,
    sputter_lp: f32,
    roar_lp: f32,
    was_supersonic: bool,
    /// The hoop womp: a counter latched from Levels, an envelope, a phase.
    last_hoops: u32,
    womp_env: f32,
    womp_t: f32,
    womp_phase: f32,
    // RCS thrusters.
    rcs: Smooth,
    rcs_phase: f32,
    rcs_lp: f32,
    // Master.
    master: Smooth,
    /// First-frame flag: smoothers snap to their initial targets rather than
    /// gliding from zero, so a ship that WAKES in vacuum is silent from the
    /// first sample instead of fading out of air that was never there.
    primed: bool,
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
            brake: Smooth::new(sample_rate, 0.04),
            brake_bp: 0.0,
            brake_hp: 0.0,
            vacuum: Smooth::new(sample_rate, 0.30),
            entry: Smooth::new(sample_rate, 0.10),
            boom_trigger: BoomTrigger::new(),
            boom_env: 0.0,
            boom_phase: 0.0,
            boom_sub_phase: 0.0,
            sputter_gate: 0.0,
            sputter_lp: 0.0,
            roar_lp: 0.0,
            was_supersonic: false,
            last_hoops: 0,
            womp_env: 0.0,
            womp_t: 0.0,
            womp_phase: 0.0,
            rcs: Smooth::new(sample_rate, 0.06),
            rcs_phase: 0.0,
            rcs_lp: 0.0,
            master: Smooth::new(sample_rate, 0.05),
            primed: false,
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
        if !self.primed {
            self.primed = true;
            self.vacuum.value = levels.vacuum.clamp(0.0, 1.0);
            self.eng_freq.value = Self::engine_pitch(levels.effort.clamp(0.0, 1.0));
            // A ship that WAKES supersonic did not just break the barrier.
            self.was_supersonic = levels.supersonic > 0.5;
            // Nor did it just pass a hoop.
            self.last_hoops = levels.hoops.max(0.0) as u32;
        }

        // Vacuum is a MASTER MUTE. Space is silent — all of it, engine
        // included. The smoother keeps the cut from clicking as the ship
        // crosses the atmosphere boundary; past it, nothing sounds.
        let vac = self.vacuum.next(levels.vacuum.clamp(0.0, 1.0));
        let silence = 1.0 - vac;

        // ---- engine -----------------------------------------------------
        let effort = levels.effort.clamp(0.0, 1.0);
        let freq = self.eng_freq.next(Self::engine_pitch(effort));
        let amp = self.eng_amp.next(if effort > 0.003 {
            0.16 + 0.34 * effort.powf(0.8)
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
        // Keep it in the chest: low-pass hard.
        let k = self.lp_coeff(95.0 + 240.0 * effort);
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

        // ---- rcs thrusters ---------------------------------------------
        // Attitude jets: a small motor whine plus valve noise, an octave-ish
        // above the main engine and far quieter.
        let rcs = self.rcs.next(levels.rcs.clamp(0.0, 1.0));
        let mut rcs_out = 0.0;
        if rcs > 0.004 {
            self.rcs_phase = (self.rcs_phase + 142.0 / self.rate).fract();
            let tri = 2.0 * (2.0 * (self.rcs_phase - 0.5)).abs() - 1.0;
            let n = self.rng_r.white();
            let lp = self.lp_coeff(420.0);
            self.rcs_lp += (n - self.rcs_lp) * lp;
            rcs_out = (tri * 0.45 + self.rcs_lp * 0.8) * rcs * 0.11;
        }

        // ---- atmosphere entry ------------------------------------------
        // The mach build-up: sparse plasma crackle that gets denser and
        // hotter as the interface bites (spark rate grows with entry², so it
        // builds the way the wind builds with q — a few pops high up, a
        // rolling buffet by the border), under a swelling low roar. Then the
        // boom: charged by the build-up, fired the instant the ship punches
        // into dense air — a pitch-dropping thunder with a bright crack on
        // its face and a long rumbling tail.
        let entry = self.entry.next(levels.entry.clamp(0.0, 1.0));
        let mut entry_out = 0.0;
        // Two roads to the same thunder: punching into dense air after the
        // build-up, or breaking mach 1 while already inside it. Both edges
        // light the same envelope, so a supersonic entry — where they land
        // together — is one rolling boom, not a double-fire.
        let ss = levels.supersonic > 0.5;
        let broke_barrier = ss && !self.was_supersonic;
        self.was_supersonic = ss;
        if self.boom_trigger.update(entry, vac) || broke_barrier {
            self.boom_env = 1.0;
            self.boom_phase = 0.0;
            self.boom_sub_phase = 0.0;
        }
        if entry > 0.01 {
            // Fire-like gate: random impulses that decay. Density scales with
            // entry² — the build-up is in the RATE as much as the level.
            let spark = self.rng_r.white();
            if spark > 0.9994 - entry * entry * 0.012 {
                self.sputter_gate = 1.0;
            }
            self.sputter_gate *= 0.9990;
            let n = self.rng_l.white();
            let lp = self.lp_coeff(500.0 + 1400.0 * entry);
            self.sputter_lp += (n - self.sputter_lp) * lp;
            entry_out += self.sputter_lp * self.sputter_gate * self.sputter_gate * entry * 0.85;
            // The roar under the crackle: torn-air noise, opening up and
            // swelling with the square of intensity — wind-like, but ahead
            // of the wind, because out here there is barely any q yet.
            let rn = self.rng_r.white();
            let rlp = self.lp_coeff(180.0 + 700.0 * entry);
            self.roar_lp += (rn - self.roar_lp) * rlp;
            entry_out += self.roar_lp * entry * entry * 0.60;
        }
        if self.boom_env > 1e-3 {
            // Pitch drops with the envelope: thunder, not a beep.
            let f = 24.0 + 42.0 * self.boom_env;
            self.boom_phase = (self.boom_phase + f / self.rate).fract();
            self.boom_sub_phase = (self.boom_sub_phase + (f * 0.52) / self.rate).fract();
            let body = (tau * self.boom_phase).sin() + 0.6 * (tau * self.boom_sub_phase).sin();
            let env2 = self.boom_env * self.boom_env;
            let env8 = env2 * env2 * env2 * env2;
            // The face of the boom is the sound of a jet filmed on a phone:
            // the bass driven straight into a brickwall clipper, so the
            // strike is a flat-topped guttural CRUNT rather than a clean
            // thump. The drive rides env², so the strike is square-edged and
            // the tail relaxes back into round rolling thunder.
            let crunch = (body * (1.2 + 7.0 * env2)).clamp(-1.0, 1.0);
            // The crack: a fast-decaying bright transient on the strike
            // (env^8 dies in a third of a second while the rumble rings on).
            let crack = self.rng_l.white() * env8 * 1.2;
            let slam = self.rng_l.white() * env2 * 0.45;
            entry_out += (crunch * 1.25 + slam + crack) * self.boom_env * 1.3;
            self.boom_env *= 1.0 - 1.0 / (self.rate * 1.8);
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

        // ---- the hoop womp ------------------------------------------------
        // A cockpit sound, not a hull sound: calm, low, round. A sine that
        // sinks from 150 Hz to 55 Hz over a third of a second under a soft
        // envelope, with a quiet octave above for shape. It fires on every
        // increment of the hoop count and nowhere else; it is not gated by
        // the vacuum because it is not the air that makes it.
        let hoops = levels.hoops.max(0.0) as u32;
        if hoops != self.last_hoops {
            self.last_hoops = hoops;
            self.womp_env = 1.0;
            self.womp_t = 0.0;
            self.womp_phase = 0.0;
        }
        let mut womp = 0.0;
        if self.womp_env > 1e-3 {
            let dt = 1.0 / self.rate;
            self.womp_t += dt;
            let f = 55.0 + 95.0 * (-self.womp_t * 7.0).exp();
            self.womp_phase = (self.womp_phase + f / self.rate).fract();
            // Attack over 12 ms so it blooms rather than clicks.
            let attack = (self.womp_t / 0.012).min(1.0);
            let body = (tau * self.womp_phase).sin() + 0.25 * (tau * 2.0 * self.womp_phase).sin();
            womp = body * self.womp_env * attack * 0.16;
            self.womp_env *= (-dt / 0.22).exp();
        }

        // ---- mix --------------------------------------------------------
        let master = self.master.next(levels.master.clamp(0.0, 1.0));
        // Silence multiplies everything the SHIP makes: past the atmosphere
        // border there is no engine, no wind, no thrusters — not a quieter
        // version of them. The one voice outside the mute is entry, because
        // it IS the border: it gates itself on interface intensity, which is
        // zero in clean space by construction, and muting it with the vacuum
        // would silence the build-up at exactly the altitudes where it
        // happens (which is why the old mix was barely audible on entry).
        let mono = engine + hiss + rcs_out;
        let l = (((mono + wind.0) * silence + entry_out + womp) * master).tanh();
        let r = (((mono + wind.1) * silence + entry_out + womp) * master).tanh();

        // DC block: the asymmetric pulse and the clipped boom both bias the
        // mean, and a DC offset is inaudible right up until it thumps on
        // stop. The pole sits at ~4 Hz — low enough to pass the 24 Hz boom
        // fundamental and the 38 Hz engine root untouched (an earlier 0.995
        // pole was a 38 Hz cutoff quietly shaving the sub off everything
        // guttural), high enough to still drain real DC within ~0.3 s.
        let out_l = l - self.dc_x.0 + 0.9995 * self.dc_y.0;
        self.dc_x.0 = l;
        self.dc_y.0 = out_l;
        let out_r = r - self.dc_x.1 + 0.9995 * self.dc_y.1;
        self.dc_x.1 = r;
        self.dc_y.1 = out_r;

        // The DC blocker is a filter and can overshoot the tanh's ±1 by a few
        // thousandths — inaudible, but out-of-range samples are the DAC's
        // problem to mangle, and the bounds test rightly refuses them.
        (out_l.clamp(-1.0, 1.0), out_r.clamp(-1.0, 1.0))
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
                brake: 1.0,
                rcs: 1.0,
                entry: 1.0,
                supersonic: 1.0,
                hoops: 0.0,
                master: 1.0,
            },
            Levels {
                effort: 55.0,
                wind_q: -3.0,
                vacuum: 9.0,
                brake: 2.0,
                rcs: 44.0,
                entry: 7.0,
                supersonic: 3.0,
                hoops: 0.0,
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

    /// SPACE IS SILENT. Not quiet — silent: past the atmosphere border the
    /// output is exactly nothing, full burn included. The one deliberate
    /// exception is the smoothing tail while the mute settles at the border.
    #[test]
    fn space_is_totally_silent() {
        let cases = [
            Levels::default(),
            Levels {
                effort: 1.0,
                rcs: 1.0,
                brake: 1.0,
                ..Default::default()
            },
        ];
        for levels in cases {
            let buf = render_secs(levels, 0.5);
            let r = rms(&buf);
            assert!(r < 1e-4, "space made sound ({r:e}) for {levels:?}");
        }
    }

    /// The same inputs INSIDE the atmosphere are loud — the mute is the
    /// border, not a broken synth.
    #[test]
    fn atmosphere_is_loud_where_space_is_silent() {
        let in_air = rms(&render_secs(
            Levels {
                effort: 1.0,
                vacuum: 0.0,
                ..Default::default()
            },
            0.5,
        ));
        assert!(in_air > 0.02, "burn inaudible in air: {in_air:.4}");
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

    /// The boom fires exactly once per entry, at the moment the ship punches
    /// through into dense air — after the build-up, not during it — and a
    /// noisy interface cannot machine-gun it.
    #[test]
    fn boom_fires_once_at_punch_through() {
        let mut t = BoomTrigger::new();
        let mut fires = 0;
        // (entry, vacuum): build-up in near-vacuum, wobble, then air.
        let descent = [
            (0.00, 1.0),
            (0.10, 0.98),
            (0.34, 0.95), // charged here — must NOT fire yet
            (0.55, 0.90),
            (0.48, 0.85),
            (0.70, 0.70),
            (0.40, 0.50),
            (0.15, 0.30), // dense air: fire
            (0.05, 0.20),
            (0.01, 0.05), // wobbling in air must not re-fire
            (0.02, 0.10),
        ];
        for (i, (e, v)) in descent.iter().enumerate() {
            if t.update(*e, *v) {
                fires += 1;
                assert_eq!(i, 7, "boom must land at punch-through, not at {i}");
            }
        }
        assert_eq!(fires, 1, "exactly one boom per entry");
    }

    /// An aborted entry — build-up, then drifting back out to vacuum — owes
    /// no thunder: the charge dissipates, and only a REAL later entry booms.
    #[test]
    fn aborted_entry_discharges_silently() {
        let mut t = BoomTrigger::new();
        let abort = [(0.0, 1.0), (0.5, 0.9), (0.2, 0.9), (0.01, 0.95), (0.0, 1.0)];
        for (e, v) in abort {
            assert!(!t.update(e, v), "aborted entry must not boom");
        }
        // The next real entry still gets its boom.
        let real = [(0.5, 0.9), (0.3, 0.6), (0.1, 0.2)];
        let fires: u32 = real.iter().map(|(e, v)| t.update(*e, *v) as u32).sum();
        assert_eq!(fires, 1, "discharge must not eat the next real entry");
    }

    /// Breaking the sound barrier IN atmosphere booms — once, on the rising
    /// edge — and going supersonic in vacuum (a meaningless mach) does not.
    #[test]
    fn breaking_the_barrier_booms_once_in_air_only() {
        // In air: crossing up must produce a bang where subsonic cruise had
        // none, and holding supersonic must not machine-gun.
        let mut synth = Synth::new(48_000.0, 0xACE);
        let sub_air = Levels {
            vacuum: 0.0,
            supersonic: 0.0,
            ..Default::default()
        };
        let super_air = Levels {
            vacuum: 0.0,
            supersonic: 1.0,
            ..Default::default()
        };
        let mut cruise = vec![0.0f32; 48_000];
        synth.render(&sub_air, &mut cruise);
        let peak = |b: &[f32]| b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(
            peak(&cruise) < 0.05,
            "subsonic cruise should be near-silent"
        );
        // 4 s of held supersonic: the bang lands at the edge, then only
        // decays — consecutive seconds of strictly falling energy prove one
        // boom and no machine-gunning while the flag stays up.
        let mut held = vec![0.0f32; 48_000 * 8];
        synth.render(&super_air, &mut held);
        assert!(peak(&held) > 0.5, "no bang at mach 1: {:.3}", peak(&held));
        let sec: Vec<f32> = (0..4)
            .map(|i| rms(&held[i * 96_000..(i + 1) * 96_000]))
            .collect();
        assert!(
            sec[0] > sec[1] && sec[1] > sec[2] && sec[2] > sec[3],
            "boom re-fired while holding supersonic: {sec:?}"
        );
    }

    /// A ship that wakes up already supersonic did not just break the
    /// barrier: no phantom boom on the first buffer.
    #[test]
    fn waking_supersonic_is_not_an_event() {
        let mut synth = Synth::new(48_000.0, 0xF1);
        let mut buf = vec![0.0f32; 48_000];
        synth.render(
            &Levels {
                vacuum: 0.0,
                supersonic: 1.0,
                ..Default::default()
            },
            &mut buf,
        );
        let peak = buf.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak < 0.05, "phantom boom on wake: peak {peak:.3}");
    }

    /// The strike is CLIPPED, by design: the flat-topped phone-mic crunch.
    /// A brickwalled low square has near-vertical flanks, and a clean
    /// 24–66 Hz thump at 48 kHz moves at most ~0.009 per sample — so the
    /// steepest sample-to-sample jump is the clip detector. Measured past
    /// the noise crack, so only the clipped body can produce it; and the
    /// late tail must relax back toward a rounder, quieter wave.
    #[test]
    fn boom_strike_is_brickwalled_then_relaxes() {
        let mut synth = Synth::new(48_000.0, 0xC11B);
        let mut warm = vec![0.0f32; 9_600];
        synth.render(
            &Levels {
                vacuum: 0.0,
                ..Default::default()
            },
            &mut warm,
        );
        // 3 s stereo, the boom fired on the first frame.
        let mut buf = vec![0.0f32; 48_000 * 6];
        synth.render(
            &Levels {
                vacuum: 0.0,
                supersonic: 1.0,
                ..Default::default()
            },
            &mut buf,
        );
        // Steepest left-channel edge in a window.
        let steepest = |b: &[f32]| {
            let mut m = 0.0f32;
            let mut prev = b[0];
            for pair in b.chunks_exact(2).skip(1) {
                m = m.max((pair[0] - prev).abs());
                prev = pair[0];
            }
            m
        };
        let strike = steepest(&buf[28_800..57_600]); // 0.3–0.6 s
        let tail = steepest(&buf[240_000..288_000]); // 2.5–3.0 s
        assert!(
            strike > 0.5,
            "strike not clipped: steepest edge {strike:.3}"
        );
        assert!(
            tail < strike * 0.7,
            "tail should relax from the clip: {strike:.3} -> {tail:.3}"
        );
    }

    /// The crackle BUILDS: entry intensity maps monotonically to loudness,
    /// and — the audibility fix — it is loud even in near-vacuum, where the
    /// old mix let the master mute eat it. This is the mach-style build-up:
    /// quiet sparse pops early, a rolling crackle-roar by the border.
    #[test]
    fn entry_crackle_builds_like_wind_even_in_near_vacuum() {
        let at = |e: f32| {
            rms(&render_secs(
                Levels {
                    entry: e,
                    vacuum: 0.92,
                    ..Default::default()
                },
                0.8,
            ))
        };
        let (a, b, c) = (at(0.15), at(0.5), at(0.95));
        assert!(
            a < b && b < c,
            "build-up not monotone: {a:.4} {b:.4} {c:.4}"
        );
        assert!(c > 0.02, "full build-up inaudible in near-vacuum: {c:.4}");
        assert!(c > 6.0 * a, "build-up too flat: {a:.4} -> {c:.4}");
    }

    /// The boom itself: after a charged build-up, hitting dense air produces
    /// a genuinely loud transient with a tail that rings on for over a
    /// second — thunder, not a tick.
    #[test]
    fn punch_through_boom_is_loud_and_rings() {
        let mut synth = Synth::new(48_000.0, 0xB00);
        // Build-up phase: charge the trigger in near-vacuum.
        let mut buildup = vec![0.0f32; 48_000];
        synth.render(
            &Levels {
                entry: 0.7,
                vacuum: 0.9,
                ..Default::default()
            },
            &mut buildup,
        );
        // Punch through: dense air, interface collapsing.
        let after = Levels {
            entry: 0.05,
            vacuum: 0.0,
            wind_q: 0.0,
            ..Default::default()
        };
        // 3 s stereo (96 000 samples per second of it).
        let mut boom = vec![0.0f32; 48_000 * 6];
        synth.render(&after, &mut boom);
        // The vacuum smoother takes ~0.3 s to cross the trigger threshold, so
        // the hit lands inside the first half second.
        let head_peak = boom[..48_000].iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(head_peak > 0.5, "boom not thunderous: peak {head_peak:.3}");
        // 1.2 s after the hit, the rumble is still audibly ringing...
        let ring = rms(&boom[144_000..192_000]);
        assert!(ring > 5e-3, "boom tail died too fast: {ring:.5}");
        // ...and it is a decay, not a drone: by 2.5 s the energy has clearly
        // fallen from the strike. (Peak is the wrong meter here — the clip
        // hugs the rails — energy is what decays.)
        let head = rms(&boom[28_800..76_800]);
        let tail = rms(&boom[240_000..]);
        assert!(
            tail < head * 0.5,
            "boom does not decay: {head:.3} -> {tail:.3}"
        );
    }

    /// Entry drama is audible mid-interface (partial air, so the master mute
    /// does not eat it) and silent in settled flight.
    #[test]
    fn entry_interface_is_loud_then_gone() {
        let during = rms(&render_secs(
            Levels {
                entry: 0.9,
                vacuum: 0.5,
                ..Default::default()
            },
            0.6,
        ));
        let settled = rms(&render_secs(
            Levels {
                entry: 0.0,
                vacuum: 0.0,
                ..Default::default()
            },
            0.6,
        ));
        assert!(during > 0.02, "entry inaudible: {during:.4}");
        assert!(
            during > settled * 3.0,
            "entry should dominate settled flight: {during:.4} vs {settled:.4}"
        );
    }

    /// Same seed, same inputs, same samples — the synth's own determinism,
    /// so a future golden-audio test is possible at all.
    /// A hoop passing in clean vacuum makes a sound — the one cockpit voice
    /// the mute does not touch — and a calm one: audible, soft, gone in
    /// well under a second. Waking with a count already high is not a hoop.
    #[test]
    fn hoop_womp_sounds_in_vacuum_and_stays_calm() {
        let mut synth = Synth::new(48_000.0, 0xC0FFEE);
        let quiet = Levels {
            vacuum: 1.0,
            hoops: 3.0,
            ..Default::default()
        };
        let mut buf = vec![0.0f32; 48_000 / 2 * 2];
        synth.render(&quiet, &mut buf);
        assert!(
            rms(&buf) < 1e-4,
            "silent vacuum was not silent: {}",
            rms(&buf)
        );
        let next = Levels {
            hoops: 4.0,
            ..quiet
        };
        synth.render(&next, &mut buf);
        let womp = rms(&buf);
        assert!(womp > 0.01, "hoop made no sound: {womp}");
        assert!(womp < 0.15, "hoop too loud to be calm: {womp}");
        // Gone within a second; the count itself stays quiet.
        synth.render(&next, &mut buf);
        synth.render(&next, &mut buf);
        assert!(rms(&buf) < 1e-3, "womp rang on: {}", rms(&buf));
    }

    #[test]
    fn rendering_is_deterministic() {
        let levels = Levels {
            effort: 0.6,
            wind_q: 0.3,
            vacuum: 0.2,
            brake: 0.5,
            rcs: 0.4,
            entry: 0.3,
            supersonic: 0.0,
            hoops: 0.0,
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
                ..Default::default()
            },
            1.0,
        );
        let mean = buf.iter().sum::<f32>() / buf.len() as f32;
        assert!(mean.abs() < 0.01, "DC offset {mean:.4}");
    }
}
