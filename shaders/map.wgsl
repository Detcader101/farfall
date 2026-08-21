// map.wgsl — the system map (pass: map)
//
// Lane: A. Cost class: trivial — a handful of SDFs per pixel, and only
// while the MAP page is open.
//
// The system at log scale: the Moon is 60 planet radii out and the Sun
// 23,000, so a linear map shows either nothing or a dot. Rings of
// distance from the planet, one per decade; the Moon on its orbit, the
// Sun on its line, the ship where it is, and the destination ring where
// the jump will land. Drawn on the glass after the world and before the
// text: a framed square pane on the right that is the map, and a dim over
// everything else — the cockpit is still there, but this is WARP MODE, and
// the map has the floor.

struct Map {
    // xy: ship, map units. z: visibility. w: aspect
    a: vec4<f32>,
    // xy: moon. zw: sun
    b: vec4<f32>,
    // xy: destination. z: its ring radius (map units). w: time
    c: vec4<f32>,
    // x: moon orbit radius (map units). yz: pane centre, NDC. w: pane half
    // width, NDC (a square in pixels: half height is w * aspect).
    d: vec4<f32>,
}

// How far the dim reaches outside the pane, and how dark the pane's own
// ground is.
const DIM_ALPHA: f32 = 0.74;
const PANE_ALPHA: f32 = 0.93;

@group(0) @binding(0) var<uniform> map: Map;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(corners[vi], 0.0, 1.0);
    out.ndc = corners[vi];
    return out;
}

fn ring(p: vec2<f32>, c: vec2<f32>, r: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, abs(length(p - c) - r) - w);
}

fn disc(p: vec2<f32>, c: vec2<f32>, r: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, length(p - c) - r);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = map.a.z;
    if (vis < 0.01) {
        discard;
    }
    let aspect = map.a.w;
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);

    // The pane: inside it the map, outside it the dim. Both edges in
    // aspect-corrected units so the frame is the same weight all round.
    let local = (in.ndc - map.d.yz) * vec2<f32>(aspect, 1.0);
    let half = vec2<f32>(map.d.w * aspect);
    let box_d = max(abs(local.x) - half.x, abs(local.y) - half.y);
    let aa_ndc = max(fwidth(local.x), 1e-4) * 1.2;
    let inside = 1.0 - smoothstep(0.0, aa_ndc, box_d);
    // Frame: a thin line on the edge, and brackets at the corners.
    let frame_w = 0.004;
    let edge = 1.0 - smoothstep(0.0, aa_ndc, abs(box_d) - frame_w);
    let corner = step(0.82, min(abs(local.x) / half.x, abs(local.y) / half.y));
    let frame = edge * (0.35 + 0.65 * corner);
    if (inside < 0.001 && frame < 0.001) {
        let dim = DIM_ALPHA * vis;
        return vec4<f32>(vec3<f32>(0.0), dim);
    }

    // Map space: the planet at the origin, 1 unit per decade of distance
    // from it; five decades reach the Sun, and the pane holds them with a
    // little room.
    let p = local / half.y * 5.4;
    let aa = max(fwidth(p.x), 1e-4) * 1.2;

    var glow = 0.0;
    var warn = 0.0;
    var white = 0.0;

    // Decade rings from 10^5 m (a radius and a half out) to the Sun.
    for (var i = 0; i < 5; i += 1) {
        glow += 0.12 * ring(p, vec2<f32>(0.0), f32(i) + 1.0, 0.004, aa);
    }
    // The planet, the Moon's orbit and the Moon, the Sun.
    white += disc(p, vec2<f32>(0.0), 0.06, aa);
    glow += 0.35 * ring(p, vec2<f32>(0.0), map.d.x, 0.003, aa);
    white += disc(p, map.b.xy, 0.035, aa);
    warn += disc(p, map.b.zw, 0.09, aa) * 0.9;
    warn += 0.25 * (1.0 - smoothstep(0.0, 0.35, length(p - map.b.zw)));
    // The ship: a small chevron pulsing.
    let pulse = 0.7 + 0.3 * sin(map.c.w * 4.0);
    glow += 1.2 * disc(p, map.a.xy, 0.03, aa) * pulse;
    glow += 0.4 * ring(p, map.a.xy, 0.07, 0.004, aa);
    // The destination: an amber ring at the arrival radius, and a line to
    // it from the ship.
    warn += 1.0 * ring(p, map.c.xy, max(map.c.z, 0.05), 0.005, aa);
    let a = map.a.xy;
    let b = map.c.xy;
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    let seg = length(p - (a + ab * t));
    let dash = step(0.5, fract(t * 24.0 - map.c.w * 0.5));
    warn += 0.5 * (1.0 - smoothstep(0.0, aa, seg - 0.003)) * dash;

    // Compose: the pane's ground inside, the dim outside, the frame over
    // both, and the map's light on top (premultiplied).
    let colour = cyan * glow + amber * warn + vec3<f32>(1.0) * white;
    let ground = vec3<f32>(0.01, 0.02, 0.04);
    let alpha = mix(DIM_ALPHA, PANE_ALPHA, inside) * vis;
    let lit = (colour * inside + cyan * frame * 0.9) * vis;
    return vec4<f32>(ground * alpha * inside + lit, alpha);
}
