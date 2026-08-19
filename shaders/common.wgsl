// common.wgsl — shader prelude, prepended to every pass (see render/src/shaders.rs).
//
// WGSL has no #include, so composition happens in Rust. Everything here is
// shared by two or more passes; duplicating a noise function across passes is
// how two parts of the same world quietly stop agreeing with each other.

// ---------------------------------------------------------------- hashing

// pcg2d-style integer hash -> 4 independent floats in [0,1).
fn hash4(cell: vec2<i32>) -> vec4<f32> {
    var v = vec2<u32>(bitcast<u32>(cell.x), bitcast<u32>(cell.y));
    v = v * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1013904223u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1013904223u;
    v = v ^ (v >> vec2<u32>(16u));
    return vec4<f32>(
        f32(v.x & 0xffffu) / 65536.0,
        f32(v.x >> 16u) / 65536.0,
        f32(v.y & 0xffffu) / 65536.0,
        f32(v.y >> 16u) / 65536.0,
    );
}

// Three decorrelated odd multipliers and a two-round finaliser.
//
// The obvious choice of 2147483647 (2^31 - 1) for one of the multipliers is a
// trap: z * (2^31 - 1) == (z << 31) - z, so the z term degenerates to -z plus a
// parity bit, and one round of mixing cannot hide it. Measured lattice
// correlation was -0.27 at a z step of 2 — an anisotropic stripe running
// through every noise field built on this hash.
fn hash31(p: vec3<i32>) -> f32 {
    var h = u32(p.x) * 0x8da6b343u + u32(p.y) * 0xd8163841u + u32(p.z) * 0xcb1ab31fu;
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    return f32(h ^ (h >> 15u)) / 4294967296.0;
}

// ------------------------------------------------------------------ noise

fn vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(
            mix(hash31(i + vec3<i32>(0, 0, 0)), hash31(i + vec3<i32>(1, 0, 0)), u.x),
            mix(hash31(i + vec3<i32>(0, 1, 0)), hash31(i + vec3<i32>(1, 1, 0)), u.x),
            u.y,
        ),
        mix(
            mix(hash31(i + vec3<i32>(0, 0, 1)), hash31(i + vec3<i32>(1, 0, 1)), u.x),
            mix(hash31(i + vec3<i32>(0, 1, 1)), hash31(i + vec3<i32>(1, 1, 1)), u.x),
            u.y,
        ),
        u.z,
    );
}

// Normalised to [0,1], like fbm5. The amplitudes sum to 0.875, so leaving the
// division out shifts the mean to 0.4375 — and every threshold written against
// it as though 0.5 were the midpoint then sits in the wrong place, silently.
fn fbm3(p: vec3<f32>) -> f32 {
    let sum = 0.5 * vnoise(p) + 0.25 * vnoise(p * 2.03) + 0.125 * vnoise(p * 4.07);
    return sum / 0.875;
}

// Detail-limited fbm: octaves are added until they reach the resolution limit
// `max_freq`, then stop.
//
// This is the whole "spend samples where they read" idea in one function. A
// fixed octave count is wrong in both directions at once: from orbit the fine
// octaves land far below a pixel and cost real time to contribute nothing but
// shimmer, while up close the field runs out of octaves entirely and the
// surface goes smooth. Driving the count from the pixel's footprint gives
// cheap distant worlds and detailed near ones from the same call.
//
// The last octave fades in rather than appearing, so crossing a threshold is
// invisible; and the sum is normalised by the weight actually accumulated, so
// the field keeps a mean of 0.5 at any octave count — otherwise every coastline
// would creep as the ship approached it.
fn fbm_lod(p: vec3<f32>, max_freq: f32, max_octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0; i < max_octaves; i += 1) {
        let weight = clamp(max_freq / freq - 1.0, 0.0, 1.0);
        if (weight <= 0.0) {
            break;
        }
        sum += amp * weight * vnoise(p * freq);
        norm += amp * weight;
        amp *= 0.5;
        freq *= 2.03;
    }
    return sum / max(norm, 1e-6);
}

// Normalised to [0,1]: five octaves, for terrain-scale structure.
fn fbm5(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0; i < 5; i += 1) {
        sum += amp * vnoise(p * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    return sum / norm;
}

// ------------------------------------------------------------------ output

// Exposure + soft shoulder. No history buffers, no temporal accumulation (P1).
fn tonemap(col: vec3<f32>, exposure: f32) -> vec3<f32> {
    return vec3<f32>(1.0) - exp(-max(col, vec3<f32>(0.0)) * exposure);
}

// +-0.5/255 of hash noise: kills 8-bit banding without temporal shimmer.
fn dither_px(px: vec2<f32>) -> f32 {
    return (hash31(vec3<i32>(vec3<f32>(px, 17.0))) - 0.5) / 255.0;
}

// ------------------------------------------------------------------ camera

// View ray for a pixel, from the camera basis. Camera-relative rendering means
// the camera is always at the origin, so this is the whole camera model.
fn view_ray(ndc: vec2<f32>, right: vec3<f32>, up: vec3<f32>, forward: vec3<f32>,
            tan_half_fov: f32, aspect: f32) -> vec3<f32> {
    return normalize(
        forward + right * (ndc.x * tan_half_fov * aspect) + up * (ndc.y * tan_half_fov)
    );
}

// Fullscreen triangle vertex: (-1,-1), (3,-1), (-1,3).
fn fullscreen_ndc(vertex_index: u32) -> vec2<f32> {
    return vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u)) * 2.0 - 1.0;
}
