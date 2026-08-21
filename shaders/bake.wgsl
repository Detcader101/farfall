// bake.wgsl — one-time field generation (SPEC §6.5; the "bake, don't
// re-derive" half of P2).
//
// Lane: A (fragment only — no compute, so the floor hardware can bake too).
// Cost class: one-off at startup, then free.
//
// The continent field never changes, yet the planet pass was evaluating a
// nine-octave noise stack for it on every covered pixel of every frame —
// five million times a second to keep getting the same answer. These entry
// points render the static fields into equirect textures once; the planet
// pass then pays one texture fetch where it paid ~forty hash evaluations.
// Nothing ships in the repo: the GPU authors the world at load, which keeps
// the asset budget at zero while the per-frame cost stops scaling with what
// the world looks like.

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

// Equirect texel -> direction on the unit sphere.
fn uv_to_dir(uv: vec2<f32>) -> vec3<f32> {
    let lon = (uv.x - 0.5) * 2.0 * PI;
    let lat = (0.5 - uv.y) * PI;
    let c = cos(lat);
    return vec3<f32>(c * cos(lon), sin(lat), c * sin(lon));
}

// Surface fields, packed per channel:
//   R: elevation, already expanded about the 0.5 midpoint (what the planet
//      pass consumes directly — see the central-limit note in planet.wgsl)
//   G: dryness, likewise expanded
//   B: settlement/light speckle field
//   A: ice-edge noise
@fragment
fn fs_surface(in: VsOut) -> @location(0) vec4<f32> {
    let n = uv_to_dir(in.uv);

    // Nine octaves: at bake time depth costs once, so the ground detail that
    // was too expensive to compute live is simply present in the texture.
    var elev_raw = 0.0;
    var amp = 0.5;
    var freq = 1.7;
    var norm = 0.0;
    for (var i = 0; i < 9; i += 1) {
        elev_raw += amp * vnoise(n * freq + 11.3);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    let elevation = 0.5 + (elev_raw / norm - 0.5) * 2.9;

    let dryness = 0.5 + (fbm3(n * 4.0 + 4.7) - 0.5) * 2.2;
    let lights = fbm3(n * 150.0);
    let ice = fbm3(n * 7.0);
    return vec4<f32>(elevation, dryness, lights, ice);
}

// Cloud field, pre-threshold. Coverage, sharpness and drift are applied live
// in the planet pass, so cycling atmosphere presets needs no re-bake and the
// deck still moves; only the underlying weather pattern is frozen.
@fragment
fn fs_cloud(in: VsOut) -> @location(0) vec4<f32> {
    let n = uv_to_dir(in.uv);
    let warp = vec2<f32>(
        vnoise(n * 2.6),
        vnoise(n * 2.6 + vec3<f32>(5.2, 1.3, 8.7)),
    ) - vec2<f32>(0.5);
    let warped = n * 5.5 + vec3<f32>(warp.x, warp.y * 0.45, warp.y) * 1.7;

    // Five octaves, then expand the deviation. fbm concentrates around its
    // mean (central limit), and a low-contrast field turns the live coverage
    // threshold into a thin veil over everything instead of banks with holes —
    // measured as an ~85% white blanket at 42% coverage before the expansion.
    var field = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0; i < 5; i += 1) {
        field += amp * vnoise(warped * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    field = 0.5 + (field / norm - 0.5) * 2.4;
    return vec4<f32>(clamp(field, 0.0, 1.0), 0.0, 0.0, 1.0);
}

// ------------------------------------------------------------------ sky

const GALACTIC_NORMAL: vec3<f32> = vec3<f32>(0.2588, 0.9330, 0.2500); // tilted band

// The Milky Way: a band of low-frequency structure across the whole sky.
// It was two three-octave fbm stacks per pixel per frame in the starfield
// pass — forty-eight hash evaluations to draw something that never changes
// and spans a thousand pixels per feature. Baked once, sampled with mips,
// it costs one fetch and looks identical.
@fragment
fn fs_sky(in: VsOut) -> @location(0) vec4<f32> {
    let dir = uv_to_dir(in.uv);
    let lat = dot(dir, GALACTIC_NORMAL);
    let band = exp(-lat * lat * 28.0);
    let patchiness = fbm3(dir * 7.0) * 0.75 + 0.25;
    let dust = fbm3(dir * 15.0 + 31.7);
    // Warm core glow occluded by cooler dust lanes.
    let glow = vec3<f32>(0.045, 0.042, 0.055) * band * patchiness;
    // fbm3 is normalised now; its old un-normalised maximum was 0.826, so an
    // upper knee at 0.85 was simply unreachable and the dust never saturated.
    let lane = vec3<f32>(0.030, 0.024, 0.020) * band * smoothstep(0.54, 0.78, dust);
    return vec4<f32>(max(glow - lane, vec3<f32>(0.0)), 1.0);
}

// ---------------------------------------------------------------- noise

// Value noise that tiles with the given integer period: the lattice
// coordinates wrap before hashing, so the texture's edges are seamless.
fn vnoise_tiled(p: vec2<f32>, period: i32) -> f32 {
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let i0 = ((i % period) + period) % period;
    let i1 = (i0 + 1) % period;
    let a = hash31(vec3<i32>(i0.x, i0.y, 7));
    let b = hash31(vec3<i32>(i1.x, i0.y, 7));
    let c = hash31(vec3<i32>(i0.x, i1.y, 7));
    let d = hash31(vec3<i32>(i1.x, i1.y, 7));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// A seamless 2D fbm tile, for anything that needs animated texture rather
// than a field tied to a direction — the plasma sheath scrolls it back from
// the nose. Four octaves, normalised to [0,1] like fbm5.
@fragment
fn fs_noise(in: VsOut) -> @location(0) vec4<f32> {
    var sum = 0.0;
    var amp = 0.5;
    var period = 8;
    var norm = 0.0;
    for (var i = 0; i < 4; i += 1) {
        sum += amp * vnoise_tiled(in.uv * f32(period), period);
        norm += amp;
        amp *= 0.5;
        period *= 2;
    }
    return vec4<f32>(sum / norm, 0.0, 0.0, 1.0);
}

// ------------------------------------------------------------- mip chain

@group(0) @binding(0) var mip_src: texture_2d<f32>;
@group(0) @binding(1) var mip_samp: sampler;

// Plain 2x2 box downsample. The mips exist so the planet pass can sample with
// gradients and get the footprint-matched level automatically — which replaces
// the hand-rolled octave-LOD logic AND its aliasing at the same time.
@fragment
fn fs_downsample(in: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(mip_src, mip_samp, in.uv, 0.0);
}
