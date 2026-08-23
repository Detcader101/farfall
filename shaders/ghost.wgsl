// ghost.wgsl — the quantum after-image (pass: ghost)
//
// WARP STOP halts the ship in place: all speed and spin taken out of it at
// once. What carries on down the old vector, for a moment, is the image
// the ship left behind — its own geometry, at the attitude it had, sliding
// away ahead along the velocity it no longer has, in the field's blue,
// fading. Ray-marched from the same fighter SDF as the cabin and the map
// dart, in the ship's current frame, so it sits exactly where the ship
// would have been.

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
    // The quantum look: a blue field seen edge-on — a rim that glows,
    // faces that barely show — with bands of light flowing down the
    // image's own length and a faint white core. The whole thing thins
    // with the fade.
    let rim = pow(1.0 - abs(dot(n, ray)), 2.2);
    let q = quat_rotate(quat_conj(gh.rot), p - c);
    let now = gh.fwd.w;
    let bands = 0.5 + 0.5 * sin(q.z * 2.4 - now * 18.0);
    let bands2 = 0.5 + 0.5 * sin(q.z * 7.0 + q.x * 3.0 + now * 31.0);
    let body = 0.10 + 0.22 * bands * bands2;
    let blue = vec3<f32>(0.30, 0.62, 1.0);
    let white = vec3<f32>(0.85, 0.95, 1.0);
    let colour = (blue * (rim * 1.8 + body) + white * rim * rim * 0.8) * fade * strength;
    return vec4<f32>(colour, 1.0);
}
