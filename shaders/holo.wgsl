// holo.wgsl — the holo3PP: a volumetric hologram in the cabin (pass: holo)
//
// Lane: A (vertex+fragment only). Cost class: bounded march (one sphere
// test ends it for most of the screen; inside, a short march of the
// fighter SDF at toy scale).
//
// Third person without ever leaving first person — and not a screen. A
// real 3D hologram stands over its emitter in the dash: the ship itself,
// in miniature, at its true attitude (the hologram is in the ship's frame,
// so the little ship always agrees with the real one), and around it the
// things the canopy cannot show — the velocity vector, the nearest body at
// its true bearing and angular size (the ground swells up under the ship
// as it comes in to land), the Sun's bearing, the engines' state. The
// pilot's eye is a real eye: turn the head and the hologram has parallax,
// lean over and look down on it. Light, not glass: it adds over the cabin
// and never occludes it.

struct Holo {
    // xyz: the head's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the head's forward axis. w: time (s)
    fwd: vec4<f32>,
    // xyz: the hologram's centre in the ship's frame (m). w: its radius (m)
    centre: vec4<f32>,
    // xyz: velocity direction in ship frame. w: arrow length 0..1
    vel: vec4<f32>,
    // xyz: the nearest body's bearing in ship frame. w: sin of its
    // angular radius (0: none)
    body: vec4<f32>,
    // xyz: the Sun's bearing in ship frame. w: shown (0 skips)
    sun: vec4<f32>,
    // x: engine effort 0..1, y: hyper field 0..1, z: socket height above
    // the dash (m), w: unused
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> holo: Holo;

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

// The fighter's bounding radius in its own units (jet.wgsl's BOUND_R).
const SHIP_R: f32 = 10.5;
// The little scene reaches past the ship: the body's shell and the Sun's
// bead sit at this radius (ship units).
const SCENE_R: f32 = 13.5;
const STEPS: u32 = 80u;

const KIND_SHIP: f32 = 0.0;
const KIND_VEL: f32 = 1.0;
const KIND_BODY: f32 = 2.0;
const KIND_SUN: f32 = 3.0;

// The miniature, in ship units about the hologram's centre. Returns
// (distance, kind).
fn holo_scene(q: vec3<f32>) -> vec2<f32> {
    var best = vec2<f32>(sd_fighter_exterior(q), KIND_SHIP);
    // The velocity vector: a rod from the ship's heart, as long as the
    // speed's log, tipped with a bead.
    let len = SHIP_R * 1.15 * holo.vel.w;
    if (len > 0.05) {
        let tip = holo.vel.xyz * len;
        let rod = sd_capsule_ab(q, vec3<f32>(0.0), tip, 0.10);
        let bead = length(q - tip) - 0.32;
        let d = min(rod, bead);
        if (d < best.x) { best = vec2<f32>(d, KIND_VEL); }
    }
    // The nearest body: a shell at its true bearing, as big in the little
    // sky as it is in the real one — the ground closing in as the ship
    // descends.
    if (holo.body.w > 0.001) {
        let shell = holo.body.xyz * SCENE_R;
        let r = SCENE_R * holo.body.w;
        let d = length(q - shell) - r;
        if (d < best.x) { best = vec2<f32>(d, KIND_BODY); }
    }
    // The Sun: a bead on its bearing.
    let sun_d = length(q - holo.sun.xyz * SCENE_R * 0.92) - 0.45;
    if (sun_d < best.x) { best = vec2<f32>(sun_d, KIND_SUN); }
    return best;
}

fn holo_normal(q: vec3<f32>) -> vec3<f32> {
    let e = 0.03;
    return normalize(vec3<f32>(
        holo_scene(q + vec3<f32>(e, 0.0, 0.0)).x - holo_scene(q - vec3<f32>(e, 0.0, 0.0)).x,
        holo_scene(q + vec3<f32>(0.0, e, 0.0)).x - holo_scene(q - vec3<f32>(0.0, e, 0.0)).x,
        holo_scene(q + vec3<f32>(0.0, 0.0, e)).x - holo_scene(q - vec3<f32>(0.0, 0.0, e)).x,
    ));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (holo.sun.w < 0.5) {
        discard;
    }
    let aspect = holo.right.w;
    let tan_half = holo.up.w;
    let ray = normalize(holo.fwd.xyz
        + holo.right.xyz * (in.ndc.x * tan_half * aspect)
        + holo.up.xyz * (in.ndc.y * tan_half));
    let centre = holo.centre.xyz;
    let radius = max(holo.centre.w, 1e-3);
    let time = holo.fwd.w;
    // Metres per ship unit: the whole scene fits the hologram's radius.
    let s = radius / SCENE_R;

    let cyan = vec3<f32>(0.40, 0.90, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.25);
    let flicker = 0.94 + 0.06 * sin(time * 37.0) * sin(time * 5.3);
    var rgb = vec3<f32>(0.0);

    // The emitter: a ring of light on the dash under the hologram, from
    // its own plane test — it is not cut by the march's bound.
    var emitter = vec3<f32>(0.0);
    {
        let base = centre - vec3<f32>(0.0, radius + holo.misc.z, 0.0);
        let normal = DIAL_DASH_N;
        let denom = dot(ray, normal);
        if (denom < -1e-4) {
            let tt = dot(base, normal) / denom;
            let hit = ray * tt - base;
            let rr = length(hit) / radius + step(tt, 0.0) * 9.0;
            let ring = (1.0 - smoothstep(0.0, 0.05, abs(rr - 0.80)))
                + 0.08 * (1.0 - smoothstep(0.55, 0.80, rr));
            emitter = cyan * ring * 0.9;
        }
    }

    // The bound: a sphere about the hologram's centre. Rays that miss it
    // are done at once (nearly all of the screen).
    let bound = radius * 1.08;
    let b = dot(ray, centre);
    let disc = b * b - (dot(centre, centre) - bound * bound);
    let t_out = b + sqrt(max(disc, 0.0));
    if (disc < 0.0 || t_out <= 0.0) {
        if (dot(emitter, emitter) < 1e-6) {
            discard;
        }
        return vec4<f32>(emitter * flicker, 1.0);
    }
    let t_in = max(b - sqrt(disc), 0.0);

    // The emitter's light: a faint column of it above the socket, thinning
    // with height — the space the hologram is drawn in, visibly lit.
    {
        let base = centre - vec3<f32>(0.0, radius, 0.0);
        let mid = ray * clamp(b, t_in, t_out);
        let axis = mid - base;
        let h = clamp(axis.y / (2.0 * radius), 0.0, 1.0);
        let off = length(axis.xz);
        let column = exp(-(off / radius) * (off / radius) * 3.0) * (1.0 - h) * 0.045;
        rgb += cyan * column;
    }

    // March the miniature. Translucent: the first surface is shaded, and
    // the ray goes on to the next so the far side shows through.
    var t = t_in;
    var passes = 0u;
    for (var i = 0u; i < STEPS; i += 1u) {
        let p = ray * t;
        let q = (p - centre) / s;
        let hit = holo_scene(q);
        let d = hit.x * s;
        if (d < 0.0025) {
            let n = holo_normal(q);
            let facing = abs(dot(n, ray));
            let rim = pow(1.0 - facing, 2.0);
            // Scanlines through the volume, drifting up: the hologram is
            // drawn in light, layer by layer.
            let scan = 0.80 + 0.20 * sin(p.y * 900.0 - time * 4.0);
            var lit = vec3<f32>(0.0);
            if (hit.y == KIND_SHIP) {
                // Sunlight on the little hull, so its shape reads; the rim
                // in the hologram's own cyan; the nozzles by the engines.
                let diff = max(dot(n, normalize(holo.sun.xyz)), 0.0);
                lit = cyan * (0.18 + 0.30 * diff + 0.75 * rim);
                let eq = vec3<f32>(abs(q.x) - 0.62, q.y + 0.85, q.z);
                let nozzle = (1.0 - smoothstep(0.30, 0.75, length(eq.xy))) * smoothstep(6.9, 7.4, q.z);
                lit += (amber * holo.misc.x * 1.6 + vec3<f32>(0.35, 0.65, 1.0) * holo.misc.y * 2.0) * nozzle;
            } else if (hit.y == KIND_VEL) {
                lit = vec3<f32>(0.75, 1.0, 1.0) * (0.55 + 0.6 * rim);
            } else if (hit.y == KIND_BODY) {
                // A wire globe: meridians and parallels in the ship's frame
                // over a faint shell, so the ground has a grain and a
                // horizon.
                let lat = asin(clamp(n.y, -1.0, 1.0));
                let lon = atan2(n.z, n.x);
                let wire = max(
                    1.0 - smoothstep(0.0, 0.045, abs(fract(lat * 5.73) - 0.5) * 0.35),
                    1.0 - smoothstep(0.0, 0.045, abs(fract(lon * 3.82) - 0.5) * 0.35));
                lit = amber * (0.05 + 0.10 * rim + 0.55 * wire);
            } else {
                lit = vec3<f32>(1.0, 0.95, 0.80) * 1.4;
            }
            rgb += lit * scan * mix(1.0, 0.45, f32(passes));
            passes += 1u;
            if (passes >= 2u) {
                break;
            }
            // Step through the surface and carry on: the back of the
            // hologram shows through the front.
            t += 0.02 * radius;
            continue;
        }
        t += max(d, 0.0025);
        if (t > t_out) {
            break;
        }
    }

    // Out through the glassware's own falloff, and additive: light only.
    let out = (rgb + emitter) * flicker;
    return vec4<f32>(out, 1.0);
}
