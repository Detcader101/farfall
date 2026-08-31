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
// map seams, so star density varies slightly by direction. Judge by eye;
// fallback is 3-plane cube hashing.
//
// The fold. oct_encode folds the back hemisphere out to the map's edges, so
// the world half-planes x=0,z<0 and y=0,z<0 are the map's four edges, each
// glued to itself mirrored (u,1)~(-u,1). Two things follow, both handled in
// stars(): (1) the grid coordinate p is discontinuous there, so its screen
// derivatives are garbage for any pixel quad straddling the fold — the
// star's pixel footprint is measured from the derivatives of the view ray
// instead, which are smooth everywhere (the old grid Jacobian drew a column
// of every neighbouring star at full size along the fold); (2) a cell past
// the edge is the mirror of a real cell, so the neighbourhood search wraps
// through the mirror (`oct_true_cell`) and the same star is found from both
// sides of the fold, at its true direction (`oct_decode` of its true cell).
// `starfield.rs` mirrors the wrap in Rust under test.

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

// A grid cell past one of the map's edges is the mirror image of a real
// cell on the other side of the fold: (u, 1+e) ~ (-u, 1-e) on the top edge,
// and the same on each side. Returns the real cell; a cell inside the map
// is its own. Corners (both edges at once) are the -Z axis and are left as
// they fall.
fn oct_true_cell(cell: vec2<i32>, grid: i32) -> vec2<i32> {
    var c = cell;
    if (c.y >= grid) {
        c = vec2<i32>(grid - 1 - c.x, 2 * grid - 1 - c.y);
    } else if (c.y < 0) {
        c = vec2<i32>(grid - 1 - c.x, -1 - c.y);
    } else if (c.x >= grid) {
        c = vec2<i32>(2 * grid - 1 - c.x, grid - 1 - c.y);
    } else if (c.x < 0) {
        c = vec2<i32>(-1 - c.x, grid - 1 - c.y);
    }
    return c;
}

// The same mirror for a position in the map: where a point of the real
// cell lands in the extended chart of the pixel that reached past the edge.
fn oct_mirror_pos(pos: vec2<f32>, cell: vec2<i32>, grid: i32) -> vec2<f32> {
    let g = f32(grid);
    if (cell.y >= grid) {
        return vec2<f32>(g - pos.x, 2.0 * g - pos.y);
    } else if (cell.y < 0) {
        return vec2<f32>(g - pos.x, -pos.y);
    } else if (cell.x >= grid) {
        return vec2<f32>(2.0 * g - pos.x, g - pos.y);
    } else if (cell.x < 0) {
        return vec2<f32>(-pos.x, g - pos.y);
    }
    return pos;
}

fn stars(dir: vec3<f32>) -> vec3<f32> {
    let grid = 192.0 * STAR_DENSITY;
    let grid_i = i32(grid);
    let p = (oct_encode(dir) * 0.5 + 0.5) * grid;
    // The 2×2 cells about this point: a star is kept well inside its cell
    // (0.15..0.85) and its support ends at 0.65 cells, so the two nearest
    // cells on each axis are the only ones that can reach here. Cells are
    // a dozen pixels and more; a star's core is a few.
    let base = vec2<i32>(floor(p - 0.5));

    // The pixel's footprint on the sky: the view ray's screen derivatives,
    // smooth everywhere (the grid's own derivatives blow up across the
    // octahedral fold — see the header). Star distances are measured in
    // PIXELS: the offset from a star's true direction to this ray, solved
    // against the two tangent vectors (a 2×2 Gram system), so every star is
    // the same round few-pixel dot regardless of the map's distortion
    // (anisotropic — a scalar normalisation leaves stars elliptical),
    // perspective stretch at wide FOV, or resolution. A point star's
    // footprint is a camera property, not a map property.
    let ddx = dpdx(dir);
    let ddy = dpdy(dir);
    let gxx = dot(ddx, ddx);
    let gxy = dot(ddx, ddy);
    let gyy = dot(ddy, ddy);
    // The Gram determinant is a squared area, never negative; clamped so a
    // degenerate quad gives a huge distance (no star), never inf * 0 = NaN.
    let inv_det = 1.0 / max(gxx * gyy - gxy * gxy, 1e-24);

    var col = vec3<f32>(0.0);
    for (var dy = 0; dy <= 1; dy += 1) {
        for (var dx = 0; dx <= 1; dx += 1) {
            let cell = base + vec2<i32>(dx, dy);
            let true_cell = oct_true_cell(cell, grid_i);
            let h = hash4(true_cell);
            // ~55% of cells host a star.
            if (h.z > 0.55) {
                continue;
            }
            // Keep the star inside its cell so the 2×2 search always covers it.
            let true_pos = vec2<f32>(true_cell) + h.xy * 0.7 + 0.15;
            let star_pos = oct_mirror_pos(true_pos, cell, grid_i);
            let d = length(p - star_pos);
            // The magnitude law, in radiance: most stars faint (a fiftieth
            // to a third of white), a few per screen brilliant (past white,
            // where the post pass's bloom starts), the rare one blazing.
            // Steep on purpose: the sky reads as depth only when nearly
            // everything is a pinprick and the bright ones are rare.
            let m = h.w;
            let m4 = m * m * m * m;
            let m16 = m4 * m4 * m4 * m4;
            let mag = 0.02 + 0.35 * m4 + 1.5 * m16 * m4 + 30.0 * m16 * m16 * m16 * m16 * m16;
            // Compact support is load-bearing: a tight gaussian core, windowed
            // to zero well inside the search radius. A falloff with an infinite
            // tail (e.g. 1/(1+d²)) clips at the neighborhood edge and turns the
            // sky into a quilt of glowing cells.
            let window = smoothstep(0.65, 0.3, d);
            if (window <= 0.0) {
                continue;
            }
            // The star's true direction, and this ray's offset from it in
            // pixels: least squares of v = a·ddx + b·ddy in the tangent plane.
            let star_dir = oct_decode(true_pos / grid * 2.0 - 1.0);
            let v = dir - star_dir;
            let bx = dot(v, ddx);
            let by = dot(v, ddy);
            let a = (bx * gyy - by * gxy) * inv_det;
            let b = (by * gxx - bx * gxy) * inv_det;
            let d_px2 = a * a + b * b;
            // A sub-pixel core (sigma ~0.6 px): a point, which the MSAA
            // resolve and the bloom soften exactly as far as they should
            // and no further. The old sigma of 1.1 px was the "snow".
            let core = mag * exp(-d_px2 * 1.4) * window;
            // Colour by temperature — and more of it on the bright ones,
            // where the eye actually sees a star's colour.
            let tint = star_tint(fract((h.x + h.y) * 7.91));
            col += mix(vec3<f32>(1.0), tint, 0.55 + 0.45 * min(mag, 1.0)) * core;
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
        star_light = acc / wsum * (1.0 + 4.0 * stretch);
    }
    // Gas in front of a star takes a little of its light; the glow of the
    // gas adds over everything.
    let neb = nebula(dir);
    star_light *= 1.0 - neb.a * 0.35;
    // The baked Milky Way at a third: under the HDR exposure the bake read
    // as a blue-grey fog over the whole sky with the nebula off; a faint
    // band is what it is.
    var col = radiance(star_light + milky_way(dir) * 0.35 + neb.rgb, exposure);
    col += vec3<f32>(dither_px(in.pos.xy));
    return vec4<f32>(max(col, vec3<f32>(0.0)), 1.0);
}
