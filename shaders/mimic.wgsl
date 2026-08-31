// mimic.wgsl — the other ships (pass: mimic)
//
// A mimic sits in the ring inside a holographic shroud shaped like a
// stone. Struck, the shroud goes: first the ship glows through the rock as
// a cyan hologram (bands of light running its length, a rim), then the
// rock projection winks off (the belt stops drawing it) and the hologram
// hardens into a sun-lit hull. Then it is a ship: engines by its effort
// (amber for a hostile, a cool blue for one that hails, with a white
// beacon pulsing at the nose while it talks), and as a wreck it is dark,
// tumbling, with embers guttering in the damage.
//
// A miner is the same fighter at its tier's size with the tier's parts
// on: ore tanks under the wings, a dorsal collector and fatter nacelles,
// the drill boom off the nose. Working, its beam runs from the nose to
// the rock's face — a thin hot core, a faint halo, ore motes sliding up
// it toward the ship, a glow where it bites. A shield sheen on the big
// ones when a hit is shed.
//
// Every hull is ray-marched from the same fighter SDF as everything else,
// at its own pose and size in the ship's frame, hidden behind nearer
// rocks. Far away — under a couple of pixels — a hull is a lit speck, so
// the ring reads as worked.

const MIMICS: u32 = 12u;
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
    // effort, kind (0 hail, 1 hostile, 2 wreck, 3 miner, 4 hostile
    // miner), wound, seed
    info: array<vec4<f32>, MIMICS>,
    // size (m per SDF unit), tier 0..3, shield sheen, beam on
    more: array<vec4<f32>, MIMICS>,
    // xyz: the beam's far end, ship frame (m)
    beam: array<vec4<f32>, MIMICS>,
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

// Into the ship's own frame, in SDF units (our fighter is 1 m a unit).
fn to_local(i: u32, p: vec3<f32>) -> vec3<f32> {
    return quat_rotate(quat_conj(mm.rot[i]), p - mm.at[i].xyz) / mm.more[i].x;
}

// The hull with the tier's parts on it, SDF units.
fn sd_parts(q: vec3<f32>, tier: f32) -> f32 {
    var d = sd_fighter_exterior(q);
    if (tier > 0.5) {
        // Ore tanks slung under the wings.
        let tq = vec3<f32>(abs(q.x) - 2.3, q.y + 1.35, q.z - 2.6);
        d = min(d, sd_capsule_ab(tq, vec3<f32>(0.0, 0.0, -1.6), vec3<f32>(0.0, 0.0, 1.6), 0.62));
    }
    if (tier > 1.5) {
        // A collector over the spine, and fatter nacelles.
        let cq = q - vec3<f32>(0.0, 0.55, 4.2);
        d = min(d, sd_round_box(cq, vec3<f32>(1.25, 0.42, 1.9), 0.18));
        let eq = vec3<f32>(abs(q.x) - 0.62, q.y + 0.85, q.z);
        d = min(d, sd_capsule_ab(eq, vec3<f32>(0.0, 0.0, 4.0), vec3<f32>(0.0, 0.0, 7.3), 0.64));
    }
    if (tier > 2.5) {
        // The drill boom off the nose, a ring at its head.
        d = min(d, sd_capsule_ab(q, vec3<f32>(0.0, -1.25, -5.5), vec3<f32>(0.0, -1.25, -9.6), 0.34));
        let hq = vec2<f32>(length(q.xy - vec2<f32>(0.0, -1.25)) - 0.75, q.z + 9.4);
        d = min(d, length(hq) - 0.16);
    }
    return d;
}

// The distance in metres.
fn sd_mimic(i: u32, p: vec3<f32>) -> f32 {
    return sd_parts(to_local(i, p), mm.more[i].y) * mm.more[i].x;
}

fn mimic_normal(i: u32, p: vec3<f32>) -> vec3<f32> {
    let e = 0.02 * mm.more[i].x;
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

// Where the beam leaves the hull, ship frame (m): the drill's head on a
// tier-3 miner, the nose otherwise.
fn beam_origin(i: u32) -> vec3<f32> {
    let tier = mm.more[i].y;
    let size = mm.more[i].x;
    var o = vec3<f32>(0.0, -1.2, -6.4);
    if (tier > 2.5) {
        o = vec3<f32>(0.0, -1.25, -9.8);
    }
    return mm.at[i].xyz + quat_rotate(mm.rot[i], o * size);
}

// The ray (from the eye) against the segment a..b: the distance between
// them, the ray's parameter there, and the segment's (0 at a, 1 at b).
fn ray_segment(ray: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let d = b - a;
    let dd = max(dot(d, d), 1e-6);
    let rd = dot(ray, d);
    let ra = dot(ray, a);
    let ad = dot(a, d);
    let den = max(dd - rd * rd, 1e-6);
    let s = clamp((rd * ra - ad) / den, 0.0, 1.0);
    let t = max(ra + s * rd, 0.0);
    let p = a + d * s;
    let dist = length(ray * t - p);
    return vec3<f32>(dist, t, s);
}

const BOUND_R: f32 = 10.4;
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
    let now = mm.fwd.w;
    let sun = normalize(mm.sun.xyz);

    // The light that is not a hull: beams, their bites, and the far
    // specks. Gathered first so a beam shows with no hull under this pixel.
    var light = vec3<f32>(0.0);
    var light_t = 1.0e12;
    // The nearest ship this ray meets.
    var best_t = 1.0e12;
    var best_i = 0u;
    var best_p = vec3<f32>(0.0);
    var hit = false;
    for (var i = 0u; i < n_ships; i += 1u) {
        let c = mm.at[i].xyz;
        let size = mm.more[i].x;
        let bound = BOUND_R * size;
        let dist = length(c);
        let kind = mm.info[i].y;
        let miner = kind > 2.5;
        // The beam, while it is on.
        if (mm.more[i].w > 0.5) {
            let a = beam_origin(i);
            let b = mm.beam[i].xyz;
            let rs = ray_segment(ray, a, b);
            let w = 0.16 * size;
            if (rs.x < w * 40.0 && rs.y > 0.5) {
                let flick = 0.85 + 0.15 * sin(now * 61.0 + rs.z * 30.0);
                let core = exp(-rs.x / w) * 2.6;
                let halo = exp(-rs.x / (w * 6.0)) * 0.22;
                // Ore motes: bright beads sliding from the rock to the ship.
                let along = length(b - a);
                let bead = pow(0.5 + 0.5 * cos((rs.z * along / (4.0 * size) + now * 1.7) * 6.2831853), 9.0);
                let motes = bead * exp(-rs.x / (w * 1.6)) * 1.8 * smoothstep(0.0, 0.08, rs.z);
                var glow = (vec3<f32>(1.0, 0.72, 0.42) * core + vec3<f32>(1.0, 0.35, 0.18) * halo) * flick
                    + vec3<f32>(1.0, 0.9, 0.7) * motes;
                // Where it bites: a hot spot on the rock's face.
                let tb = max(dot(ray, b), 0.0);
                let db = length(ray * tb - b);
                let bite = exp(-db / (1.6 * size)) * (0.9 + 0.3 * sin(now * 23.0)) * 1.4;
                glow += vec3<f32>(1.0, 0.62, 0.30) * bite;
                if (max(glow.r, max(glow.g, glow.b)) > 0.003) {
                    let occ = rock_t(ray);
                    // The bite sits ON the rock: let it through a little.
                    if (occ > rs.y - 0.5 * size || occ > tb - 2.5 * size) {
                        light += glow;
                        light_t = min(light_t, rs.y);
                    }
                }
            }
        }
        // Far away: a speck of its lights.
        let ang_r = bound / max(dist, 1.0);
        if (ang_r < 0.006) {
            let cd = dot(ray, c / max(dist, 1.0));
            if (cd > 0.99999) {
                let ang = acos(clamp(cd, -1.0, 1.0));
                let w = max(0.0022, ang_r * 0.6);
                let g = exp(-(ang * ang) / (w * w)) * smoothstep(0.006, 0.003, ang_r);
                if (g > 0.01 && rock_t(ray) > dist) {
                    let effort = mm.info[i].x;
                    var col = vec3<f32>(0.8, 0.9, 1.0);
                    if (kind > 1.5 && kind < 2.5) {
                        col = vec3<f32>(0.5, 0.35, 0.3);
                    } else if (miner) {
                        col = vec3<f32>(1.0, 0.85, 0.55);
                    } else if (kind > 0.5) {
                        col = vec3<f32>(1.0, 0.55, 0.3);
                    }
                    light += col * g * (1.2 + effort * 1.5);
                    light_t = min(light_t, dist);
                }
            }
        }
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - bound * bound);
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
        let eps = max(0.008 * size, t_in * tan_half * 0.0006);
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
    let lit = max(light.r, max(light.g, light.b)) > 0.002;
    if (!hit) {
        if (!lit) {
            discard;
        }
        return vec4<f32>(tonemap(light, mm.look.x), 0.0);
    }
    if (rock_t(ray) < best_t) {
        if (!lit) {
            discard;
        }
        return vec4<f32>(tonemap(light, mm.look.x), 0.0);
    }
    let i = best_i;
    let p = best_p;
    let n = mimic_normal(i, p);
    let q = to_local(i, p);
    let reveal = mm.at[i].w;
    let effort = mm.info[i].x;
    let kind = mm.info[i].y;
    let wound = mm.info[i].z;
    let seed = mm.info[i].w;
    let tier = mm.more[i].y;
    let sheen = mm.more[i].z;
    let wreck = kind > 1.5 && kind < 2.5;
    let miner = kind > 2.5;
    let hostile = (kind > 0.5 && kind < 1.5) || kind > 3.5;

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
    if (miner) {
        // A working ship: an ochre stripe down each flank, grime in the
        // seams, the tanks and collector a duller plate.
        let stripe = smoothstep(0.34, 0.20, abs(q.y + 0.42)) * step(0.5, abs(q.x)) * step(q.z, 5.0);
        albedo = mix(albedo, vec3<f32>(0.80, 0.52, 0.10), stripe);
        let grime = smoothstep(0.35, 0.65, vnoise(q * 3.1 + seed * 5.0));
        albedo = mix(albedo, albedo * 0.6, grime * 0.6) * 0.78;
        if (tier > 0.5 && q.y < -1.0 && abs(q.x) > 1.5) {
            albedo = mix(albedo, vec3<f32>(0.42, 0.40, 0.36), 0.7);
        }
    } else if (hostile) {
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
    // hailer, a working white-amber for a miner; a wreck's are out, and
    // embers gutter in its scorch.
    let eq = vec3<f32>(abs(q.x) - 0.62, q.y + 0.85, q.z);
    let nozzle_r = select(0.30, 0.50, tier > 1.5);
    let near_nozzle = (1.0 - smoothstep(nozzle_r, nozzle_r + 0.45, length(eq.xy))) * smoothstep(6.9, 7.4, q.z);
    var engine_col = vec3<f32>(0.35, 0.65, 1.0);
    if (hostile) {
        engine_col = vec3<f32>(1.0, 0.42, 0.13);
    } else if (miner) {
        engine_col = vec3<f32>(1.0, 0.80, 0.45);
    }
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
    // A miner's working lights: a lamp on the drill or the nose while the
    // beam is on, and the shield's sheen when a hit is shed.
    if (miner) {
        if (mm.more[i].w > 0.5) {
            var lamp = vec3<f32>(0.0, -1.2, -6.4);
            if (tier > 2.5) {
                lamp = vec3<f32>(0.0, -1.25, -9.6);
            }
            hull += vec3<f32>(1.0, 0.75, 0.45) * exp(-length(q - lamp) * 1.4) * 2.5;
        }
        if (sheen > 0.001) {
            let cells = 0.6 + 0.4 * sin(q.x * 9.0 + now * 3.0) * sin(q.y * 9.0 - now * 2.0) * sin(q.z * 9.0);
            hull += vec3<f32>(0.45, 0.65, 1.0) * rim3 * sheen * 1.6 * cells;
        }
    }

    // The reveal: hologram through the shroud, then the hull hardening
    // under it, the last of the field flickering off its rim.
    let solid = smoothstep(0.42, 1.0, reveal);
    let field = (1.0 - solid) * (0.4 + 0.6 * smoothstep(0.0, 0.3, reveal));
    var rgb = hull * solid + holo * field;
    // Light nearer than the hull lies over it.
    if (light_t < best_t) {
        rgb += light;
    }
    let out = tonemap(rgb, mm.look.x);
    // Premultiplied: the hull is solid; the field and the light let what
    // is behind show through.
    let alpha = max(solid, 0.0);
    return vec4<f32>(out, alpha);
}
