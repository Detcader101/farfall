// ghost.wgsl — the quantum after-image (pass: ghost)
//
// WARP STOP halts the ship in place: all speed and spin taken out of it at
// once. What carries on down the old vector, for a moment, is the image
// the ship left behind — its own geometry, at the attitude it had, sliding
// away ahead along the velocity it no longer has, fading. Ray-marched
// from the same fighter SDF as the cabin and the map dart, in the ship's
// current frame, so it sits exactly where the ship would have been.
//
// The look is liquid: a shell of the field's blue seen edge-on, its rim
// split into a chromatic fringe (red inside, blue outside — the image is
// refracting the sky behind it), caustic light flowing down its length
// like water over glass, a faint white heart. Additive: light only.
// Written as radiance for the post pass's bloom.

struct Ghost {
    // xyz: the head's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the head's forward axis (-Z is the nose). w: time (s)
    fwd: vec4<f32>,
    // xyz: where the image's origin is now, ship frame (m); w: fade 0..1
    at: vec4<f32>,
    // the image's attitude relative to the ship's, a quaternion (xyz, w)
    rot: vec4<f32>,
    // xyz: the image's direction of travel, ship frame; w: strength
    dir: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gh: Ghost;

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

// The image's distance field at a point in the ship's frame.
fn sd_ghost(p: vec3<f32>) -> f32 {
    let q = quat_rotate(quat_conj(gh.rot), p - gh.at.xyz);
    return sd_fighter_exterior(q);
}

fn ghost_normal(p: vec3<f32>) -> vec3<f32> {
    let e = 0.02;
    return normalize(vec3<f32>(
        sd_ghost(p + vec3<f32>(e, 0.0, 0.0)) - sd_ghost(p - vec3<f32>(e, 0.0, 0.0)),
        sd_ghost(p + vec3<f32>(0.0, e, 0.0)) - sd_ghost(p - vec3<f32>(0.0, e, 0.0)),
        sd_ghost(p + vec3<f32>(0.0, 0.0, e)) - sd_ghost(p - vec3<f32>(0.0, 0.0, e)),
    ));
}

const BOUND_R: f32 = 9.5;
const STEPS: u32 = 56u;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let fade = gh.at.w;
    let strength = gh.dir.w;
    if (fade <= 0.001 || strength <= 0.0) {
        discard;
    }
    let aspect = gh.right.w;
    let tan_half = gh.up.w;
    let ray = normalize(gh.fwd.xyz + gh.right.xyz * (in.ndc.x * tan_half * aspect) + gh.up.xyz * (in.ndc.y * tan_half));
    // A sphere about the image bounds the march: rays that miss it are
    // done at once (most of the screen).
    let c = gh.at.xyz;
    let b = dot(ray, c);
    let disc = b * b - (dot(c, c) - BOUND_R * BOUND_R);
    if (disc < 0.0) {
        discard;
    }
    let t_in = max(b - sqrt(disc), 0.0);
    let t_out = b + sqrt(disc);
    if (t_out <= 0.0) {
        discard;
    }
    var t = t_in;
    var hit = false;
    var p = vec3<f32>(0.0);
    for (var i = 0u; i < STEPS; i += 1u) {
        p = ray * t;
        let d = sd_ghost(p);
        if (d < 0.01) {
            hit = true;
            break;
        }
        t += max(d, 0.01);
        if (t > t_out) {
            break;
        }
    }
    if (!hit) {
        discard;
    }
    let n = ghost_normal(p);
    let q = quat_rotate(quat_conj(gh.rot), p - c);
    let now = gh.fwd.w;
    let facing = abs(dot(n, ray));
    // The chromatic rim: three fringes at three angles, red hugging the
    // body, blue standing off the silhouette — the image refracts.
    let rim_r = pow(1.0 - facing, 1.4);
    let rim_g = pow(1.0 - facing, 2.2);
    let rim_b = pow(1.0 - facing, 3.2);
    // Caustics flowing down the image's length: two noise fields sliding
    // aft at different speeds, their product sharpened into ropes of light
    // — water over glass.
    let flow_a = vnoise(vec3<f32>(q.x * 1.1, q.y * 1.6, q.z * 0.9 - now * 7.0));
    let flow_b = vnoise(vec3<f32>(q.x * 2.4 + 5.0, q.y * 2.4, q.z * 1.7 - now * 11.0));
    let caustic = pow(clamp(flow_a * flow_b * 3.2, 0.0, 1.5), 2.6);
    // Bands of light down the length, the field's pulse.
    let bands = 0.5 + 0.5 * sin(q.z * 2.4 - now * 18.0);
    let body = 0.06 + 0.16 * bands + 0.9 * caustic;
    let blue = vec3<f32>(0.25, 0.55, 1.00);
    let white = vec3<f32>(0.90, 0.96, 1.00);
    let fringe = vec3<f32>(rim_r * 0.55, rim_g * 0.95, rim_b * 1.6);
    let colour = (fringe * 1.1 + blue * body * 0.8 + white * rim_g * rim_g * 0.5 + white * caustic * 0.25 * facing)
        * fade * strength * 0.75;
    return vec4<f32>(colour, 1.0);
}
