// hologram.wgsl — the SHIP bay: the pilot's own ship as a hologram in a
// pane (pass: hologram)
//
// Lane: A (vertex+fragment only). Cost class: bounded march inside the
// pane only (a sphere test ends it for the rest of the pane; the dim
// outside is a flat write).
//
// Screen-fixed like the map: a flat pane, dragged by the cursor, its
// anchor saved. Inside it an orbit camera (the map's pattern — drag turns
// it, or it yaws by itself) looks at the fighter's own exterior SDF —
// the same sd_fighter_exterior the cabin, the map dart and the after-image
// share — drawn as a translucent hologram: rim glow, bands of light
// running nose to tail, a scan plane sweeping the height, scanlines on
// the eye. Each hardpoint is a pip; what is mounted there is drawn on it
// (the twin cannon, the long rail); the card's chosen slot pulses amber.

struct Hologram {
    // xyz: orbit camera eye, model frame (ship m). w: visibility 0..1
    eye: vec4<f32>,
    // camera basis; w of fwd: tan(fov/2)
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // x,y: pane centre (NDC), z: pane half width (NDC), w: aspect
    pane: vec4<f32>,
    // x: hologram hue 0..1, y: saturation 0..1, z: scanline density
    // (lines per pane height), w: chosen hardpoint (-1 none)
    look: vec4<f32>,
    // x: time (s), y: surface height px, zw: unused
    misc: vec4<f32>,
    // xyz: each hardpoint, model frame (ship m); w: 0 empty, 1 cannon,
    // 2 rail
    pts: array<vec4<f32>, 4>,
}

@group(0) @binding(0) var<uniform> holo: Hologram;

const DIM_ALPHA: f32 = 0.74;
const PANE_ALPHA: f32 = 0.93;
const STEPS: u32 = 72u;
// Half-extent bounding the hull and anything mounted, ship m.
const BOUND_MODEL: f32 = 10.5;

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

fn hue_rgb(h: f32, s: f32) -> vec3<f32> {
    let k = fract(vec3<f32>(h, h + 2.0 / 3.0, h + 1.0 / 3.0)) * 6.0;
    let rgb = clamp(abs(k - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
    return mix(vec3<f32>(1.0), rgb, s);
}

// A ring about the z axis: the rail's coils.
fn sd_ring_z(p: vec3<f32>, r: f32, t: f32) -> f32 {
    let q = vec2<f32>(length(p.xy) - r, p.z);
    return length(q) - t;
}

// What sits on a hardpoint, model frame (ship m), by kind.
fn sd_mount(q: vec3<f32>, m: vec3<f32>, kind: f32) -> f32 {
    let p = q - m;
    if (kind > 1.5) {
        // The rail: one long barrel, three coils along it, a breech.
        var d = sd_capsule_ab(p, vec3<f32>(0.0, 0.0, 0.9), vec3<f32>(0.0, 0.0, -2.6), 0.13);
        d = min(d, sd_ring_z(p - vec3<f32>(0.0, 0.0, -0.4), 0.28, 0.05));
        d = min(d, sd_ring_z(p - vec3<f32>(0.0, 0.0, -1.1), 0.28, 0.05));
        d = min(d, sd_ring_z(p - vec3<f32>(0.0, 0.0, -1.8), 0.28, 0.05));
        d = min(d, sd_round_box(p - vec3<f32>(0.0, 0.0, 0.7), vec3<f32>(0.24, 0.22, 0.4), 0.05));
        return d;
    }
    if (kind > 0.5) {
        // The cannon: twin barrels off a breech block.
        let b = vec3<f32>(abs(p.x) - 0.17, p.y, p.z);
        var d = sd_capsule_ab(b, vec3<f32>(0.0, 0.0, 0.3), vec3<f32>(0.0, 0.0, -1.5), 0.08);
        d = min(d, sd_round_box(p - vec3<f32>(0.0, 0.0, 0.45), vec3<f32>(0.4, 0.22, 0.45), 0.05));
        return d;
    }
    return 1e9;
}

// The hull and its mounts, model frame.
fn sd_holo(q: vec3<f32>) -> f32 {
    var d = sd_fighter_exterior(q);
    for (var i = 0u; i < 4u; i += 1u) {
        let hp = holo.pts[i];
        d = min(d, sd_mount(q, hp.xyz, hp.w));
    }
    return d;
}

fn holo_normal(q: vec3<f32>) -> vec3<f32> {
    let e = 0.02;
    return normalize(vec3<f32>(
        sd_holo(q + vec3<f32>(e, 0.0, 0.0)) - sd_holo(q - vec3<f32>(e, 0.0, 0.0)),
        sd_holo(q + vec3<f32>(0.0, e, 0.0)) - sd_holo(q - vec3<f32>(0.0, e, 0.0)),
        sd_holo(q + vec3<f32>(0.0, 0.0, e)) - sd_holo(q - vec3<f32>(0.0, 0.0, e)),
    ));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = holo.eye.w;
    if (vis < 0.01) {
        discard;
    }
    let aspect = holo.pane.w;
    let now = holo.misc.x;
    let cyan = hue_rgb(holo.look.x, holo.look.y);
    let white = vec3<f32>(0.85, 0.96, 1.0);
    let amber = vec3<f32>(1.0, 0.72, 0.28);

    // The pane: inside it the bay, outside it the dim (the map's frame).
    let local = (in.ndc - holo.pane.xy) * vec2<f32>(aspect, 1.0);
    let half = vec2<f32>(holo.pane.z * aspect);
    let box_d = max(abs(local.x) - half.x, abs(local.y) - half.y);
    let aa_ndc = max(fwidth(local.x), 1e-4) * 1.2;
    let inside = 1.0 - smoothstep(0.0, aa_ndc, box_d);
    let frame_w = 0.004;
    let edge = 1.0 - smoothstep(0.0, aa_ndc, abs(box_d) - frame_w);
    let corner = step(0.82, min(abs(local.x) / half.x, abs(local.y) / half.y));
    let frame = edge * (0.35 + 0.65 * corner);
    if (inside < 0.001 && frame < 0.001) {
        return vec4<f32>(vec3<f32>(0.0), DIM_ALPHA * vis);
    }

    // A ray into the bay from the orbiting camera.
    let uv = local / half.y;
    let tan_half = holo.fwd.w;
    let ray = normalize(holo.fwd.xyz + holo.right.xyz * uv.x * tan_half + holo.up.xyz * uv.y * tan_half);
    let eye = holo.eye.xyz;

    // The projector's flicker, and scanlines on the eye: a projection
    // seen, not a thing.
    let flick = 0.94 + 0.06 * sin(now * 41.0 + sin(now * 7.3) * 3.0);
    let scan = 0.86 + 0.14 * sin(local.y / max(half.y, 1e-4) * holo.look.z * 3.14159);

    var colour = vec3<f32>(0.0);
    var t_hit = 1e9;

    // A sphere about the model bounds the march.
    let b = dot(ray, -eye);
    let disc = b * b - (dot(eye, eye) - BOUND_MODEL * BOUND_MODEL);
    if (disc > 0.0) {
        var t = max(b - sqrt(disc), 0.0);
        let t_out = b + sqrt(disc);
        var hit = false;
        var q = vec3<f32>(0.0);
        for (var i = 0u; i < STEPS; i += 1u) {
            q = eye + ray * t;
            let d = sd_holo(q);
            if (d < 0.01) {
                hit = true;
                break;
            }
            t += max(d, 0.01);
            if (t > t_out) {
                break;
            }
        }
        if (hit) {
            t_hit = t;
            let n = holo_normal(q);
            let rim = pow(1.0 - abs(dot(n, ray)), 1.8);
            // Bands of light run nose to tail; a scan plane sweeps the
            // height.
            let bands = 0.5 + 0.5 * sin(q.z * 3.0 - now * 6.0);
            let sweep_y = -2.4 + 4.6 * fract(now * 0.23);
            let sweep = smoothstep(0.35, 0.0, abs(q.y - sweep_y));
            let body = 0.06 + 0.10 * bands + 0.5 * sweep;
            colour += cyan * (rim * 1.6 + body) + white * rim * rim * 0.7;
            // Mounted things read a shade warmer than the hull.
            var on_mount = 0.0;
            for (var i = 0u; i < 4u; i += 1u) {
                let hp = holo.pts[i];
                if (hp.w > 0.5 && sd_mount(q, hp.xyz, hp.w) < 0.05) {
                    on_mount = 1.0;
                }
            }
            colour += white * on_mount * (0.12 + 0.25 * rim);
        }
    }

    // The turntable: rings and slow spokes on a floor under the hull.
    let floor_y = -2.7;
    if (abs(ray.y) > 1e-4) {
        let tf = (floor_y - eye.y) / ray.y;
        if (tf > 0.05 && tf < t_hit) {
            let f = eye + ray * tf;
            let r = length(f.xz);
            if (r < 9.5) {
                let rings = smoothstep(0.10, 0.0, abs(fract(r / 2.4 + 0.5) - 0.5) * 2.4);
                let ring_edge = smoothstep(0.08, 0.0, abs(r - 9.3));
                let ang = atan2(f.z, f.x) + now * 0.15;
                let spokes = smoothstep(0.03, 0.0, abs(fract(ang * 6.0 / 6.2832 + 0.5) - 0.5)) * smoothstep(1.0, 3.0, r);
                let fade = 1.0 - smoothstep(6.0, 9.3, r);
                colour += cyan * ((rings * 0.16 + spokes * 0.06) * fade + ring_edge * 0.25 + 0.012 * fade);
            }
        }
    }

    // The pips: each hardpoint a point of light; the chosen one a pulsing
    // amber ring; an empty one a hollow ring. Dimmer behind the hull.
    let sel = i32(holo.look.w);
    for (var i = 0u; i < 4u; i += 1u) {
        let hp = holo.pts[i];
        let rel = hp.xyz - eye;
        let along = dot(ray, rel);
        if (along <= 0.0) {
            continue;
        }
        // Angular distance, so the pip keeps its size on screen.
        let d = length(rel - ray * along) / along;
        let behind = select(1.0, 0.35, along > t_hit + 0.05);
        let pulse = 0.5 + 0.5 * sin(now * 5.0);
        let chosen = i32(i) == sel;
        let ring_r = select(0.018, 0.028 + 0.006 * pulse, chosen);
        let ring = smoothstep(0.004, 0.0, abs(d - ring_r));
        let dot_g = exp(-d * d / (2.0 * 0.006 * 0.006));
        let halo = exp(-d / 0.035) * 0.10;
        var pc = cyan;
        if (hp.w > 1.5) { pc = vec3<f32>(0.70, 0.55, 1.0); }
        if (hp.w < 0.5) { pc = vec3<f32>(0.45, 0.60, 0.70); }
        let filled = select(0.0, 1.0, hp.w > 0.5);
        var pip = pc * (dot_g * (0.6 + 0.9 * filled) + ring * (0.5 + 0.5 * (1.0 - filled)) + halo);
        if (chosen) {
            pip += amber * (ring * (0.9 + 0.6 * pulse) + dot_g * 0.8 + halo * 2.0);
        }
        colour += pip * behind;
    }

    let ground = vec3<f32>(0.01, 0.02, 0.04);
    let alpha = mix(DIM_ALPHA, PANE_ALPHA, inside) * vis;
    let lit = (colour * flick * scan * inside + cyan * frame * 0.9) * vis;
    return vec4<f32>(ground * alpha * inside + lit, alpha);
}
