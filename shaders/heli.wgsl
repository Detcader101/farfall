// heli.wgsl — the helicopters (pass: heli)
//
// Lane: world (HDR). Cost class: bounded march — up to 4 hulls, each
// behind a bounding-sphere test first, so an empty sky ray pays four
// sphere tests and nothing else.
//
// A generic cold-war utility helicopter of our own: a rounded cabin pod,
// a tail boom with a fin and tailplane, two landing skids on struts, a
// mast and two rotors. The rotors are analytic discs, not marched: a
// translucent blur ring with blade streaks swinging round at the rotor's
// speed. Parked on a pad it sits over a painted circle-and-H; the planet
// itself occludes hulls beyond the horizon.

const HELIS: u32 = 4u;
const BOUND_R: f32 = 8.6;
const STEPS: u32 = 48u;

struct Heli {
    // xyz: the head's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the head's forward axis (-Z is the nose). w: time (s)
    fwd: vec4<f32>,
    // xyz: the Sun's direction, ship frame; w: helis in use
    sun: vec4<f32>,
    // x exposure
    look: vec4<f32>,
    // the occluding body underfoot: xyz centre (ship frame), w radius
    occ: vec4<f32>,
    // xyz: each heli's origin, ship frame (m); w: rotor speed 0..1
    at: array<vec4<f32>, HELIS>,
    // each heli's attitude relative to ours, a quaternion (xyz, w)
    rot: array<vec4<f32>, HELIS>,
    // seed, pad flag
    info: array<vec4<f32>, HELIS>,
}

@group(0) @binding(0) var<uniform> hh: Heli;

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

// Into heli i's own frame (metres; skid bottoms rest at y = -2.5).
fn to_local(i: u32, p: vec3<f32>) -> vec3<f32> {
    return quat_rotate(quat_conj(hh.rot[i]), p - hh.at[i].xyz);
}

// The hull, local metres: pod, boom, fin, tailplane, skids, mast.
fn sd_heli(q: vec3<f32>) -> f32 {
    // The cabin pod, fattest at the seats, and the engine housing on top.
    let pod = sd_ellipsoid_c(q, vec3<f32>(0.0, -0.55, -1.2), vec3<f32>(1.15, 1.05, 2.55));
    let roof = sd_round_box(q - vec3<f32>(0.0, 0.5, 0.4), vec3<f32>(0.6, 0.4, 1.5), 0.15);
    var d = min(pod, roof);
    // The tail boom, rising a little to the fin; the tailplane either side.
    let boom = sd_capsule_ab(q, vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(0.0, 0.55, 5.6), 0.34);
    d = min(d, boom);
    let fin = sd_round_box(q - vec3<f32>(0.0, 1.1, 5.75), vec3<f32>(0.05, 0.8, 0.42), 0.04);
    d = min(d, fin);
    let tq = vec3<f32>(abs(q.x) - 0.75, q.y - 0.55, q.z - 4.9);
    d = min(d, sd_round_box(tq, vec3<f32>(0.45, 0.04, 0.3), 0.03));
    // Skids on their struts.
    let sq = vec3<f32>(abs(q.x) - 1.05, q.y + 2.4, q.z);
    d = min(d, sd_capsule_ab(sq, vec3<f32>(0.0, 0.0, -2.2), vec3<f32>(0.0, 0.0, 1.9), 0.1));
    let aq = vec3<f32>(abs(q.x), q.y, q.z);
    d = min(d, sd_capsule_ab(aq, vec3<f32>(0.55, -1.45, -1.2), vec3<f32>(1.05, -2.3, -1.2), 0.07));
    d = min(d, sd_capsule_ab(aq, vec3<f32>(0.55, -1.45, 1.1), vec3<f32>(1.05, -2.3, 1.1), 0.07));
    // The mast, the main hub, and the tail rotor's hub on the right.
    d = min(d, sd_capsule_ab(q, vec3<f32>(0.0, 0.4, -0.2), vec3<f32>(0.0, 1.8, -0.2), 0.13));
    d = min(d, length(q - vec3<f32>(0.0, 1.85, -0.2)) - 0.24);
    d = min(d, length(q - vec3<f32>(0.32, 0.95, 5.5)) - 0.16);
    return d;
}

fn heli_normal(i: u32, p: vec3<f32>) -> vec3<f32> {
    let q = to_local(i, p);
    let e = 0.02;
    let n = normalize(vec3<f32>(
        sd_heli(q + vec3<f32>(e, 0.0, 0.0)) - sd_heli(q - vec3<f32>(e, 0.0, 0.0)),
        sd_heli(q + vec3<f32>(0.0, e, 0.0)) - sd_heli(q - vec3<f32>(0.0, e, 0.0)),
        sd_heli(q + vec3<f32>(0.0, 0.0, e)) - sd_heli(q - vec3<f32>(0.0, 0.0, e)),
    ));
    return quat_rotate(hh.rot[i], n);
}

// Where the ray meets the body underfoot, or a very long way. The radius
// is pulled in a touch so a hull ON the surface is not eaten by it.
fn occluder_t(ray: vec3<f32>) -> f32 {
    let r = hh.occ.w - 40.0;
    if (r <= 0.0) {
        return 1.0e12;
    }
    let c = hh.occ.xyz;
    let b = dot(ray, c);
    let disc = b * b - (dot(c, c) - r * r);
    if (disc <= 0.0 || b <= 0.0) {
        return 1.0e12;
    }
    return b - sqrt(disc);
}

// A rotor disc: the ray against the plane through `c` with normal `n`
// (all local). Returns (t_local_ok, t, radius, angle).
fn disc_hit(ro: vec3<f32>, rd: vec3<f32>, c: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let dn = dot(rd, n);
    if (abs(dn) < 1.0e-5) {
        return vec3<f32>(-1.0, 0.0, 0.0);
    }
    let t = dot(c - ro, n) / dn;
    if (t <= 0.0) {
        return vec3<f32>(-1.0, 0.0, 0.0);
    }
    let h = ro + rd * t - c;
    // Two axes in the disc's plane.
    var u = normalize(cross(n, vec3<f32>(0.0, 0.0, 1.0) + vec3<f32>(0.0, 0.017, 0.0)));
    let v = cross(n, u);
    return vec3<f32>(t, length(h), atan2(dot(h, v), dot(h, u)));
}

// The blur-and-blades look of a spinning rotor: alpha at this radius and
// angle, for a disc of radius R turning at `spin` (0 idle .. 1 flying).
fn rotor_alpha(r: f32, ang: f32, big_r: f32, hub_r: f32, spin: f32, now: f32) -> f32 {
    if (r > big_r || r < hub_r) {
        return 0.0;
    }
    let phase = now * (1.5 + 26.0 * spin);
    // Two blades: the streak is |cos| raised high, softened as it blurs.
    let lobe = abs(cos(ang - phase));
    let sharp = mix(80.0, 8.0, clamp(spin, 0.0, 1.0));
    let blade = pow(lobe, sharp);
    let ring = 0.05 + 0.22 * clamp(spin, 0.0, 1.0);
    // The tips read strongest — a faint band at the rim.
    let tip = smoothstep(big_r * 0.9, big_r * 0.985, r) * 0.25;
    let edge = 1.0 - smoothstep(big_r * 0.985, big_r, r);
    return (ring + blade * 0.55 + tip) * edge;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n_helis = u32(hh.sun.w);
    if (n_helis == 0u) {
        discard;
    }
    let aspect = hh.right.w;
    let tan_half = hh.up.w;
    let ray = normalize(hh.fwd.xyz
        + hh.right.xyz * (in.ndc.x * tan_half * aspect)
        + hh.up.xyz * (in.ndc.y * tan_half));
    let now = hh.fwd.w;
    let sun = normalize(hh.sun.xyz);

    // The nearest solid thing: a hull, or a pad's paint.
    var best_t = 1.0e12;
    var best_i = 0u;
    var pad_hit = false;
    var pad_local = vec3<f32>(0.0);
    var hit = false;
    for (var i = 0u; i < n_helis; i += 1u) {
        let c = hh.at[i].xyz;
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - BOUND_R * BOUND_R * 2.6);
        // The march, inside the bounding sphere only.
        if (disc > 0.0) {
            let t_in = max(b - sqrt(disc), 0.0);
            let t_out = b + sqrt(disc);
            if (t_out > 0.0 && t_in < best_t) {
                var t = t_in;
                let ro = to_local(i, ray * 0.0);
                let rd = quat_rotate(quat_conj(hh.rot[i]), ray);
                let eps = max(0.01, t_in * tan_half * 0.0008);
                for (var k = 0u; k < STEPS; k += 1u) {
                    let d = sd_heli(ro + rd * t);
                    if (d < eps) {
                        if (t < best_t) {
                            best_t = t;
                            best_i = i;
                            pad_hit = false;
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
        }
        // The pad under a parked hull: a plane at the skids' feet.
        if (hh.info[i].y > 0.5) {
            let ro = to_local(i, ray * 0.0);
            let rd = quat_rotate(quat_conj(hh.rot[i]), ray);
            if (abs(rd.y) > 1.0e-5) {
                let t = (-2.5 - ro.y) / rd.y;
                if (t > 0.0 && t < best_t) {
                    let h = ro + rd * t;
                    if (length(h.xz) < 11.0) {
                        best_t = t;
                        best_i = i;
                        pad_hit = true;
                        pad_local = h;
                        hit = true;
                    }
                }
            }
        }
    }
    // The planet's limb hides what is past it.
    if (hit && occluder_t(ray) < best_t - 5.0) {
        hit = false;
    }

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;
    if (hit) {
        let i = best_i;
        if (pad_hit) {
            // Tarmac, a painted ring, and the H between the skids.
            let r = length(pad_local.xz);
            var albedo = vec3<f32>(0.055, 0.055, 0.052)
                * (0.85 + 0.3 * vnoise(vec3<f32>(pad_local.x, 0.0, pad_local.z) * 0.9));
            let ring = smoothstep(9.4, 9.7, r) * (1.0 - smoothstep(10.2, 10.5, r));
            let bar = step(0.9, abs(pad_local.x)) * step(abs(pad_local.x), 1.5)
                * step(abs(pad_local.z), 1.8);
            let cross = step(abs(pad_local.z), 0.3) * step(abs(pad_local.x), 0.9);
            let paint = clamp(ring + bar + cross, 0.0, 1.0);
            albedo = mix(albedo, vec3<f32>(0.62, 0.60, 0.55), paint * 0.9);
            let up_w = quat_rotate(hh.rot[i], vec3<f32>(0.0, 1.0, 0.0));
            let diff = max(dot(up_w, sun), 0.0);
            rgb = albedo * (diff * 1.25 + 0.04);
            alpha = 1.0;
        } else {
            let p = ray * best_t;
            let n = heli_normal(i, p);
            let q = to_local(i, p);
            let seed = hh.info[i].x;
            // Olive drab, weathered; the boom a shade darker; the canopy
            // glass across the nose; skids and rotors near-black steel.
            var albedo = vec3<f32>(0.13, 0.15, 0.10)
                * (0.85 + 0.3 * vnoise(q * 2.1 + seed * 7.0));
            let boomy = smoothstep(1.4, 2.2, q.z);
            albedo = mix(albedo, vec3<f32>(0.10, 0.11, 0.08), boomy * 0.6);
            // Glass: the pod's nose above the sill.
            let glass = sd_ellipsoid_c(q, vec3<f32>(0.0, -0.15, -2.35), vec3<f32>(0.95, 0.75, 1.15));
            let canopy = 1.0 - smoothstep(0.0, 0.3, glass);
            albedo = mix(albedo, vec3<f32>(0.03, 0.05, 0.07), canopy);
            // Steel where the skids and mast live.
            let steel = step(q.y, -1.4) + step(1.55, q.y);
            albedo = mix(albedo, vec3<f32>(0.06, 0.06, 0.065), clamp(steel, 0.0, 1.0) * 0.8);
            let diff = max(dot(n, sun), 0.0);
            let h = normalize(sun - ray);
            let spec = pow(max(dot(n, h), 0.0), 36.0) * mix(0.25, 1.4, canopy);
            let rim = pow(1.0 - abs(dot(n, ray)), 3.0);
            rgb = albedo * (diff * 1.35 + 0.05)
                + vec3<f32>(1.0, 0.97, 0.92) * spec * (0.3 + 0.7 * diff)
                + vec3<f32>(0.10, 0.12, 0.15) * rim * 0.5;
            alpha = 1.0;
        }
    }

    // The rotors: translucent discs over whatever is behind them.
    for (var i = 0u; i < n_helis; i += 1u) {
        let spin = hh.at[i].w;
        let ro = to_local(i, ray * 0.0);
        let rd = quat_rotate(quat_conj(hh.rot[i]), ray);
        // Main rotor: plane y = 1.95, radius 7; tail rotor: plane x =
        // 0.38 at the boom's end, radius 1.35.
        let main = disc_hit(ro, rd, vec3<f32>(0.0, 1.95, -0.2), vec3<f32>(0.0, 1.0, 0.0));
        let tail = disc_hit(ro, rd, vec3<f32>(0.38, 0.95, 5.5), vec3<f32>(1.0, 0.0, 0.0));
        var a = 0.0;
        var t_r = 1.0e12;
        if (main.x > 0.0 && main.x < best_t) {
            a = rotor_alpha(main.y, main.z, 7.0, 0.7, spin, now + hh.info[i].x * 9.0);
            t_r = main.x;
        }
        if (tail.x > 0.0 && tail.x < best_t) {
            let ta = rotor_alpha(tail.y, tail.z, 1.35, 0.2, spin, (now + hh.info[i].x * 9.0) * 1.7);
            if (ta > a) {
                a = ta;
                t_r = tail.x;
            }
        }
        if (a > 0.003 && occluder_t(ray) > t_r) {
            let blade = vec3<f32>(0.05, 0.05, 0.055) * (0.4 + 0.6 * max(dot(vec3<f32>(0.0, 1.0, 0.0), sun), 0.2));
            rgb = mix(rgb, blade, a);
            alpha = max(alpha, a);
        }
    }

    if (alpha < 0.004) {
        discard;
    }
    return vec4<f32>(radiance(rgb, hh.look.x) * alpha, alpha);
}
