// pointer.wgsl — the mouse pointer (pass: pointer)
//
// Lane: A (vertex+fragment only). Cost class: trivial (a few pixels; the
// rest of the screen discards on a bound test).
//
// A pointer for the panels: an arrow of the cluster's light with a dark
// edge so it reads over anything, a soft halo, a click's flash. Drawn
// last, screen-fixed, whenever a panel is up.

struct Pointer {
    // xy: the pointer's tip (NDC), z: size (screen height fraction),
    // w: aspect
    a: vec4<f32>,
    // x: shown, y: press 0..1 (a flash that fades), z: time, w: unused
    b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> ptr: Pointer;

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

// A triangle's signed distance (points in order).
fn sd_tri(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, c: vec2<f32>) -> f32 {
    let d = min(min(seg_dist(p, a, b), seg_dist(p, b, c)), seg_dist(p, c, a));
    let s0 = (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let s1 = (c.x - b.x) * (p.y - b.y) - (c.y - b.y) * (p.x - b.x);
    let s2 = (a.x - c.x) * (p.y - c.y) - (a.y - c.y) * (p.x - c.x);
    let inside = (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0);
    return select(d, -d, inside);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (ptr.b.x < 0.5) {
        discard;
    }
    let aspect = ptr.a.w;
    let size = max(ptr.a.z, 0.005);
    // Screen units with the aspect folded in, the tip at the origin.
    let p = (in.ndc - ptr.a.xy) * vec2<f32>(aspect, 1.0) / size;
    if (max(abs(p.x), abs(p.y)) > 2.2) {
        discard;
    }
    // The arrow: a lean head pointing up-left, a short tail.
    let head = sd_tri(p, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, -1.0), vec2<f32>(0.70, -0.70));
    let tail = seg_dist(p, vec2<f32>(0.30, -0.62), vec2<f32>(0.52, -1.05)) - 0.10;
    let d = min(head, tail);
    let aa = max(fwidth(p.x), 1e-3) * 1.2;
    let fill = 1.0 - smoothstep(0.0, aa, d);
    let edge = (1.0 - smoothstep(0.0, aa, abs(d) - 0.07));
    let cyan = vec3<f32>(0.55, 0.95, 1.0);
    let press = ptr.b.y;
    // A halo, swelling on a click.
    let halo = exp(-max(d, 0.0) * (3.0 - 1.6 * press)) * (0.10 + 0.5 * press);
    var rgb = cyan * (0.95 * fill) + vec3<f32>(1.0) * fill * 0.25 + cyan * halo;
    // The dark edge: opaque, so the pointer reads over light and dark.
    let dark = edge * (1.0 - fill);
    rgb = rgb * (1.0 - dark);
    let alpha = max(fill, max(dark, halo * 0.6));
    return vec4<f32>(rgb * alpha, alpha);
}
