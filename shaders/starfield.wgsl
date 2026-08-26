// starfield.wgsl — procedural full-sky starfield (SPEC §6.5, pass: starfield)
//
// Lane: A (vertex+fragment only). Cost class: cheap (one fullscreen pass,
// 9 hash taps + one fetch of the baked Milky Way). No assets.
//
// Technique: fullscreen triangle; per-pixel view ray from camera basis; ray
// direction → octahedral map → 2D grid cells; one candidate star per cell from
// an integer hash (position jitter, brightness power-law, temperature tint).
// A Milky Way band adds low-frequency structure. Hash dithering kills banding
// (P1: no temporal noise, no smear).
//
// Known limitation (SPEC §11.1): octahedral mapping distorts cell area near the
// map seams, so star density/size varies slightly by direction. Judge by eye;
// fallback is 3-plane cube hashing.

// Quality knob (SPEC §6.2/§6.3): grid resolution multiplier. Tiers override this
// at pipeline creation; 1.0 ≈ a few thousand visible stars.
override STAR_DENSITY: f32 = 1.0;

struct Frame {
    // Camera basis, world space (camera-relative rendering: no translation).
    // right.w: the stars' stretch 0..1 — at speed every star draws out
    // into a streak away from the centre of the view, the old Star Trek
    // way: the picture's exposure dragging as the sky rushes past.
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect (w/h), z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: an opaque sphere in front of the sky, camera-relative metres;
    // w: its radius, 0 for none. Stars behind it are never shaded.
    occluder: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;
@group(0) @binding(1) var sky_tex: texture_2d<f32>;
@group(0) @binding(2) var sky_samp: sampler;
// The nebula, baked (nebula.wgsl): rgb glow, a = how much gas is in the way.
@group(0) @binding(3) var nebula_tex: texture_2d<f32>;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Fullscreen triangle: (-1,-1), (3,-1), (-1,3)
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

// Octahedral mapping: oct_encode() in the prelude (shared with the thermal
// field, which stores its hull map in the same projection).

// ---------------------------------------------------------------- stars

// Approximate stellar tint from a temperature parameter in [0,1):
// 0 → cool orange, 0.5 → white, 1 → hot blue-white.
fn star_tint(t: f32) -> vec3<f32> {
    let cool = vec3<f32>(1.0, 0.62, 0.36);
    let mid = vec3<f32>(1.0, 0.96, 0.90);
    let hot = vec3<f32>(0.68, 0.78, 1.0);
    if (t < 0.5) {
        return mix(cool, mid, t * 2.0);
    }
    return mix(mid, hot, (t - 0.5) * 2.0);
}

fn stars(dir: vec3<f32>) -> vec3<f32> {
    let grid = 192.0 * STAR_DENSITY;
    let p = (oct_encode(dir) * 0.5 + 0.5) * grid;
    // The 2×2 cells about this point: a star is kept well inside its cell
    // (0.15..0.85) and its support ends at 0.65 cells, so the two nearest
    // cells on each axis are the only ones that can reach here. Cells are
    // a dozen pixels and more; a star's core is a few.
    let base = vec2<i32>(floor(p - 0.5));

    // Screen-space Jacobian of the grid coords. Star distances are measured in
    // PIXELS by pushing the grid-space offset through the FULL inverse Jacobian:
    // that makes every star the same round few-pixel dot regardless of
    // octahedral distortion (which is anisotropic — a scalar/RMS normalization
    // leaves stars elliptical), perspective stretch at wide FOV, or resolution.
    // A point star's footprint is a camera property, not a map property.
    // det clamped: derivatives explode across oct seams (the grid-space window
    // below still bounds any garbage to one cell neighborhood).
    let jx = vec2<f32>(dpdx(p.x), dpdy(p.x));
    let jy = vec2<f32>(dpdx(p.y), dpdy(p.y));
    let det = jx.x * jy.y - jx.y * jy.x;
    // sign(0.0) is 0.0, so the obvious `sign(det) * max(abs(det), eps)` yields
    // 1/0 = inf on an exactly degenerate Jacobian, and inf * 0 = NaN for a
    // fragment sitting precisely on a star's centre.
    let det_mag = max(abs(det), 1e-8);
    let inv_det = 1.0 / select(det_mag, -det_mag, det < 0.0);

    var col = vec3<f32>(0.0);
    for (var dy = 0; dy <= 1; dy += 1) {
        for (var dx = 0; dx <= 1; dx += 1) {
            let cell = base + vec2<i32>(dx, dy);
            let h = hash4(cell);
            // ~55% of cells host a star.
            if (h.z > 0.55) {
                continue;
            }
            // Keep the star inside its cell so the 3×3 search always covers it.
            let star_pos = vec2<f32>(cell) + h.xy * 0.7 + 0.15;
            let d = length(p - star_pos);
            // Power-law brightness: many dim, few brilliant.
            let mag = pow(h.w, 14.0) * 60.0 + pow(h.w, 4.0) * 1.2 + 0.02;
            // Compact support is load-bearing: a tight gaussian core, windowed
            // to zero well inside the search radius. A falloff with an infinite
            // tail (e.g. 1/(1+d²)) clips at the neighborhood edge and turns the
            // sky into a quilt of glowing cells.
            let window = smoothstep(0.65, 0.3, d);
            let v = p - star_pos;
            let offs_px = vec2<f32>(jy.y * v.x - jx.y * v.y, -jy.x * v.x + jx.x * v.y) * inv_det;
            let d_px2 = dot(offs_px, offs_px);
            let core = mag * exp(-d_px2 * 0.4) * window;
            col += star_tint(fract((h.x + h.y) * 7.91)) * core;
        }
    }
    return col;
}

// ---------------------------------------------------------------- sky

// The Milky Way is baked (bake.wgsl, fs_sky): one equirect fetch.
fn milky_way(dir: vec3<f32>) -> vec3<f32> {
    let uv = vec2<f32>(
        atan2(dir.z, dir.x) / 6.28318531 + 0.5,
        acos(clamp(dir.y, -1.0, 1.0)) / 3.14159265,
    );
    var du = dpdx(uv);
    var dv = dpdy(uv);
    du.x = fract(du.x + 0.5) - 0.5;
    dv.x = fract(dv.x + 0.5) - 0.5;
    return textureSampleGrad(sky_tex, sky_samp, uv, du, dv).rgb;
}

// The nebula: same equirect, same one fetch.
fn nebula(dir: vec3<f32>) -> vec4<f32> {
    let uv = vec2<f32>(
        atan2(dir.z, dir.x) / 6.28318531 + 0.5,
        acos(clamp(dir.y, -1.0, 1.0)) / 3.14159265,
    );
    var du = dpdx(uv);
    var dv = dpdy(uv);
    du.x = fract(du.x + 0.5) - 0.5;
    dv.x = fract(dv.x + 0.5) - 0.5;
    return textureSampleGrad(nebula_tex, sky_samp, uv, du, dv);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tan_half_fov = frame.params.x;
    let aspect = frame.params.y;
    let exposure = frame.params.w;

    let dir = view_ray(
        in.ndc, frame.right.xyz, frame.up.xyz, frame.forward.xyz, tan_half_fov, aspect,
    );

    // The planet is opaque and drawn over this: stars under its disc are
    // work that ends up behind a wall. Low and level — the normal view —
    // that was most of the screen. A few pixels inside the limb are left
    // to the planet pass's own analytic edge, so the seam is its, not ours.
    let occ_r = frame.occluder.w;
    if (occ_r > 0.0) {
        let d = length(frame.occluder.xyz);
        if (d > occ_r) {
            let cos_limb = sqrt(1.0 - (occ_r * occ_r) / (d * d));
            let cos_view = dot(dir, frame.occluder.xyz / d);
            if (cos_view > cos_limb + 0.004) {
                return vec4<f32>(0.0, 0.0, 0.0, 1.0);
            }
        }
    }

    let stretch = frame.right.w;
    var star_light = stars(dir);
    if (stretch > 0.001) {
        // The streak: the same star field sampled a few times along the
        // line from this pixel toward the centre of the view, the tail
        // fading — so each star trails outward from where it is.
        let q = in.ndc * vec2<f32>(aspect, 1.0);
        let r = length(q);
        let reach = stretch * (0.06 + 0.22 * r);
        var acc = star_light;
        var wsum = 1.0;
        for (var k = 1; k <= 5; k += 1) {
            let s = f32(k) / 5.0;
            let ndc_k = in.ndc * (1.0 - reach * s);
            let dir_k = view_ray(ndc_k, frame.right.xyz, frame.up.xyz, frame.forward.xyz, tan_half_fov, aspect);
            let w = 1.0 - s * 0.75;
            acc += stars(dir_k) * w;
            wsum += w;
        }
        // Brighter than the mean: a streak is the star's light spread out,
        // and the eye reads the long faint line as the brilliant star it
        // came from.
        star_light = acc / wsum * (1.0 + 1.5 * stretch);
    }
    // Gas in front of a star takes a little of its light; the glow of the
    // gas adds over everything.
    let neb = nebula(dir);
    star_light *= 1.0 - neb.a * 0.35;
    var col = tonemap(star_light + milky_way(dir) + neb.rgb, exposure);
    col += vec3<f32>(dither_px(in.pos.xy));
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
