// jet.wgsl — the ship seen from outside (pass: jet)
//
// Lane: A (vertex+fragment only). Cost class: bounded march (one sphere
// test ends it for most of the screen, as the ghost's does).
//
// The chase view and the holo3PP both need the one thing the cockpit never
// draws: the ship itself. This is the same fighter SDF the cabin, the map
// dart and the after-image all share (sd_fighter_exterior in the common
// prelude), marched in the ship's own frame from an eye a few lengths
// back, and lit for real — the Sun on hull metal, a dark canopy, the
// nozzles glowing with whatever the engines are doing. Opaque where hit:
// the ship hides the stars behind it.

struct Jet {
    // xyz: the view's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the view's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the view's forward axis (-Z is the nose). w: time (s)
    fwd: vec4<f32>,
    // xyz: the eye in the ship's frame (m). w: exposure
    eye: vec4<f32>,
    // xyz: the Sun's direction in ship frame. w: engine effort 0..1
    sun: vec4<f32>,
    // x: hyper field 0..1, y: draw at all (0 skips), z,w: unused
    glow: vec4<f32>,
}

@group(0) @binding(0) var<uniform> jet: Jet;

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

fn jet_normal(p: vec3<f32>) -> vec3<f32> {
    let e = 0.02;
    return normalize(vec3<f32>(
        sd_fighter_exterior(p + vec3<f32>(e, 0.0, 0.0)) - sd_fighter_exterior(p - vec3<f32>(e, 0.0, 0.0)),
        sd_fighter_exterior(p + vec3<f32>(0.0, e, 0.0)) - sd_fighter_exterior(p - vec3<f32>(0.0, e, 0.0)),
        sd_fighter_exterior(p + vec3<f32>(0.0, 0.0, e)) - sd_fighter_exterior(p - vec3<f32>(0.0, 0.0, e)),
    ));
}

// The whole fighter fits well inside this sphere about its origin.
const BOUND_R: f32 = 10.5;
const STEPS: u32 = 72u;

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (jet.glow.y < 0.5) {
        discard;
    }
    let aspect = jet.right.w;
    let tan_half = jet.up.w;
    let ray = normalize(jet.fwd.xyz
        + jet.right.xyz * (in.ndc.x * tan_half * aspect)
        + jet.up.xyz * (in.ndc.y * tan_half));
    let eye = jet.eye.xyz;
    // A sphere about the ship bounds the march: rays that miss it are done
    // at once (most of the screen).
    let oc = -eye;
    let b = dot(ray, oc);
    let disc = b * b - (dot(oc, oc) - BOUND_R * BOUND_R);
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
        p = eye + ray * t;
        let d = sd_fighter_exterior(p);
        if (d < 0.008) {
            hit = true;
            break;
        }
        t += max(d, 0.008);
        if (t > t_out) {
            break;
        }
    }
    if (!hit) {
        discard;
    }
    let n = jet_normal(p);
    let sun = normalize(jet.sun.xyz);

    // Hull metal: grey-blue on top, darker under the belly, with a hint of
    // panel banding down the fuselage so the surface reads as built.
    let band = 0.94 + 0.06 * sin(p.z * 7.0) * sin(p.x * 5.0 + 1.7);
    var albedo = mix(vec3<f32>(0.30, 0.32, 0.36), vec3<f32>(0.50, 0.54, 0.60),
                     clamp(p.y * 0.5 + 0.8, 0.0, 1.0)) * band;
    // The canopy: the same glass cut the hull pass carves — anything close
    // to it from outside is dark smoked glass, not metal.
    let glass = sd_round_box(p - vec3<f32>(0.0, 0.7, -0.45), vec3<f32>(0.80, 0.9, 1.25), 0.15);
    let canopy = 1.0 - smoothstep(0.0, 0.25, glass);
    albedo = mix(albedo, vec3<f32>(0.04, 0.07, 0.10), canopy);

    let diff = max(dot(n, sun), 0.0);
    let h = normalize(sun - ray);
    let spec = pow(max(dot(n, h), 0.0), 42.0) * mix(0.5, 1.6, canopy);
    // Starlight ambient: cold, faint, so the night side is a silhouette
    // with a readable rim, never a black hole in the sky.
    let rim = pow(1.0 - abs(dot(n, ray)), 3.0);
    var rgb = albedo * (diff * 1.35 + 0.05)
        + vec3<f32>(1.0, 0.97, 0.92) * spec * (0.35 + 0.65 * diff)
        + vec3<f32>(0.10, 0.13, 0.18) * rim;

    // The nozzles: a glow ring at each nacelle's tail, by what the engines
    // are doing — effort a hot amber, the hyper field an unnatural blue.
    let eq = vec3<f32>(abs(p.x) - 0.62, p.y + 0.85, p.z);
    let near_nozzle = (1.0 - smoothstep(0.30, 0.75, length(eq.xy)))
        * smoothstep(6.9, 7.4, p.z);
    let effort = clamp(jet.sun.w, 0.0, 1.0);
    let hyper = clamp(jet.glow.x, 0.0, 1.0);
    let engine = vec3<f32>(1.0, 0.42, 0.13) * effort * 2.4
        + vec3<f32>(0.35, 0.65, 1.0) * hyper * 3.0
        + vec3<f32>(0.30, 0.13, 0.05) * 0.12;
    rgb += engine * near_nozzle;

    let out = radiance(rgb, jet.eye.w);
    // Premultiplied, alpha 1: the ship is solid — stars end at the hull.
    return vec4<f32>(out, 1.0);
}
