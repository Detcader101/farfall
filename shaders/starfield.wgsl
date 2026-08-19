// starfield.wgsl — procedural full-sky starfield (SPEC §6.5, pass: starfield)
//
// Lane: A (vertex+fragment only). Cost class: cheap (one fullscreen pass,
// 9 hash taps + a 3-octave fbm). No textures, no assets.
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
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect (w/h), z: time_s, w: exposure
    params: vec4<f32>,
}

@group(0) @binding(0) var<uniform> frame: Frame;

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

// ------------------------------------------------------------- octahedral map

fn sign_not_zero(v: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(select(-1.0, 1.0, v.x >= 0.0), select(-1.0, 1.0, v.y >= 0.0));
}

// Unit direction → octahedral UV in [-1,1]².
fn oct_encode(d: vec3<f32>) -> vec2<f32> {
    let n = d / (abs(d.x) + abs(d.y) + abs(d.z));
    var uv = n.xy;
    if (n.z < 0.0) {
        uv = (1.0 - abs(n.yx)) * sign_not_zero(n.xy);
    }
    return uv;
}

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
    let base = vec2<i32>(floor(p));

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
    let inv_det = 1.0 / (sign(det) * max(abs(det), 1e-8));

    var col = vec3<f32>(0.0);
    // 3×3 neighborhood so stars survive cell boundaries.
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            let cell = base + vec2<i32>(dx, dy);
            let h = hash4(cell);
            // ~55% of cells host a star.
            if (h.z > 0.55) {
                continue;
            }
            // Keep the star inside its cell so the 3×3 search always covers it.
            let star_pos = vec2<f32>(cell) + h.xy * 0.8 + 0.1;
            let d = length(p - star_pos);
            // Power-law brightness: many dim, few brilliant.
            let mag = pow(h.w, 14.0) * 60.0 + pow(h.w, 4.0) * 1.2 + 0.02;
            // Compact support is load-bearing: a tight gaussian core, windowed
            // to zero well inside the search radius. A falloff with an infinite
            // tail (e.g. 1/(1+d²)) clips at the neighborhood edge and turns the
            // sky into a quilt of glowing cells.
            let window = smoothstep(1.2, 0.6, d);
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

const GALACTIC_NORMAL: vec3<f32> = vec3<f32>(0.2588, 0.9330, 0.2500); // tilted band

fn milky_way(dir: vec3<f32>) -> vec3<f32> {
    let lat = dot(dir, GALACTIC_NORMAL);
    let band = exp(-lat * lat * 28.0);
    let patchiness = fbm3(dir * 7.0) * 0.75 + 0.25;
    let dust = fbm3(dir * 15.0 + 31.7);
    // Warm core glow occluded by cooler dust lanes.
    let glow = vec3<f32>(0.045, 0.042, 0.055) * band * patchiness;
    let lane = vec3<f32>(0.030, 0.024, 0.020) * band * smoothstep(0.55, 0.85, dust);
    return max(glow - lane, vec3<f32>(0.0));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let tan_half_fov = frame.params.x;
    let aspect = frame.params.y;
    let exposure = frame.params.w;

    let dir = view_ray(
        in.ndc, frame.right.xyz, frame.up.xyz, frame.forward.xyz, tan_half_fov, aspect,
    );

    var col = tonemap(stars(dir) + milky_way(dir), exposure);
    col += vec3<f32>(dither_px(in.pos.xy));
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
