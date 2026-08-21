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
// text, as a dark pane the pilot reads through.

struct Map {
    // xy: ship, map units. z: visibility. w: aspect
    a: vec4<f32>,
    // xy: moon. zw: sun
    b: vec4<f32>,
    // xy: destination. z: its ring radius (map units). w: time
    c: vec4<f32>,
    // x: moon orbit radius (map units). y: sun distance (map units).
    // z: destination index (0 planet, 1 moon, 2 sun). w: unused
    d: vec4<f32>,
}

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
    // Map space: the planet at the origin, 1 unit per decade of distance
    // from it, the view centred a little low to leave the text room.
    let p = (in.ndc * vec2<f32>(aspect, 1.0) - vec2<f32>(0.0, -0.15)) * 3.2;
    let aa = max(fwidth(p.x), 1e-4) * 1.2;
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);

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

    // The pane.
    let colour = cyan * glow + amber * warn + vec3<f32>(1.0) * white;
    let pane = vec3<f32>(0.01, 0.02, 0.04);
    let alpha = 0.72 * vis;
    let lit = colour * vis;
    return vec4<f32>(pane * alpha + lit, alpha + min(dot(lit, vec3<f32>(1.0)), 1.0) * 0.0);
}
