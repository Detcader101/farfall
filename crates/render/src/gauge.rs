//! Holographic velocity gauge (SPEC §6.5): the first cockpit instrument.
//!
//! Additively blended over the scene, SDF-drawn in the shader; this module
//! owns the pipeline and the *relevance fade* — the logic that decides how
//! present the hologram is. That logic is pure and tested here: instruments
//! that appear when they matter are what make a holographic cockpit feel
//! seamless, and "when they matter" is a behaviour worth pinning.

/// Relevance fade: the gauge surfaces on acceleration and at high speed,
/// and melts away in slow, settled flight. Framerate-independent.
#[derive(Debug, Clone, Copy)]
pub struct GaugeFade {
    level: f32,
    prev_speed: f32,
    primed: bool,
}

impl Default for GaugeFade {
    fn default() -> Self {
        Self::new()
    }
}

impl GaugeFade {
    pub fn new() -> Self {
        Self {
            level: 0.0,
            prev_speed: 0.0,
            primed: false,
        }
    }

    /// Advance by `dt` seconds with the current speed (m/s). Returns the
    /// visibility level 0..1.
    pub fn update(&mut self, dt: f32, speed: f32) -> f32 {
        if !self.primed {
            self.primed = true;
            self.prev_speed = speed;
        }
        let dt = dt.clamp(1e-4, 0.25);
        let accel = ((speed - self.prev_speed) / dt).abs();
        self.prev_speed = speed;

        // Two reasons to exist: things are changing, or things are fast.
        let from_accel = ((accel - 2.0) / 10.0).clamp(0.0, 1.0);
        let from_speed = ((speed - 160.0) / 240.0).clamp(0.0, 1.0);
        let target = from_accel.max(from_speed);

        // Quick to appear (an instrument that lags its moment is useless),
        // slow to leave (it should linger long enough to be read).
        let tau = if target > self.level { 0.20 } else { 1.4 };
        let alpha = 1.0 - (-dt / tau).exp();
        self.level += (target - self.level) * alpha;
        self.level.clamp(0.0, 1.0)
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}

/// Relevance fade for the G meter: load matters when there is some. A
/// quarter g of anything shows it; it lingers after, so a pulled turn
/// leaves its number on the glass for a beat.
#[derive(Debug, Clone, Copy, Default)]
pub struct GForceFade {
    level: f32,
}

impl GForceFade {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, dt: f32, g: f32) -> f32 {
        let dt = dt.clamp(1e-4, 0.25);
        let target = ((g - 0.25) / 0.5).clamp(0.0, 1.0);
        let tau = if target > self.level { 0.15 } else { 1.6 };
        let alpha = 1.0 - (-dt / tau).exp();
        self.level += (target - self.level) * alpha;
        self.level.clamp(0.0, 1.0)
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}

/// Relevance fade for the altimeter: altitude matters when the ground is
/// coming up — low, or approached fast. High settled cruise hides it.
#[derive(Debug, Clone, Copy, Default)]
pub struct AltitudeFade {
    level: f32,
}

impl AltitudeFade {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance by `dt` seconds. `vspeed` is radial velocity, m/s, positive up.
    pub fn update(&mut self, dt: f32, altitude_m: f32, vspeed_mps: f32) -> f32 {
        let dt = dt.clamp(1e-4, 0.25);
        // Two reasons to exist: the ground is close, or closing fast.
        let from_low = ((4_000.0 - altitude_m) / 3_000.0).clamp(0.0, 1.0);
        let from_sink = ((-vspeed_mps - 15.0) / 60.0).clamp(0.0, 1.0);
        let target = from_low.max(from_sink);
        let tau = if target > self.level { 0.20 } else { 1.4 };
        let alpha = 1.0 - (-dt / tau).exp();
        self.level += (target - self.level) * alpha;
        self.level.clamp(0.0, 1.0)
    }

    pub fn level(&self) -> f32 {
        self.level
    }
}

/// Hologram inertia: the instruments float in the cockpit's air, so when the
/// ship rotates they lag a beat before the projector "catches up" — and the
/// shader parallaxes each depth layer by this vector, needles drifting more
/// than dial faces. That layered disagreement is what makes flat SDFs read
/// as things with SHAPE. Output is a small screen-space offset in canopy
/// units; input is body pitch/yaw rate. Pure and framerate-independent.
#[derive(Debug, Clone, Copy, Default)]
pub struct HoloSway {
    x: f32,
    y: f32,
}

impl HoloSway {
    /// Full deflection, canopy units. Small on purpose: parallax is felt,
    /// not watched.
    const MAX: f32 = 0.030;

    pub fn new() -> Self {
        Self::default()
    }

    /// `pitch_rate` about body +X (positive = nose up), `yaw_rate` about
    /// body +Y (positive = nose left), rad/s.
    pub fn update(&mut self, dt: f32, pitch_rate: f32, yaw_rate: f32) -> [f32; 2] {
        let dt = dt.clamp(1e-4, 0.25);
        // Nose left → world slides right → the floating holo lags right.
        // Nose up → holo lags down.
        let tx = (yaw_rate * 0.022).clamp(-Self::MAX, Self::MAX);
        let ty = (-pitch_rate * 0.022).clamp(-Self::MAX, Self::MAX);
        let alpha = 1.0 - (-dt / 0.15).exp();
        self.x += (tx - self.x) * alpha;
        self.y += (ty - self.y) * alpha;
        [self.x, self.y]
    }

    pub fn sway(&self) -> [f32; 2] {
        [self.x, self.y]
    }
}

/// The sound-barrier flash: fires on the rising edge of "supersonic in
/// atmosphere" and decays. The CALLER derives that flag from the same
/// expression that drives the audio's boom edge, so the flash and the
/// thunder land on the same frame by construction.
#[derive(Debug, Clone, Copy, Default)]
pub struct MachAlert {
    env: f32,
    was: bool,
    primed: bool,
}

impl MachAlert {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn update(&mut self, dt: f32, supersonic: bool) -> f32 {
        if !self.primed {
            // A ship that wakes supersonic did not just break the barrier.
            self.primed = true;
            self.was = supersonic;
        }
        if supersonic && !self.was {
            self.env = 1.0;
        }
        self.was = supersonic;
        let dt = dt.clamp(1e-4, 0.25);
        self.env *= (-dt / 1.1).exp();
        self.env
    }

    pub fn level(&self) -> f32 {
        self.env
    }
}

/// Auto-ranging altitude readout: three significant digits of kilometres with
/// a floating decimal dot, so "0.05", "3.52", "12.4" and "127" are all the
/// same three-digit instrument. Returns (digits 0..999, dot position — the
/// dot sits after digit 1 or 2; 0 means none).
/// One lap of the speed arc, m/s: two machs.
pub const SPEED_LAP_MPS: f32 = 680.0;

/// Speed readout: metres per second to 999, then kilometres per second with
/// a decimal dot ("1.36"), so the three digits never lie by clamping.
pub fn speed_readout(speed_mps: f32) -> (u32, u32) {
    let v = speed_mps.max(0.0);
    if v < 999.5 {
        (v.round() as u32, 0)
    } else {
        km_readout(v)
    }
}

pub fn km_readout(altitude_m: f32) -> (u32, u32) {
    let km = (altitude_m / 1_000.0).max(0.0);
    if km < 9.995 {
        (((km * 100.0).round() as u32).min(999), 1)
    } else if km < 99.95 {
        (((km * 10.0).round() as u32).min(999), 2)
    } else {
        ((km.round() as u32).min(999), 0)
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GaugeUniforms {
    /// x: arc value, y: visibility, z: time s, w: aspect
    a: [f32; 4],
    /// x: arc full scale, y: target height px, zw: canopy anchor NDC
    b: [f32; 4],
    /// x: readout digits, y: decimal dot slot, z: warning sense (0 high/1 low),
    /// w: 1 if the arc wraps (the needle laps the dial and a multiplier shows)
    c: [f32; 4],
    /// xy: hologram sway (canopy units), z: mach-alert flash 0..1,
    /// w: mach number (negative: this instrument has no mach readout)
    d: [f32; 4],
}

impl GaugeUniforms {
    /// The velocity gauge. `anchor_ndc`: where on the canopy this instrument
    /// sits — the cluster grows by adding gauges at new anchors: same glass,
    /// same warp, different numbers.
    /// `mach`: speed over the local speed of sound (or a negative number
    /// outside the atmosphere, which hides the mach readout entirely — a
    /// mach number in vacuum is a meaningless quantity and the instrument
    /// should say nothing rather than something false). `alert`: the
    /// sound-barrier flash envelope. `sway`: hologram inertia offset.
    #[allow(clippy::too_many_arguments)]
    pub fn speed(
        speed_mps: f32,
        visibility: f32,
        time_s: f32,
        aspect: f32,
        height_px: f32,
        anchor_ndc: [f32; 2],
        sway: [f32; 2],
        mach: f32,
        alert: f32,
    ) -> Self {
        let (digits, dot) = speed_readout(speed_mps);
        Self {
            a: [speed_mps, visibility, time_s, aspect],
            // One lap of the arc is mach 2 (680 m/s at this planet's 340):
            // the amber bars sit at mach 1 and mach 2, and past the end the
            // needle laps the dial with a ×N multiplier beside it — orbital
            // speed is lap 2, halfway round. 680 here must match the
            // MACH1_MPS the app owns.
            b: [SPEED_LAP_MPS, height_px, anchor_ndc[0], anchor_ndc[1]],
            c: [digits as f32, dot as f32, 0.0, 1.0],
            d: [sway[0], sway[1], alert.clamp(0.0, 1.0), mach],
        }
    }

    /// The G meter: felt acceleration in g, 0..10 on the arc, two decimals
    /// on the readout, amber at the top — the hull and the pilot both have
    /// a limit up there.
    pub fn g_force(
        g: f32,
        visibility: f32,
        time_s: f32,
        aspect: f32,
        height_px: f32,
        anchor_ndc: [f32; 2],
        sway: [f32; 2],
    ) -> Self {
        let g = g.max(0.0);
        let digits = ((g * 100.0).round() as u32).min(999);
        Self {
            a: [g, visibility, time_s, aspect],
            b: [10.0, height_px, anchor_ndc[0], anchor_ndc[1]],
            c: [digits as f32, 1.0, 0.0, 0.0],
            d: [sway[0], sway[1], 0.0, -1.0],
        }
    }

    /// The altimeter: same instrument, different numbers. The arc spans the
    /// atmosphere-relevant band (0..15 km); the readout auto-ranges in km,
    /// and the warning amber sits at the BOTTOM of the arc — low is what an
    /// altimeter warns about.
    pub fn altitude(
        altitude_m: f32,
        visibility: f32,
        time_s: f32,
        aspect: f32,
        height_px: f32,
        anchor_ndc: [f32; 2],
        sway: [f32; 2],
    ) -> Self {
        let (digits, dot) = km_readout(altitude_m);
        Self {
            a: [altitude_m.max(0.0), visibility, time_s, aspect],
            b: [15_000.0, height_px, anchor_ndc[0], anchor_ndc[1]],
            c: [digits as f32, dot as f32, 1.0, 0.0],
            d: [sway[0], sway[1], 0.0, -1.0],
        }
    }
}

/// The speedo and the altimeter are [`InstrumentPass`]es running
/// `gauge.wgsl`; see [`crate::instrument`].
pub type GaugePass = crate::instrument::InstrumentPass;

/// Build a gauge instrument.
pub fn gauge_pass(
    device: &wgpu::Device,
    target_format: wgpu::TextureFormat,
    sample_count: u32,
) -> GaugePass {
    GaugePass::new(
        device,
        target_format,
        sample_count,
        "gauge",
        crate::shaders::GAUGE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settle(fade: &mut GaugeFade, secs: f32, speed: f32) -> f32 {
        let mut level = 0.0;
        for _ in 0..(secs * 120.0) as u32 {
            level = fade.update(1.0 / 120.0, speed);
        }
        level
    }

    /// Hard acceleration surfaces the gauge quickly.
    #[test]
    fn appears_under_acceleration() {
        let mut fade = GaugeFade::new();
        let mut speed = 50.0;
        let mut level = 0.0;
        for _ in 0..60 {
            speed += 40.0 / 120.0; // 40 m/s^2 burn
            level = fade.update(1.0 / 120.0, speed);
        }
        assert!(level > 0.5, "gauge missed a hard burn: {level:.2}");
    }

    /// High cruise keeps it visible even with zero acceleration.
    #[test]
    fn stays_visible_at_high_speed() {
        let mut fade = GaugeFade::new();
        let level = settle(&mut fade, 5.0, 700.0);
        assert!(level > 0.9, "gauge faded at 700 m/s cruise: {level:.2}");
    }

    /// Slow, settled flight melts it away — and slowly enough to be read.
    #[test]
    fn fades_in_settled_slow_flight() {
        let mut fade = GaugeFade::new();
        settle(&mut fade, 3.0, 700.0);
        let after_1s = settle(&mut fade, 1.0, 40.0);
        let after_6s = settle(&mut fade, 5.0, 40.0);
        assert!(
            after_1s > 0.25,
            "gauge vanished too fast to read: {after_1s:.2}"
        );
        assert!(after_6s < 0.1, "gauge never left: {after_6s:.2}");
    }

    /// The altimeter surfaces when descending hard, even from high up.
    #[test]
    fn altimeter_appears_in_a_dive() {
        let mut fade = AltitudeFade::new();
        let mut level = 0.0;
        for _ in 0..90 {
            level = fade.update(1.0 / 120.0, 11_000.0, -180.0);
        }
        assert!(level > 0.5, "altimeter slept through a dive: {level:.2}");
    }

    /// Near the ground it stays up even in level flight — and melts away in
    /// high, settled cruise.
    #[test]
    fn altimeter_watches_the_ground_and_leaves_at_altitude() {
        let mut fade = AltitudeFade::new();
        let mut low = 0.0;
        for _ in 0..600 {
            low = fade.update(1.0 / 120.0, 600.0, 0.0);
        }
        assert!(low > 0.9, "altimeter hid near the ground: {low:.2}");
        let mut high = low;
        for _ in 0..(120 * 8) {
            high = fade.update(1.0 / 120.0, 12_000.0, 0.0);
        }
        assert!(high < 0.1, "altimeter never left at cruise: {high:.2}");
    }

    /// Sway lags rotation, points the right way, clamps, and settles back
    /// to centre when the rates stop: hologram inertia, not hologram drift.
    #[test]
    fn sway_lags_clamps_and_settles() {
        let mut sway = HoloSway::new();
        // Nose-left yaw: the holo drifts right (+x), and stays bounded even
        // at a silly rate.
        let mut v = [0.0, 0.0];
        for _ in 0..120 {
            v = sway.update(1.0 / 120.0, 0.0, 30.0);
        }
        assert!(v[0] > 0.0, "yaw left should sway right: {v:?}");
        assert!(v[0] <= HoloSway::MAX + 1e-6, "sway unclamped: {v:?}");
        // Pitch up: holo lags down.
        let mut sway = HoloSway::new();
        for _ in 0..120 {
            v = sway.update(1.0 / 120.0, 1.5, 0.0);
        }
        assert!(v[1] < 0.0, "pitch up should sway down: {v:?}");
        // Rates stop: it settles back to centre.
        for _ in 0..(120 * 3) {
            v = sway.update(1.0 / 120.0, 0.0, 0.0);
        }
        assert!(v[0].abs() < 1e-3 && v[1].abs() < 1e-3, "sway stuck: {v:?}");
    }

    /// The barrier flash fires on the rising edge only — in-air gating is
    /// the caller's job — decays on its own, and re-arms after dropping
    /// subsonic. Waking already supersonic is not an event.
    #[test]
    fn mach_alert_fires_on_the_edge_only() {
        let dt = 1.0 / 120.0;
        let mut alert = MachAlert::new();
        // Wake already supersonic: nothing.
        assert!(alert.update(dt, true) < 1e-3, "phantom flash on wake");
        // Drop subsonic, cross again: full flash.
        alert.update(dt, false);
        let fired = alert.update(dt, true);
        assert!(fired > 0.9, "no flash at the barrier: {fired:.3}");
        // Holding supersonic decays rather than re-firing.
        let mut level = fired;
        for _ in 0..(120 * 4) {
            level = alert.update(dt, true);
        }
        assert!(level < 0.1, "flash never fades: {level:.3}");
    }

    /// The km readout auto-ranges: three significant digits, floating dot.
    #[test]
    fn speed_readout_goes_to_km_past_three_digits() {
        assert_eq!(speed_readout(0.0), (0, 0));
        assert_eq!(speed_readout(773.4), (773, 0));
        assert_eq!(speed_readout(999.4), (999, 0));
        assert_eq!(speed_readout(1_360.0), (136, 1)); // 1.36 km/s
        assert_eq!(speed_readout(12_400.0), (124, 2)); // 12.4 km/s
    }

    #[test]
    fn g_meter_shows_two_decimals_and_clamps() {
        let u = GaugeUniforms::g_force(3.456, 1.0, 0.0, 1.6, 900.0, [0.0, 0.0], [0.0, 0.0]);
        assert_eq!(u.c[0], 346.0);
        assert_eq!(u.c[1], 1.0);
        assert_eq!(u.b[0], 10.0);
        let wild = GaugeUniforms::g_force(-2.0, 1.0, 0.0, 1.6, 900.0, [0.0, 0.0], [0.0, 0.0]);
        assert_eq!(wild.a[0], 0.0);
        let huge = GaugeUniforms::g_force(40.0, 1.0, 0.0, 1.6, 900.0, [0.0, 0.0], [0.0, 0.0]);
        assert_eq!(huge.c[0], 999.0);
    }

    #[test]
    fn g_fade_shows_under_load_and_lingers() {
        let mut f = GForceFade::new();
        for _ in 0..40 {
            f.update(0.05, 0.0);
        }
        assert!(f.level() < 0.01);
        for _ in 0..40 {
            f.update(0.05, 2.0);
        }
        assert!(f.level() > 0.95);
        f.update(0.1, 0.0);
        assert!(f.level() > 0.8, "dropped too fast: {}", f.level());
    }

    #[test]
    fn km_readout_auto_ranges() {
        assert_eq!(km_readout(50.0), (5, 1)); // 0.05 km
        assert_eq!(km_readout(3_520.0), (352, 1)); // 3.52
        assert_eq!(km_readout(12_400.0), (124, 2)); // 12.4
        assert_eq!(km_readout(127_000.0), (127, 0)); // 127
                                                     // Range edges do not overflow three digits.
        assert_eq!(km_readout(9_996.0), (100, 2)); // 9.996 -> 10.0
        assert_eq!(km_readout(1.0e9), (999, 0));
        assert_eq!(km_readout(-5.0), (0, 1));
    }

    /// The first frame must not read a garbage "acceleration" from the
    /// uninitialised previous speed.
    #[test]
    fn first_frame_is_calm() {
        let mut fade = GaugeFade::new();
        let level = fade.update(1.0 / 120.0, 790.0);
        // High speed legitimately raises it, but only via the speed term —
        // never a spike from a phantom 790 m/s-per-frame acceleration.
        assert!(level < 0.05, "first frame spiked: {level:.3}");
    }
}
