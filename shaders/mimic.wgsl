// mimic.wgsl — ships out of the rocks (pass: mimic)
//
// A mimic sits in the ring inside a holographic shroud shaped like a
// stone. Struck, the shroud goes: first the ship glows through the rock as
// a cyan hologram (bands of light running its length, a rim), then the
// rock projection winks off (the belt stops drawing it) and the hologram
// hardens into a sun-lit hull. Then it is a ship: engines by its effort
// (amber for a hostile, a cool blue for one that hails, with a white
// beacon pulsing at the nose while it talks), and as a wreck it is dark,
// tumbling, with embers guttering in the damage. Ray-marched from the
// same fighter SDF as everything else, at each ship's own pose in the
// ship's frame; hidden behind nearer rocks.

const MIMICS: u32 = 4u;
const LIVE: u32 = 48u;

struct Mimic {
    // xyz: the head's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the head's forward axis (-Z is the nose). w: time (s)
    fwd: vec4<f32>,
    // xyz: the Sun's direction, ship frame; w: ships in use
    sun: vec4<f32>,
    // x exposure, y rocks in use
    look: vec4<f32>,
    // xyz: each ship's origin, ship frame (m); w: reveal 0..1
    at: array<vec4<f32>, MIMICS>,
    // each ship's attitude relative to ours, a quaternion (xyz, w)
    rot: array<vec4<f32>, MIMICS>,
    // effort, kind (0 hail, 1 hostile, 2 wreck), wound, seed
    info: array<vec4<f32>, MIMICS>,
    // the rocks: xyz centre, w radius
    rocks: array<vec4<f32>, LIVE>,
}

@group(0) @binding(0) var<uniform> mm: Mimic;

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

fn quat_conj(q: vec4<f32>) -> vec4<f32> {
    return vec4<f32>(-q.xyz, q.w);
}

fn to_local(i: u32, p: vec3<f32>) -> vec3<f32> {
    return quat_rotate(quat_conj(mm.rot[i]), p - mm.at[i].xyz);
}

fn sd_mimic(i: u32, p: vec3<f32>) -> f32 {
    return sd_fighter_exterior(to_local(i, p));
}

fn mimic_normal(i: u32, p: vec3<f32>) -> vec3<f32> {
    let e = 0.02;
    return normalize(vec3<f32>(
        sd_mimic(i, p + vec3<f32>(e, 0.0, 0.0)) - sd_mimic(i, p - vec3<f32>(e, 0.0, 0.0)),
        sd_mimic(i, p + vec3<f32>(0.0, e, 0.0)) - sd_mimic(i, p - vec3<f32>(0.0, e, 0.0)),
        sd_mimic(i, p + vec3<f32>(0.0, 0.0, e)) - sd_mimic(i, p - vec3<f32>(0.0, 0.0, e)),
    ));
}

// The nearest rock along the ray, or a very long way.
fn rock_t(ray: vec3<f32>) -> f32 {
    var best = 1.0e12;
    let n = u32(mm.look.y);
    for (var i = 0u; i < n; i += 1u) {
        let c = mm.rocks[i].xyz;
        let r = mm.rocks[i].w;
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - r * r);
        if (disc > 0.0) {
            let t = b - sqrt(disc);
            if (t > 0.0 && t < best) {
                best = t;
            }
        }
    }
    return best;
}

const BOUND_R: f32 = 9.5;
const STEPS: u32 = 56u;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n_ships = u32(mm.sun.w);
    if (n_ships == 0u) {
        discard;
    }
    let aspect = mm.right.w;
    let tan_half = mm.up.w;
    let ray = normalize(mm.fwd.xyz
        + mm.right.xyz * (in.ndc.x * tan_half * aspect)
        + mm.up.xyz * (in.ndc.y * tan_half));
    // The nearest ship this ray meets.
    var best_t = 1.0e12;
    var best_i = 0u;
    var best_p = vec3<f32>(0.0);
    var hit = false;
    for (var i = 0u; i < n_ships; i += 1u) {
        let c = mm.at[i].xyz;
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - BOUND_R * BOUND_R);
        if (disc < 0.0) {
            continue;
        }
        let t_in = max(b - sqrt(disc), 0.0);
        let t_out = b + sqrt(disc);
        if (t_out <= 0.0 || t_in > best_t) {
            continue;
        }
        var t = t_in;
        // Far away a ship is a few pixels: a coarser stop keeps the march
        // from running out of steps on its silhouette.
        let eps = max(0.008, t_in * tan_half * 0.0006);
        for (var k = 0u; k < STEPS; k += 1u) {
            let p = ray * t;
            let d = sd_mimic(i, p);
            if (d < eps) {
                if (t < best_t) {
                    best_t = t;
                    best_i = i;
                    best_p = p;
                    hit = true;
                }
                break;
            }
            t += max(d, eps);
            if (t > t_out) {
                break;
            }
        }
    }
    if (!hit) {
        discard;
    }
    if (rock_t(ray) < best_t) {
        discard;
    }
    let i = best_i;
    let p = best_p;
    let n = mimic_normal(i, p);
    let q = to_local(i, p);
    let now = mm.fwd.w;
    let reveal = mm.at[i].w;
    let effort = mm.info[i].x;
    let kind = mm.info[i].y;
    let wound = mm.info[i].z;
    let seed = mm.info[i].w;
    let sun = normalize(mm.sun.xyz);
    let wreck = kind > 1.5;
    let hostile = kind > 0.5 && !wreck;

    // The hologram: a cyan field seen edge-on, bands running nose to
    // tail, a scan plane, flickering like a projector losing power.
    let rim = pow(1.0 - abs(dot(n, ray)), 2.2);
    let bands = 0.5 + 0.5 * sin(q.z * 2.6 - now * 14.0);
    let bands2 = 0.5 + 0.5 * sin(q.z * 7.0 + q.x * 3.0 + now * 23.0);
    let sweep = exp(-abs(q.y - (-2.0 + 4.0 * fract(now * 0.7 + seed))) * 3.0);
    let flick = 0.8 + 0.2 * sin(now * 47.0 + seed * 20.0) * sin(now * 9.0);
    let cyan = vec3<f32>(0.12, 0.72, 1.0);
    let white = vec3<f32>(0.75, 0.95, 1.0);
    let holo = (cyan * (rim * 2.2 + 0.16 + 0.35 * bands * bands2 + sweep * 0.8)
        + white * rim * rim * 0.5) * flick;

    // The hull: the same metal as our own, sun-lit; a wreck's is scorched.
    let band = 0.94 + 0.06 * sin(q.z * 7.0) * sin(q.x * 5.0 + 1.7);
    var albedo = mix(vec3<f32>(0.26, 0.27, 0.30), vec3<f32>(0.46, 0.48, 0.52),
                     clamp(q.y * 0.5 + 0.8, 0.0, 1.0)) * band;
    if (hostile) {
        // A darker ship with a rust-red belly stripe: not one of ours.
        albedo = mix(albedo, vec3<f32>(0.42, 0.14, 0.10),
                     smoothstep(0.35, 0.15, abs(q.x - 0.0) + max(q.y, 0.0) * 2.0) * 0.8);
        albedo *= 0.8;
    }
    let glass = sd_round_box(q - vec3<f32>(0.0, 0.7, -0.45), vec3<f32>(0.80, 0.9, 1.25), 0.15);
    let canopy = 1.0 - smoothstep(0.0, 0.25, glass);
    albedo = mix(albedo, vec3<f32>(0.04, 0.07, 0.10), canopy);
    // Damage: scorched patches spreading with the wound.
    let scorch = smoothstep(0.55, 0.0, vnoise(q * 1.6 + seed * 10.0) - wound * 0.9 + 0.35);
    albedo = mix(albedo, vec3<f32>(0.05, 0.04, 0.04), scorch * (0.5 + 0.5 * wound));
    let diff = max(dot(n, sun), 0.0);
    let h = normalize(sun - ray);
    let spec = pow(max(dot(n, h), 0.0), 42.0) * mix(0.5, 1.6, canopy) * (1.0 - scorch);
    let rim3 = pow(1.0 - abs(dot(n, ray)), 3.0);
    var hull = albedo * (diff * 1.35 + 0.05)
        + vec3<f32>(1.0, 0.97, 0.92) * spec * (0.35 + 0.65 * diff)
        + vec3<f32>(0.10, 0.13, 0.18) * rim3;

    // Engines at the nacelles' tails: amber for a hostile, blue for a
    // hailer; a wreck's are out, and embers gutter in its scorch.
    let eq = vec3<f32>(abs(q.x) - 0.62, q.y + 0.85, q.z);
    let near_nozzle = (1.0 - smoothstep(0.30, 0.75, length(eq.xy))) * smoothstep(6.9, 7.4, q.z);
    let engine_col = select(vec3<f32>(0.35, 0.65, 1.0), vec3<f32>(1.0, 0.42, 0.13), hostile);
    hull += engine_col * (effort * 2.4 + 0.12) * near_nozzle * f32(!wreck);
    if (wreck) {
        // Embers: a few patches in the scorch, each breathing on its own.
        let ember = smoothstep(0.62, 0.92, vnoise(q * 2.2 + vec3<f32>(seed * 9.0, 0.0, 0.0)));
        let breathe = 0.45 + 0.55 * sin(now * 6.0 + vnoise(q * 0.8) * 20.0);
        hull += vec3<f32>(1.0, 0.30, 0.06) * scorch * ember * breathe * 1.2;
    }
    // A beacon at the nose while it hails.
    if (kind < 0.5) {
        let nose = length(q - vec3<f32>(0.0, 0.0, -7.2));
        let pulse = pow(0.5 + 0.5 * sin(now * 4.0), 6.0);
        hull += white * exp(-nose * 1.2) * pulse * 3.0;
    }

    // The reveal: hologram through the shroud, then the hull hardening
    // under it, the last of the field flickering off its rim.
    let solid = smoothstep(0.42, 1.0, reveal);
    let field = (1.0 - solid) * (0.4 + 0.6 * smoothstep(0.0, 0.3, reveal));
    let rgb = hull * solid + holo * field;
    let out = tonemap(rgb, mm.look.x);
    // Premultiplied: the hull is solid; the field is light and lets the
    // rock behind show through.
    let alpha = max(solid, 0.0);
    return vec4<f32>(out, alpha);
}
