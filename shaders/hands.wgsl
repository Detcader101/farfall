// hands.wgsl — VR controller glyphs (pass: hands, SPEC §5.3b)
//
// A small SDF raymarch of each tracked hand's grip pose: a capsule shaft
// (the grip) and a flattened ring (the Index's tracking puck), lit as an
// emissive line-glow silhouette — the same TRON/JET vocabulary as the
// cabin's own dials and the ghost's after-image, not a photoreal
// controller model. A trigger-reactive dot brightens with the trigger
// axis; the whole glyph brightens and tightens while held (grabbing the
// stick or throttle, SPEC §5.3b(d)). Additive: light only, drawn in the
// ship pass right after the cabin.

struct Hands {
    // xyz: the head's right axis, ship frame. w: aspect.
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2).
    up: vec4<f32>,
    // xyz: the head's forward axis (-Z the nose). w: time (s).
    fwd: vec4<f32>,
    // xyz: left grip position, ship frame (eye-shifted). w: shown (1/0).
    left_pos: vec4<f32>,
    // Left grip orientation, a quaternion (xyz, w).
    left_rot: vec4<f32>,
    // x: trigger 0..1. y: squeeze 0..1. z: held (1/0). w: occlusion fade.
    left_state: vec4<f32>,
    right_pos: vec4<f32>,
    right_rot: vec4<f32>,
    right_state: vec4<f32>,
    // VR BEAM: the laser's origin (xyz, ship frame, eye-shifted) and
    // whether it is shown at all (w).
    beam_a: vec4<f32>,
    // The laser's hit point (xyz); w unused.
    beam_b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> hd: Hands;

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

// One hand's glyph, in that hand's own local frame (grip at origin,
// +Y along the grip, +Z toward the fingers roughly): a shaft plus a
// flattened ring, and separately the trigger dot's own distance so it
// can be picked out for colour once the body is hit.
fn sd_hand_body(p: vec3<f32>) -> f32 {
    let shaft = sd_capsule_ab(p, vec3<f32>(0.0, -0.09, 0.02), vec3<f32>(0.0, 0.07, -0.02), 0.023);
    let ring = sd_ellipsoid_c(p, vec3<f32>(0.0, 0.05, 0.05), vec3<f32>(0.065, 0.065, 0.02));
    return min(shaft, ring);
}

fn sd_hand_dot(p: vec3<f32>, squeeze: f32) -> f32 {
    return length(p - vec3<f32>(0.0, -0.02, -0.05)) - (0.010 + squeeze * 0.008);
}

// Both hands' combined field at a point in the ship's (eye-shifted)
// frame: returns the nearer distance and which hand/part it belongs to
// so the fragment shader can shade it — x: distance, y: hand (0 left, 1
// right, -1 none), z: part (0 body, 1 dot, 2 beam).
fn sd_hands(p: vec3<f32>) -> vec3<f32> {
    var best = vec3<f32>(1e9, -1.0, 0.0);
    if (hd.left_pos.w > 0.5) {
        let q = quat_rotate(quat_conj(hd.left_rot.xyzw), p - hd.left_pos.xyz);
        let body = sd_hand_body(q);
        let dot_d = sd_hand_dot(q, hd.left_state.y);
        let d = min(body, dot_d);
        if (d < best.x) {
            best = vec3<f32>(d, 0.0, select(0.0, 1.0, dot_d < body));
        }
    }
    if (hd.right_pos.w > 0.5) {
        let q = quat_rotate(quat_conj(hd.right_rot.xyzw), p - hd.right_pos.xyz);
        let body = sd_hand_body(q);
        let dot_d = sd_hand_dot(q, hd.right_state.y);
        let d = min(body, dot_d);
        if (d < best.x) {
            best = vec3<f32>(d, 1.0, select(0.0, 1.0, dot_d < body));
        }
    }
    // VR BEAM: a thin capsule from the hand to the hit point — right
    // hand's own part code (2), so the fragment shader can pick a
    // colour independent of whichever hand is nearer at that point.
    if (hd.beam_a.w > 0.5) {
        let beam_d = sd_capsule_ab(p, hd.beam_a.xyz, hd.beam_b.xyz, 0.0025);
        if (beam_d < best.x) {
            best = vec3<f32>(beam_d, 1.0, 2.0);
        }
    }
    return best;
}

fn hands_normal(p: vec3<f32>) -> vec3<f32> {
    let e = 0.004;
    return normalize(vec3<f32>(
        sd_hands(p + vec3<f32>(e, 0.0, 0.0)).x - sd_hands(p - vec3<f32>(e, 0.0, 0.0)).x,
        sd_hands(p + vec3<f32>(0.0, e, 0.0)).x - sd_hands(p - vec3<f32>(0.0, e, 0.0)).x,
        sd_hands(p + vec3<f32>(0.0, 0.0, e)).x - sd_hands(p - vec3<f32>(0.0, 0.0, e)).x,
    ));
}

const STEPS: u32 = 48u;
const MAX_T: f32 = 4.0;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (hd.left_pos.w < 0.5 && hd.right_pos.w < 0.5 && hd.beam_a.w < 0.5) {
        discard;
    }
    let aspect = hd.right.w;
    let tan_half = hd.up.w;
    let ray = normalize(hd.fwd.xyz + hd.right.xyz * (in.ndc.x * tan_half * aspect) + hd.up.xyz * (in.ndc.y * tan_half));
    var t = 0.05;
    var hit = false;
    var info = vec3<f32>(1e9, -1.0, 0.0);
    var p = vec3<f32>(0.0);
    for (var i = 0u; i < STEPS; i += 1u) {
        p = ray * t;
        info = sd_hands(p);
        if (info.x < 0.0015) {
            hit = true;
            break;
        }
        t += max(info.x, 0.001);
        if (t > MAX_T) {
            break;
        }
    }
    if (!hit) {
        discard;
    }
    // The beam is its own part (2): a steady cyan-white line, no
    // occlusion fade or hand state to read (it is not "a hand").
    if (info.z > 1.5) {
        let beam_colour = vec3<f32>(0.55, 0.85, 1.0) * 1.6;
        return vec4<f32>(beam_colour, 1.0);
    }
    let hand = info.y;
    let fade = select(hd.right_state.w, hd.left_state.w, hand < 0.5);
    if (fade <= 0.002) {
        discard;
    }
    let held = select(hd.right_state.z, hd.left_state.z, hand < 0.5);
    let trigger = select(hd.right_state.x, hd.left_state.x, hand < 0.5);
    let squeeze = select(hd.right_state.y, hd.left_state.y, hand < 0.5);
    let n = hands_normal(p);
    let facing = clamp(dot(n, -ray), 0.0, 1.0);
    let rim = pow(1.0 - facing, 2.4);
    // Cool metal body, brighter and a touch warmer while held (SPEC
    // §5.3b(d) — the pilot's own grip on the stick or throttle).
    let base = mix(vec3<f32>(0.16, 0.22, 0.30), vec3<f32>(0.30, 0.30, 0.20), held);
    let body_light = base * (0.35 + 0.65 * facing) + vec3<f32>(0.35, 0.55, 0.85) * rim * 0.6;
    // The trigger dot: an amber glow that brightens with the pull.
    let dot_colour = vec3<f32>(1.0, 0.55, 0.15) * (0.25 + trigger * 1.4);
    let colour = select(body_light, dot_colour, info.z > 0.5) * fade * (0.7 + 0.3 * squeeze);
    return vec4<f32>(colour, 1.0);
}
