// nebula.wgsl — the nebula: coloured gas across the sky, baked once
// (SPEC §6.5; P2's "bake, don't re-derive").
//
// Lane: A (fragment only). Cost class: one-off bake of a 4096×2048 equirect
// with mips whenever a shape knob changes (~100 ms of GPU, a discrete event), then ONE fetch per pixel inside
// the starfield pass (starfield.wgsl `nebula()`). Nothing per frame here.
//
// Technique: a handful of soft lobes on the sphere pick where the clouds
// are (so the sky is mostly black, as it should be — a nebula is a place,
// not weather). Inside a lobe a domain-warped fbm gives the gas its banks
// and filaments, a second, finer field lights the emission cores, a third
// cuts dark dust lanes through it. Two hues bleed into each other along a
// slow field so a cloud is never one flat colour. Every knob is a uniform:
// the menu re-bakes, it never recomputes live.

struct Params {
    // x: seed, y: scale (feature frequency across the sky, 1..8),
    // z: density 0..1 (how much of a cloud is gas), w: cloud count 1..8
    shape: vec4<f32>,
    // xyz: first hue, linear rgb; w: intensity (0 = off, the bake is black)
    col_a: vec4<f32>,
    // xyz: second hue; w: lobe softness (how wide each cloud spreads)
    col_b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> nb: Params;
@group(0) @binding(1) var mip_src: texture_2d<f32>;
@group(0) @binding(2) var mip_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

const PI: f32 = 3.14159265;

// Equirect texel -> direction on the unit sphere (same convention as
// bake.wgsl and starfield.wgsl's milky_way(): lon about +y from +x).
fn uv_to_dir(uv: vec2<f32>) -> vec3<f32> {
    let lon = (uv.x - 0.5) * 2.0 * PI;
    let lat = (0.5 - uv.y) * PI;
    let c = cos(lat);
    return vec3<f32>(c * cos(lon), sin(lat), c * sin(lon));
}

// A pseudo-random unit direction for cloud `i` of this seed.
fn lobe_dir(seed: i32, i: i32) -> vec3<f32> {
    let u = hash31(vec3<i32>(seed * 3 + 1, i * 7 + 2, 19));
    let v = hash31(vec3<i32>(seed * 5 + 3, i * 11 + 4, 23));
    let z = u * 2.0 - 1.0;
    let r = sqrt(max(1.0 - z * z, 0.0));
    let phi = v * 2.0 * PI;
    return vec3<f32>(r * cos(phi), z, r * sin(phi));
}

// Where the clouds are: soft lobes, summed and clamped, so two clouds that
// touch merge into one larger complex instead of showing a seam.
fn cloud_mask(dir: vec3<f32>, seed: i32, count: i32, softness: f32) -> f32 {
    var m = 0.0;
    for (var i = 0; i < count; i += 1) {
        let c = lobe_dir(seed, i);
        // Width varies per cloud: some a broad veil, some a knot.
        let w = hash31(vec3<i32>(seed * 7 + 5, i * 13 + 6, 29));
        let k = mix(14.0, 4.0, w) / max(softness, 0.05);
        m += exp(-(1.0 - dot(dir, c)) * k);
    }
    return clamp(m, 0.0, 1.0);
}

@fragment
fn fs_bake(in: VsOut) -> @location(0) vec4<f32> {
    let dir = uv_to_dir(in.uv);
    let seed_f = nb.shape.x;
    let seed = i32(seed_f);
    let scale = nb.shape.y;
    let density = nb.shape.z;
    let count = clamp(i32(nb.shape.w), 1, 8);
    let intensity = nb.col_a.w;
    if (intensity <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let s = vec3<f32>(seed_f * 17.3, seed_f * 5.1, seed_f * 9.7);
    let mask = cloud_mask(dir, seed, count, nb.col_b.w);
    if (mask < 0.002) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    // Domain warp: the gas has been pushed about by something.
    let warp = vec3<f32>(
        fbm3(dir * scale * 0.9 + s),
        fbm3(dir * scale * 0.9 + s + vec3<f32>(4.1, 7.7, 2.3)),
        fbm3(dir * scale * 0.9 + s + vec3<f32>(9.2, 1.9, 6.6)),
    ) - vec3<f32>(0.5);
    let p = dir * scale + warp * 1.6;

    // Banks: five octaves, contrast expanded (fbm hugs its mean, and a
    // low-contrast field thresholds into a veil, not clouds), then the
    // coverage threshold set by the density knob.
    var field = 0.5 + (fbm5(p + s) - 0.5) * 2.4;
    let gas = smoothstep(1.0 - density, 1.0 - density + 0.55, field);

    // Filaments: a finer warp, then five octaves of ridged noise (1 - |2n-1|,
    // squared: sharp bright threads on a dark ground) — the emission cores,
    // brightest where the banks are thickest. The bake can afford the
    // octaves; the fetch never sees them as anything but one texel.
    let warp2 = vec3<f32>(
        fbm3(p * 3.3 + s + vec3<f32>(21.0, 4.0, 11.0)),
        fbm3(p * 3.3 + s + vec3<f32>(2.0, 27.0, 14.0)),
        fbm3(p * 3.3 + s + vec3<f32>(15.0, 8.0, 33.0)),
    ) - vec3<f32>(0.5);
    let q = p + warp2 * 0.45;
    var ridge_sum = 0.0;
    var amp = 0.5;
    var freq = 2.7;
    var norm = 0.0;
    for (var i = 0; i < 5; i += 1) {
        let v = vnoise(q * freq + s + vec3<f32>(31.0, 17.0, 5.0));
        let r = 1.0 - abs(v * 2.0 - 1.0);
        ridge_sum += amp * r * r;
        norm += amp;
        amp *= 0.55;
        freq *= 2.1;
    }
    let ridge = ridge_sum / norm;
    let cores = pow(ridge, 2.6) * gas;
    // The finest threads: one high octave of ridged noise, a lace over the
    // gas that only shows at full resolution.
    let lace = pow(1.0 - abs(vnoise(q * 41.0 + s) * 2.0 - 1.0), 5.0);

    // Dust lanes: cold, dark gas in front of the glow — a broad lane with a
    // ragged, finer edge.
    let dust = fbm3(p * 3.1 + s + vec3<f32>(41.0, 3.0, 27.0));
    let dust_fine = fbm3(p * 11.0 + s + vec3<f32>(3.0, 44.0, 9.0));
    let lane = smoothstep(0.56, 0.80, dust + (dust_fine - 0.5) * 0.18) * 0.88;

    // Hue drifts across the cloud along a slow field.
    let t = fbm3(dir * scale * 0.45 + s + vec3<f32>(13.1, 2.2, 8.8));
    let hue = mix(nb.col_a.xyz, nb.col_b.xyz, smoothstep(0.3, 0.7, t));
    // Emission cores are the hue pushed toward white; the lace a little
    // more so.
    let core_col = mix(hue, vec3<f32>(1.0), 0.35);
    let lace_col = mix(hue, vec3<f32>(1.0), 0.55);

    let glow = (hue * gas * 0.32 + core_col * cores * 0.95 + lace_col * lace * gas * 0.35)
        * (1.0 - lane);
    // Absolute scale: the Milky Way's baked glow peaks near 0.045, so 1.0
    // intensity puts a bright bank at about three times the galaxy and a
    // veil well below it.
    let col = glow * mask * 0.18 * intensity;
    // Alpha: how much gas is in the way — the starfield dims stars behind
    // a thick bank a little.
    let a = clamp(gas * mask * (1.0 - lane * 0.5), 0.0, 1.0);
    return vec4<f32>(max(col, vec3<f32>(0.0)), a);
}

// Plain 2×2 box downsample for the mip chain, so the starfield's
// gradient fetch picks a footprint-matched level.
@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(mip_src, mip_samp, in.uv, 0.0);
}
