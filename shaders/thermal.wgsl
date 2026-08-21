// thermal.wgsl — hull heating from atmospheric entry (pass: thermal)
//
// Lane: A (vertex+fragment only). Cost class: negligible — a 64x64 target,
// once per frame.
//
// The thermal state of the ship lives on the GPU and nowhere else. A small
// octahedral texture covers every direction on the hull (see oct_encode in
// the prelude); each texel is one patch of skin, and each frame this pass
// reads last frame's field and writes the next one — a ping-pong, the whole
// simulation in one fragment shader. The CPU contributes only raw physics it
// already has: the ship's velocity in its own frame and the air density at
// the hull. It never sees a temperature.
//
// Two fields, because entry has two timescales:
//
//   g — the GAS CAP. The shocked air in front of the hull. Its temperature is
//       the stagnation temperature, ∝ v², and it exists only while there is
//       air to shock: it arrives in a fraction of a second and vanishes the
//       moment the ship is back in vacuum or slows down.
//   r — the HULL. Thermal mass. Heated by the gas at the Sutton-Graves rate
//       q ∝ sqrt(ρ) · v³, cooled by radiation (T⁴, which is why a glowing
//       hull fades fast from white but lingers long at dull red), by the
//       airstream once the ship is slow and deep, and by conduction to the
//       neighbouring patches. It keeps glowing after the plasma is gone.
//
// Both are in kilokelvin above ambient. Nothing here is an "event": the glow
// is a continuous function of the trajectory, so a shallow skip, a steep
// plunge and a slow sink all look like what they are.

struct Thermal {
    // xyz: velocity in ship space (right, up, forward), m/s. w: speed.
    vel: vec4<f32>,
    // x: air density at the hull, kg/m³. y: sea-level density.
    // z: frame dt, s. w: 1 to reset the field (first frame / respawn).
    air: vec4<f32>,
}

@group(0) @binding(0) var<uniform> th: Thermal;
@group(0) @binding(1) var prev_tex: texture_2d<f32>;
@group(0) @binding(2) var prev_samp: sampler;

// Speed at which full-density air delivers unit heat flux. Just under this
// planet's orbital speed (~790 m/s), so a de-orbit is a real entry and a
// mach-1 pass at sea level is a warm hull, not a fireball.
const V_REF: f32 = 700.0;
// Stagnation temperature at V_REF, kK above ambient. ~2.8 kK is a hot
// yellow-white core — hotter than a real ship's gas cap reads from inside,
// cooler than the 7 kK of a capsule from orbit, picked for readability (P1).
const T_GAS_REF: f32 = 2.8;
// Hull heating gain (kK/s at unit flux) and radiative loss, kK^-3 s^-1.
// Their ratio sets the steady-state glow: (GAIN/RAD)^(1/4) ≈ 2.5 kK at q = 1.
const HEAT_GAIN: f32 = 1.5;
const RAD_LOSS: f32 = 0.038;
// Convective loss ∝ sqrt(ρ) v: the same airstream that heats a fast hull
// cools a slow one.
const CONV_LOSS: f32 = 0.35;
// Lateral conduction between neighbouring patches, per second.
const CONDUCTION: f32 = 1.2;
// Gas cap response time, s.
const GAS_TAU: f32 = 0.18;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x, -xy.y) * 0.5 + 0.5;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (th.air.w > 0.5) {
        return vec4<f32>(0.0);
    }
    // Explicit integration: clamp the step so a hitch cannot overshoot, and
    // treat every loss term implicitly so nothing can go negative.
    let dt = clamp(th.air.z, 0.0, 0.05);
    let texel = 1.0 / vec2<f32>(textureDimensions(prev_tex));
    let prev = textureSampleLevel(prev_tex, prev_samp, in.uv, 0.0);

    let n = oct_decode(in.uv * 2.0 - 1.0);
    let speed = max(th.vel.w, 1e-3);
    // The wind comes from where the ship is going: the patch facing the
    // velocity is the stagnation point.
    let flow = th.vel.xyz / speed;
    // Incidence: 1 at the stagnation point, 0 at the shoulder, 0 behind.
    let cosi = max(dot(n, flow), 0.0);
    let rho_ratio = max(th.air.x / max(th.air.y, 1e-9), 0.0);
    let v = speed / V_REF;

    // ---- gas cap ---------------------------------------------------------
    // Stagnation temperature ∝ v², present only where there is air to shock.
    // The air onset is wide and early (sqrt ρ), so the cap kindles high up,
    // the way the entry crackle does in the audio. The shoulders see a
    // cooler, thinner sheath; the wake behind still carries a little.
    let air_on = clamp(sqrt(rho_ratio) * 6.0, 0.0, 1.0);
    let shape = mix(0.22, 1.0, pow(cosi, 1.5));
    let gas_target = air_on * v * v * T_GAS_REF * shape;
    let gas = mix(prev.g, gas_target, 1.0 - exp(-dt / GAS_TAU));

    // ---- hull ------------------------------------------------------------
    // Sutton-Graves heat flux, spread over the nose as cos^1.5.
    let q = sqrt(rho_ratio) * v * v * v * pow(cosi, 1.5);
    // Conduction: a four-tap Laplacian over the neighbouring patches.
    let lap = textureSampleLevel(prev_tex, prev_samp, in.uv + vec2<f32>(texel.x, 0.0), 0.0).r
        + textureSampleLevel(prev_tex, prev_samp, in.uv - vec2<f32>(texel.x, 0.0), 0.0).r
        + textureSampleLevel(prev_tex, prev_samp, in.uv + vec2<f32>(0.0, texel.y), 0.0).r
        + textureSampleLevel(prev_tex, prev_samp, in.uv - vec2<f32>(0.0, texel.y), 0.0).r
        - 4.0 * prev.r;
    var hull = prev.r + dt * (HEAT_GAIN * q + CONDUCTION * lap);
    hull = max(hull, 0.0);
    let loss = RAD_LOSS * hull * hull * hull + CONV_LOSS * sqrt(rho_ratio) * v;
    hull = hull / (1.0 + dt * loss);

    return vec4<f32>(hull, gas, 0.0, 1.0);
}
