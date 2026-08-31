// jet.wgsl — the ship seen from outside (pass: jet)
//
// Lane: A (vertex+fragment only). Cost class: bounded march (one sphere
// test ends it for most of the screen, as the ghost's does) plus a
// closed-form plume — two segments' closest approach, no march.
//
// The chase view and the holo3PP both need the one thing the cockpit never
// draws: the ship itself. This is the same fighter SDF the cabin, the map
// dart and the after-image all share (sd_fighter_exterior in the common
// prelude), marched in the ship's own frame from an eye a few lengths
// back, and lit for real — the Sun on hull metal, the planet's light on
// the belly, a dark canopy, panel lines, the nozzles' lips hot amber.
// Opaque where hit: the ship hides the stars behind it.
//
// The engines: out of each nozzle a plume of plasma — a white-hot core
// with shock diamonds down its first length, inside a translucent
// blue-violet skin rippling as it streams aft, longer and brighter with
// the effort and pulled to cyan by the hyper field. And the RCS: small
// bright balls of gas at the nose and the wingtips, lit by the demand on
// each axis. Both are light gathered along the ray by closest approach,
// written with alpha 0 so the pane's premultiplied blend adds them; the
// hull hides what is behind it.

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
    // xyz: pitch / yaw / roll demands -1..1
    rcs: vec4<f32>,
    // xyz: the nearest body's direction, ship frame; w: its fill 0..1
    fill: vec4<f32>,
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
// Where the plumes leave the nacelles (ship frame), and their axis.
const NOZZLE_Y: f32 = -0.85;
const NOZZLE_Z: f32 = 7.6;

// The point on the ray nearest the segment a..b, as (u along the segment
// 0..1, t along the ray, distance).
fn ray_segment(eye: vec3<f32>, ray: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let ao = a - eye;
    let e = b - a;
    let ac = ao - ray * dot(ao, ray);
    let bc = e - ray * dot(e, ray);
    let u = clamp(-dot(ac, bc) / max(dot(bc, bc), 1e-8), 0.0, 1.0);
    let q = ao + e * u;
    let t = max(dot(q, ray), 0.0);
    let d = length(q - ray * t);
    return vec3<f32>(u, t, d);
}

// One engine's plume: radiance along this ray, and the depth it sits at
// (so the hull can hide it). `x`: which nacelle.
fn plume(eye: vec3<f32>, ray: vec3<f32>, x: f32, effort: f32, hyper: f32, now: f32) -> vec4<f32> {
    let drive = max(effort, hyper * 0.55);
    if (drive < 0.01) {
        return vec4<f32>(0.0);
    }
    let len = 2.0 + 9.0 * effort + 6.0 * hyper;
    let a = vec3<f32>(x, NOZZLE_Y, NOZZLE_Z);
    let b = a + vec3<f32>(0.0, 0.0, len);
    let s = ray_segment(eye, ray, a, b);
    let u = s.x;
    let t = s.y;
    let d = s.z;
    // The plasma widens as it streams aft and thins with distance from
    // the nozzle: a cone of skin round a needle of core.
    let r_skin = 0.32 + 0.6 * u;
    let r_core = 0.10 + 0.08 * u;
    if (d > r_skin * 2.2) {
        return vec4<f32>(0.0, 0.0, 0.0, t);
    }
    let along = u * len;
    let tail = pow(1.0 - u, 1.3);
    // Ripples on the skin: noise streaming aft fast, fine across.
    let p = eye + ray * t - a;
    let ang = atan2(p.y - 0.0, p.x - 0.0);
    let rip = vnoise(vec3<f32>(ang * 1.6, along * 2.6 - now * 34.0, x * 3.0 + now * 2.0));
    let rip2 = vnoise(vec3<f32>(ang * 3.1 + 7.0, along * 5.0 - now * 61.0, x * 5.0));
    let ripple = 0.55 + 0.6 * rip + 0.25 * rip2;
    let dd = d / r_skin;
    // The skin: a translucent shell, brightest at its edge (seen through
    // more gas there), rippling.
    let shell = exp(-dd * dd * 2.2) * (0.45 + 0.7 * smoothstep(0.35, 0.95, dd) * (1.0 - smoothstep(0.95, 1.4, dd)));
    let skin = shell * ripple * tail;
    // The core: white-hot, and shock diamonds — standing nodes of
    // brightness down the first half of the plume.
    let dc = d / r_core;
    let core = exp(-dc * dc * 1.6) * tail;
    let diamonds = pow(0.5 + 0.5 * cos(along * 4.2 - 0.6), 6.0) * (1.0 - smoothstep(0.1, 0.65, u)) * exp(-dc * dc * 0.9);
    // Colour: blue-white at the core, blue-violet in the skin's tail;
    // the hyper field pulls it all to cyan.
    let blue = mix(vec3<f32>(0.30, 0.50, 1.00), vec3<f32>(0.20, 0.85, 1.00), hyper);
    let violet = mix(vec3<f32>(0.55, 0.35, 1.00), vec3<f32>(0.35, 0.70, 1.00), hyper);
    let white = vec3<f32>(0.95, 0.97, 1.00);
    let skin_col = mix(blue, violet, u);
    var rgb = skin_col * skin * (0.5 + 0.8 * drive)
        + mix(white, blue, 0.35) * core * (1.2 + 1.5 * drive)
        + white * diamonds * 2.2 * drive;
    // The nozzle's mouth: a hot disc of light where the plume is born.
    let mouth = exp(-(along * along) / 0.35) * exp(-dd * dd * 1.2) * drive;
    rgb += mix(vec3<f32>(1.0, 0.75, 0.45), white, 0.6) * mouth * 1.8;
    return vec4<f32>(rgb, t);
}

// The RCS: a small bright ball of gas at each thruster, by its demand.
fn rcs_light(eye: vec3<f32>, ray: vec3<f32>, now: f32) -> vec4<f32> {
    let pitch = jet.rcs.x;
    let yaw = jet.rcs.y;
    let roll = jet.rcs.z;
    let puffs = array<vec4<f32>, 6>(
        vec4<f32>(0.0, -0.55, -5.6, max(pitch, 0.0)),
        vec4<f32>(0.0, -1.35, -5.6, max(-pitch, 0.0)),
        vec4<f32>(-0.55, -0.95, -5.4, max(yaw, 0.0)),
        vec4<f32>(0.55, -0.95, -5.4, max(-yaw, 0.0)),
        vec4<f32>(-5.6, -0.75, 4.5, max(-roll, 0.0)),
        vec4<f32>(5.6, -0.75, 4.5, max(roll, 0.0)),
    );
    var light = vec3<f32>(0.0);
    var depth = 1e9;
    for (var i = 0; i < 6; i += 1) {
        let pf = puffs[i];
        if (pf.w < 0.02) { continue; }
        let rel = pf.xyz - eye;
        let t = max(dot(rel, ray), 0.0);
        let d = length(rel - ray * t);
        // A ball with a flicker: the valve chattering.
        let flick = 0.75 + 0.25 * sin(now * 47.0 + f32(i) * 2.1) * sin(now * 31.0);
        let ball = exp(-(d * d) / 0.06) * 2.2 + exp(-d / 0.45) * 0.5;
        light += vec3<f32>(0.80, 0.90, 1.00) * ball * pf.w * flick;
        depth = min(depth, t);
    }
    return vec4<f32>(light, depth);
}

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
    let now = jet.fwd.w;
    let effort = clamp(jet.sun.w, 0.0, 1.0);
    let hyper = clamp(jet.glow.x, 0.0, 1.0);

    // The light in the air about the ship: the plumes and the RCS, with
    // the depth each sits at.
    let pl = plume(eye, ray, -0.62, effort, hyper, now);
    let pr = plume(eye, ray, 0.62, effort, hyper, now);
    let rc = rcs_light(eye, ray, now);
    var light = pl.xyz + pr.xyz + rc.xyz;
    var light_t = 1e9;
    if (dot(pl.xyz, pl.xyz) > 1e-8) { light_t = min(light_t, pl.w); }
    if (dot(pr.xyz, pr.xyz) > 1e-8) { light_t = min(light_t, pr.w); }
    if (dot(rc.xyz, rc.xyz) > 1e-8) { light_t = min(light_t, rc.w); }

    // A sphere about the ship bounds the march: rays that miss it are done
    // at once (most of the screen).
    let oc = -eye;
    let b = dot(ray, oc);
    let disc = b * b - (dot(oc, oc) - BOUND_R * BOUND_R);
    var hit = false;
    var p = vec3<f32>(0.0);
    var t_hit = 1e9;
    if (disc >= 0.0) {
        let t_in = max(b - sqrt(disc), 0.0);
        let t_out = b + sqrt(disc);
        if (t_out > 0.0) {
            var t = t_in;
            for (var i = 0u; i < STEPS; i += 1u) {
                p = eye + ray * t;
                let d = sd_fighter_exterior(p);
                if (d < 0.008) {
                    hit = true;
                    t_hit = t;
                    break;
                }
                t += max(d, 0.008);
                if (t > t_out) {
                    break;
                }
            }
        }
    }
    if (!hit) {
        if (dot(light, light) < 1e-7) {
            discard;
        }
        // Light alone: alpha 0, the blend adds it.
        return vec4<f32>(tonemap(light, jet.eye.w), 0.0);
    }
    let n = jet_normal(p);
    let sun = normalize(jet.sun.xyz);

    // Hull metal: grey-blue on top, darker under the belly, with panel
    // lines down the fuselage and across the wings so the surface reads
    // as built, and a faint band of tone between them.
    let band = 0.94 + 0.06 * sin(p.z * 7.0) * sin(p.x * 5.0 + 1.7);
    let seam_z = abs(fract(p.z * 0.9 + 0.3) - 0.5) * 2.0;
    let seam_x = abs(fract(abs(p.x) * 0.8 + 0.1) - 0.5) * 2.0;
    let seams = (1.0 - smoothstep(0.955, 0.99, seam_z)) * 0.35
        + (1.0 - smoothstep(0.965, 0.995, seam_x)) * 0.25;
    var albedo = mix(vec3<f32>(0.30, 0.32, 0.36), vec3<f32>(0.52, 0.56, 0.62),
                     clamp(p.y * 0.5 + 0.8, 0.0, 1.0)) * band * (1.0 - seams);
    // The canopy: the same glass cut the hull pass carves — anything close
    // to it from outside is dark smoked glass, not metal.
    let glass = sd_round_box(p - vec3<f32>(0.0, 0.7, -0.45), vec3<f32>(0.80, 0.9, 1.25), 0.15);
    let canopy = 1.0 - smoothstep(0.0, 0.25, glass);
    albedo = mix(albedo, vec3<f32>(0.04, 0.07, 0.10), canopy);

    let diff = max(dot(n, sun), 0.0);
    let h = normalize(sun - ray);
    let gloss = mix(42.0, 160.0, canopy);
    let spec = pow(max(dot(n, h), 0.0), gloss) * mix(0.55, 2.2, canopy);
    // Fresnel: metal and glass both brighten toward a grazing view.
    let fres = pow(1.0 - max(dot(n, -ray), 0.0), 4.0);
    // The nearest body's light on whatever faces it (the planet under
    // the belly), and starlight ambient: cold, faint, so the night side
    // is a silhouette with a readable rim, never a black hole in the sky.
    let body = max(dot(n, normalize(jet.fill.xyz + vec3<f32>(1e-6))), 0.0) * jet.fill.w;
    let rim = pow(1.0 - abs(dot(n, ray)), 3.0);
    var rgb = albedo * (diff * 1.45 + 0.05 + body * 0.55 * vec3<f32>(0.75, 0.85, 1.0))
        + vec3<f32>(1.0, 0.97, 0.92) * spec * (0.35 + 0.65 * diff)
        + vec3<f32>(0.55, 0.62, 0.72) * fres * (0.10 + 0.5 * diff + 0.25 * body)
        + vec3<f32>(0.10, 0.13, 0.18) * rim;

    // The nozzles: the lip of each glows hot amber with the effort, and
    // the plume's own light washes the nacelle's tail blue.
    let eq = vec3<f32>(abs(p.x) - 0.62, p.y + 0.85, p.z);
    let lip = (1.0 - smoothstep(0.30, 0.75, length(eq.xy))) * smoothstep(7.0, 7.45, p.z);
    let tail_wash = (1.0 - smoothstep(0.4, 1.6, length(eq.xy))) * smoothstep(5.5, 7.5, p.z);
    let drive = max(effort, hyper * 0.55);
    let engine = vec3<f32>(1.0, 0.42, 0.13) * (effort * 2.4 + 0.10)
        + mix(vec3<f32>(0.35, 0.55, 1.0), vec3<f32>(0.25, 0.85, 1.0), hyper) * hyper * 3.0;
    rgb += engine * lip;
    rgb += vec3<f32>(0.35, 0.55, 1.0) * tail_wash * drive * 0.6;

    // Light in front of the hull adds over it; behind, the hull hides it.
    if (light_t < t_hit) {
        rgb += light;
    }
    let out = tonemap(rgb, jet.eye.w);
    // Premultiplied, alpha 1: the ship is solid — stars end at the hull.
    return vec4<f32>(out, 1.0);
}
