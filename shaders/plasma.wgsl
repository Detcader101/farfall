// plasma.wgsl — the entry sheath, seen from inside it (pass: plasma)
//
// Lane: A (vertex+fragment only). Cost class: cheap — one texture fetch and
// a three-octave noise per pixel, and most frames discard at the first test.
//
// Reads the hull thermal field (thermal.wgsl) along each view ray and draws
// what the pilot sees: shocked air glowing at its stagnation temperature,
// streaming back from the nose, and the hull's own incandescence bleeding in
// at the rim of the glass once it has soaked up enough heat. Additive over
// the world, because that is what looking through a luminous gas is.
//
// Colour comes from the temperature, not from a palette: blackbody() in the
// prelude. Above ~4 kK the gas stops being a hot body and starts being a
// plasma, and nitrogen and oxygen lines push it toward violet-white — the
// mix at the end. Everything is in ship space: the camera is bolted to the
// hull, so the view ray in camera space IS the hull direction.

struct Plasma {
    // x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: velocity in ship space (right, up, forward), m/s. w: speed.
    vel: vec4<f32>,
}

@group(0) @binding(0) var<uniform> pl: Plasma;
@group(0) @binding(1) var heat_tex: texture_2d<f32>;
@group(0) @binding(2) var heat_samp: sampler;
// Seamless fbm tile (bake.wgsl fs_noise): the streak texture, two fetches
// where a live fbm3 cost twenty-four hashes per pixel.
@group(0) @binding(3) var noise_tex: texture_2d<f32>;
@group(0) @binding(4) var noise_samp: sampler;

const TAU: f32 = 6.28318531;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

// Radiance of a patch at `kk` kilokelvin: T⁴ under a threshold so that warm
// is invisible and only incandescent reads, normalised so 2.5 kK is unity —
// then saturating. Real radiance keeps climbing as the fourth power, but
// the pilot's eye and the tonemap do not: past ~4 kK the sheath gets whiter
// and more violet (the ionisation mix), not brighter, or an orbital entry
// in thick air erases the instruments (P1).
fn glow(kk: f32) -> f32 {
    let t = kk / 2.5;
    let t4 = t * t * t * t;
    return smoothstep(0.34, 0.52, t) * t4 / (1.0 + t4 / 2.5);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tan_half = pl.params.x;
    let aspect = pl.params.y;
    let time = pl.params.z;
    let exposure = pl.params.w;

    // The view ray in ship space — no world basis needed.
    let ray = normalize(vec3<f32>(in.ndc.x * tan_half * aspect, in.ndc.y * tan_half, 1.0));
    let heat = textureSample(heat_tex, heat_samp, oct_encode(ray) * 0.5 + 0.5);
    let gas_kk = heat.g;
    let hull_kk = heat.r;

    let g_gas = glow(gas_kk);
    let g_hull = glow(hull_kk);
    if (g_gas + g_hull < 1e-4) {
        discard;
    }

    // ---- the sheath: plasma streaming back from the stagnation point ----
    let speed = max(pl.vel.w, 1e-3);
    let flow = pl.vel.xyz / speed;
    let cosi = clamp(dot(ray, flow), -1.0, 1.0);
    // Polar coordinates about the stagnation point: streaks run along theta,
    // away from the nose, and are banded in phi around it.
    let theta = acos(cosi);
    var tangent = cross(flow, vec3<f32>(0.0, 1.0, 0.0));
    if (dot(tangent, tangent) < 1e-4) {
        tangent = cross(flow, vec3<f32>(1.0, 0.0, 0.0));
    }
    tangent = normalize(tangent);
    let bitangent = cross(flow, tangent);
    let phi = atan2(dot(ray, bitangent), dot(ray, tangent));
    // Faster air, faster streaks; the sheath flickers the way a flame does.
    let run = time * (2.5 + 6.0 * clamp(speed / 700.0, 0.0, 2.0));
    // Two octaves of the tile, scrolled back along theta and drifting slowly
    // in phi so the pattern never repeats exactly. Explicit level: the
    // coordinates wrap, and derivative-based mips would see the wrap as a
    // seam.
    // Phi wraps at ±π: a whole number of tiles per revolution, or the
    // pattern fails to meet itself and draws a seam down one azimuth.
    let turn = phi / TAU;
    let uv_a = vec2<f32>(turn * 2.0 + time * 0.02, theta * 0.8 - run * 0.16);
    let uv_b = vec2<f32>(turn * 4.0 - time * 0.03, theta * 1.9 - run * 0.31);
    let streaks = 0.62 * textureSampleLevel(noise_tex, noise_samp, uv_a, 0.0).r
        + 0.38 * textureSampleLevel(noise_tex, noise_samp, uv_b, 1.0).r;
    // Polar coordinates pinch at the pole: blend to a steady core there.
    let flicker = mix(0.75, 0.30 + 1.40 * streaks * streaks, smoothstep(0.0, 0.22, theta));

    // How much luminous gas the sightline crosses: the sheath is a bright
    // cap over the nose with a thin veil streaming back over the shoulders
    // and a faint wake behind — so the view ahead is a core of fire and the
    // rest of the canopy stays readable (P1).
    let cap = pow(max(cosi, 0.0), 6.0);
    let veil = 0.10 * smoothstep(-0.3, 0.6, cosi) + 0.03;
    let thickness = cap + veil;

    var gas_rgb = blackbody(gas_kk);
    // Ionisation: past ~4 kK the air is a plasma, and the line emission of
    // N2 and O pulls the colour toward violet-white.
    let ion = smoothstep(4.5, 8.0, gas_kk);
    gas_rgb = mix(gas_rgb, vec3<f32>(1.0, 0.78, 0.92), ion * 0.45);
    var rgb = gas_rgb * g_gas * flicker * thickness * 0.14;

    // ---- the hull: the glass itself, incandescent at the rim -----------
    // Heat soaks into the frame from the outside in, so the rim of the
    // canopy reads hottest and the centre of the view stays clear.
    let rim = 1.0 - canopy_glass(in.ndc, aspect);
    let hull_rim = smoothstep(0.0, 0.38, rim);
    rgb += blackbody(hull_kk) * g_hull * (0.02 + 0.30 * hull_rim);

    let out = tonemap(rgb, exposure);
    // Additive: alpha carries nothing, the blend is ONE + ONE.
    return vec4<f32>(out + dither_px(in.pos.xy), 0.0);
}
