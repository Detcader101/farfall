// cockpit.wgsl — the cabin the pilot sits in (pass: cockpit)
//
// Lane: A. Cost class: moderate — a short SDF march per pixel over the
// cabin and the ship's own nose and wings, bounded to a few metres.
//
// This is a physical cockpit: a fighter's fuselage with the cabin carved
// out of it (common.wgsl, sd_fighter_hull), so the hull has a wall, the
// nose runs out ahead below the glass and the wings sweep back either
// side; solid canopy arches and rails; a sloping dash and side consoles in
// dark metal, lit by the Sun through the glass. On the dash sit SOCKETS —
// recessed pads with a lit rim — one under each instrument the pilot has
// placed, and from each a beam of light stands up to where the hologram
// floats. The holograms (drawn after this, on the glass) are the tech; the
// metal is the ship. TRON in the light, a flight sim in the metal.
//
// Drawn in the SHIP'S frame: every pixel's ray is turned by the pilot's
// head, so looking round is looking round the cabin.

struct Cockpit {
    // xyz: the head's right axis in ship frame. w: line glow 0..2
    right: vec4<f32>,
    // xyz: the head's up axis in ship frame. w: metal brightness 0..1
    up: vec4<f32>,
    // xyz: the head's forward axis in ship frame (-Z is the nose). w: tan(fov/2)
    fwd: vec4<f32>,
    // x: aspect, y: gauge style (0 TRON sockets and beams, 1 JET bowls
    // and bezels, 2 DIAL flush wells), z: on 0..1, w: number of sockets
    misc: vec4<f32>,
    // xyz: the Sun's direction in ship frame. w: exposure
    sun: vec4<f32>,
    // Sockets: xyz the hologram's direction from the head (ship frame), w
    // 0 if unused, else 1 + style (0 TRON, 1 JET, 2 DIAL) + 10 × size.
    pad0: vec4<f32>,
    pad1: vec4<f32>,
    pad2: vec4<f32>,
    pad3: vec4<f32>,
}

@group(0) @binding(0) var<uniform> ck: Cockpit;

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

const MARCH_STEPS: u32 = 48u;
const MAX_T: f32 = 9.5;
// Where the holograms float: this far from the head along their direction.
const HOLO_M: f32 = 1.05;
// The dash top: a plane, sloped toward the pilot. Sockets are set into it.
const DASH_C: vec3<f32> = vec3<f32>(0.0, -0.50, -1.05);
const DASH_N: vec3<f32> = vec3<f32>(0.0, 0.9563, 0.2924); // 17 degrees back

// ---------------------------------------------------------------- cabin

struct Hit {
    d: f32,
    // 0 hull metal, 1 dash/console metal, 2 arch/rail, 3 socket rim
    kind: f32,
}

fn pad_dir(i: i32) -> vec4<f32> {
    if (i == 0) { return ck.pad0; }
    if (i == 1) { return ck.pad1; }
    if (i == 2) { return ck.pad2; }
    return ck.pad3;
}

// A socket's centre: where the hologram's direction meets the dash plane
// (or, if it does not, a floating emitter a little below the hologram).
fn socket_centre(dir: vec3<f32>) -> vec3<f32> {
    let denom = dot(dir, DASH_N);
    if (denom < -1e-4) {
        let t = dot(DASH_C, DASH_N) / denom;
        let p = dir * t;
        if (t > 0.3 && t < 2.2 && abs(p.x) < 1.0) {
            return p;
        }
    }
    return dir * HOLO_M - vec3<f32>(0.0, 0.16, 0.0);
}

fn sd_cabin(p: vec3<f32>) -> Hit {
    // The ship's hull, with the cabin carved out of it.
    var h = Hit(sd_fighter_hull(p), 0.0);
    // Everything inside the cabin sits in one box: outside it, its
    // distance is bound enough.
    let inside = sd_round_box(p - vec3<f32>(0.0, -0.2, -0.3), vec3<f32>(1.1, 1.0, 1.7), 0.0);
    if (inside > 0.25) {
        if (inside < h.d) { h = Hit(inside, 1.0); }
        return h;
    }
    // The dash: a slab set into the nose, its top sloping toward the
    // pilot; and the side consoles at the elbows.
    let ca = 0.9563;
    let sa = 0.2924;
    let dq0 = p - DASH_C;
    let dq = vec3<f32>(dq0.x, dq0.y * ca + dq0.z * sa, -dq0.y * sa + dq0.z * ca);
    let dash = sd_round_box(dq - vec3<f32>(0.0, -0.2, 0.0), vec3<f32>(0.95, 0.2, 0.42), 0.04);
    let cq = vec3<f32>(abs(p.x), p.y, p.z);
    let console = sd_round_box(cq - vec3<f32>(0.74, -0.7, -0.1), vec3<f32>(0.2, 0.1, 0.8), 0.03);
    var furniture = min(dash, console);
    // The instrument hood: a lip along the dash's far edge, shading the
    // glass from the dash's own light.
    let hood = sd_round_box(dq - vec3<f32>(0.0, 0.04, -0.40), vec3<f32>(0.97, 0.035, 0.045), 0.02);
    furniture = min(furniture, hood);
    // Switch banks on both consoles: rows of small toggles, repeated.
    let sq = vec3<f32>(cq.x - 0.74, cq.y + 0.60, fract((cq.z + 0.5) / 0.09) * 0.09 - 0.045);
    let in_bank = step(abs(cq.z - 0.1), 0.55);
    // A toggle: a short upright cylinder with a bead on top.
    let toggle = max(length(sq.xz) - 0.007, abs(sq.y - 0.014) - 0.014);
    let bead = length(sq - vec3<f32>(0.0, 0.03, 0.0)) - 0.011;
    let switch_ = mix(1e9, min(toggle, bead), in_bank);
    furniture = min(furniture, switch_);
    // The stick between the knees and the throttle on the left console:
    // the pilot's own hands' furniture.
    let stick = sd_capsule_ab(p, vec3<f32>(0.0, -1.0, -0.45), vec3<f32>(0.0, -0.62, -0.5), 0.022);
    let grip = sd_ellipsoid_c(p, vec3<f32>(0.0, -0.58, -0.5), vec3<f32>(0.035, 0.07, 0.04));
    let throttle = sd_round_box(p - vec3<f32>(-0.74, -0.53, -0.3), vec3<f32>(0.045, 0.06, 0.03), 0.012);
    furniture = min(furniture, min(min(stick, grip), throttle));
    // Sockets: shallow recesses in the furniture under each hologram, with
    // a raised rim.
    let n = i32(ck.misc.w);
    var rim = 1e9;
    // Only near the dash and consoles are there sockets to cut.
    let near_dash = furniture < 0.2;
    for (var i = 0; i < 4; i += 1) {
        if (i >= n || !near_dash) { break; }
        let pd = pad_dir(i);
        if (pd.w < 0.5) { continue; }
        // w = (style + 1) + 10 × round(size × 100): exact integers.
        let style = pd.w - 10.0 * floor(pd.w / 10.0) - 1.0;
        let size = max(floor(pd.w / 10.0) / 100.0, 0.25);
        let c = socket_centre(pd.xyz);
        // The socket's geometry in its own scaled space: distances come
        // back multiplied by the size, so the march stays honest.
        let q = (p - c) / size;
        let along = dot(q, DASH_N);
        let radial = length(q - DASH_N * along);
        if (style > 1.5) {
            // DIAL: a shallow flush well, the instrument's face set into
            // the dash a finger deep behind a raised bezel — the face
            // itself is drawn in this plane by the gauge pass. The cut
            // reaches above the surface too, or it is flush and cuts
            // nothing.
            let well = max(radial - 0.205, abs(along - 0.01) - 0.06) * size;
            furniture = max(furniture, -well);
            let bezel = (length(vec2<f32>(radial - 0.215, along - 0.012)) - 0.016) * size;
            rim = min(rim, bezel);
        } else if (style > 0.5) {
            // JET: a spherical bowl hollowed into the dash, the classic
            // round instrument's well, with a raised bezel at its mouth
            // the hologram sits in. The dial is drawn after the cabin, on
            // the glass, so there is nothing here to fight it for depth.
            let bowl = (length(q + DASH_N * 0.10) - 0.21) * size;
            furniture = max(furniture, -bowl);
            let bezel = (length(vec2<f32>(radial - 0.185, along - 0.015)) - 0.016) * size;
            rim = min(rim, bezel);
        } else {
            // TRON: a shallow recess, a thin lit rim, a beam up to the
            // hologram.
            let recess = max(radial - 0.085, abs(along) - 0.025) * size;
            furniture = max(furniture, -recess);
            let ring = (length(vec2<f32>(radial - 0.095, along - 0.012)) - 0.012) * size;
            rim = min(rim, ring);
        }
    }
    if (furniture < h.d) { h = Hit(furniture, 1.0); }
    if (rim < h.d) { h = Hit(rim, 3.0); }
    // The frame is all above y = -0.35: below -0.4 the gap to that height
    // is a safe lower bound — and one that never reaches zero, so the
    // march cannot mistake the bound's plane for a surface.
    let frame = select(-0.35 - p.y, sd_frame(p), p.y > -0.4);
    if (frame < h.d) { h = Hit(frame, 2.0); }
    return h;
}

// Canopy structure: a front arch and a rear arch (tori, tilted), and two
// rails running between them over the pilot's shoulders.
fn sd_frame(p: vec3<f32>) -> f32 {
    let fa = p - vec3<f32>(0.0, -0.25, -1.55);
    let faq = vec3<f32>(fa.x, fa.y * 0.92 - fa.z * 0.39, fa.y * 0.39 + fa.z * 0.92);
    let front_arch = max(length(vec2<f32>(length(faq.xy) - 0.98, faq.z)) - 0.035, -faq.y - 0.1);
    let ra = p - vec3<f32>(0.0, -0.25, 1.25);
    let raq = vec3<f32>(ra.x, ra.y * 0.96 + ra.z * 0.28, -ra.y * 0.28 + ra.z * 0.96);
    let rear_arch = max(length(vec2<f32>(length(raq.xy) - 0.92, raq.z)) - 0.035, -raq.y - 0.1);
    let rq = vec3<f32>(abs(p.x), p.y, p.z);
    let rail = sd_capsule_ab(rq, vec3<f32>(0.55, 0.72, -1.2), vec3<f32>(0.5, 0.64, 1.0), 0.028);
    return min(min(front_arch, rear_arch), rail);
}

// Tetrahedral gradient: four samples, not six.
// One call site in a loop, so the (large) scene function is inlined
// once here rather than four times — code size is occupancy.
fn cabin_normal(p: vec3<f32>) -> vec3<f32> {
    let e = 0.004;
    var n = vec3<f32>(0.0);
    for (var i = 0; i < 4; i += 1) {
        let k = vec3<f32>(
            select(-1.0, 1.0, (i & 1) == 0),
            select(-1.0, 1.0, i >= 2),
            select(-1.0, 1.0, i == 1 || i == 3),
        );
        n += k * sd_cabin(p + k * e).d;
    }
    return normalize(n);
}

// The light lines of the cabin: along the dash's front edge, the console
// edges, the rails, the wing leading edges. Distance to the nearest, for
// an emissive glow on whatever surface is near one.
fn sd_lines(p: vec3<f32>) -> f32 {
    var d = sd_capsule_ab(p, vec3<f32>(-0.95, -0.62, -0.65), vec3<f32>(0.95, -0.62, -0.65), 0.004);
    let cq = vec3<f32>(abs(p.x), p.y, p.z);
    // Panel seams across the dash, and the strip of light under the hood.
    d = min(d, sd_capsule_ab(cq, vec3<f32>(0.32, -0.6, -0.65), vec3<f32>(0.32, -0.45, -1.4), 0.003));
    d = min(d, sd_capsule_ab(p, vec3<f32>(-0.95, -0.43, -1.40), vec3<f32>(0.95, -0.43, -1.40), 0.004));
    // Console borders.
    d = min(d, sd_capsule_ab(cq, vec3<f32>(0.56, -0.6, 0.7), vec3<f32>(0.56, -0.6, -0.9), 0.003));
    d = min(d, sd_capsule_ab(cq, vec3<f32>(0.94, -0.6, 0.7), vec3<f32>(0.94, -0.6, -0.9), 0.004));
    d = min(d, sd_capsule_ab(cq, vec3<f32>(0.55, 0.72, -1.2), vec3<f32>(0.5, 0.64, 1.0), 0.004));
    // Wing leading edges, seen from inside: a line of light along each.
    d = min(d, sd_capsule_ab(cq, vec3<f32>(1.0, -0.92, 1.2), vec3<f32>(5.8, -0.92, 4.6), 0.01));
    // The spine of the nose.
    d = min(d, sd_capsule_ab(p, vec3<f32>(0.0, 0.0, -1.9), vec3<f32>(0.0, -0.55, -6.2), 0.008));
    return d;
}

// Light from the socket beams on a ray that runs `reach` metres from the
// head: each beam is a thin line from the socket up to the hologram, and
// the ray's light is set by how close it passes — sampled along the beam,
// closest approach to the ray taken. Seen from the head the beams are
// nearly end-on, so each is a bright point under its hologram with a soft
// skirt, not a wash.
fn beam_light(ray: vec3<f32>, reach: f32) -> f32 {
    let n = i32(ck.misc.w);
    var g = 0.0;
    for (var i = 0; i < 4; i += 1) {
        if (i >= n) { break; }
        let pd = pad_dir(i);
        if (pd.w < 0.5) { continue; }
        let style = pd.w - 10.0 * floor(pd.w / 10.0) - 1.0;
        if (style > 0.5) { continue; }
        let c = socket_centre(pd.xyz);
        let top = pd.xyz * HOLO_M;
        // Closest approach of the beam segment to the ray's line: the
        // offset from the line is affine in the beam parameter, so the
        // nearest point is a clamped quadratic minimum.
        let e = top - c;
        let a = c - ray * dot(c, ray);
        let b = e - ray * dot(e, ray);
        let u = clamp(-dot(a, b) / max(dot(b, b), 1e-8), 0.0, 1.0);
        let q = c + e * u;
        let t = clamp(dot(q, ray), 0.0, reach);
        let best = length(q - ray * t);
        g += exp(-best / 0.03) * (1.0 - 0.5 * u);
    }
    return g;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let on = ck.misc.z;
    if (on < 0.01) {
        return vec4<f32>(0.0);
    }
    let aspect = ck.misc.x;
    let tan_half = ck.fwd.w;
    let glow_k = ck.right.w;
    let metal_k = ck.up.w;
    let sun = normalize(ck.sun.xyz);
    let exposure = ck.sun.w;

    // The ray in the ship's frame: x right, y up, -z the nose.
    let ray = normalize(
        ck.fwd.xyz + ck.right.xyz * (in.ndc.x * tan_half * aspect) + ck.up.xyz * (in.ndc.y * tan_half)
    );

    // Sky early-out: a ray that leaves the glass box through its top, or
    // through its front or sides above the hull's brow, meets nothing but
    // (maybe) the frame — test the frame's bars analytically and skip the
    // march for the open sky. Most of the screen, most of the time.
    {
        let bmin = vec3<f32>(-0.95, -0.55, -1.85);
        let bmax = vec3<f32>(0.95, 1.45, 0.95);
        let inv = 1.0 / ray;
        let t1 = (bmin - vec3<f32>(0.0)) * inv;
        let t2 = (bmax - vec3<f32>(0.0)) * inv;
        let tfar = min(min(max(t1.x, t2.x), max(t1.y, t2.y)), max(t1.z, t2.z));
        let exit = ray * tfar;
        let through_top = exit.y > 1.4;
        let over_brow = exit.y > -0.05 && (exit.z < -1.8 || abs(exit.x) > 0.9 || exit.z > 0.9);
        if (through_top || over_brow) {
            // Near a bar of the frame? Distance from the ray (a line from
            // the origin) to each bar's axis, roughly: sample the bars'
            // SDF at the exit point and a point halfway — enough to catch
            // anything within a bar's width of the line.
            var near = 1e9;
            for (var i = 0; i < 3; i += 1) {
                near = min(near, sd_frame(exit * (0.5 + 0.25 * f32(i))));
            }
            if (near > 0.12) {
                // Open sky: written transparent, not discarded, so a
                // redraw in place leaves no stale pixel behind.
                return vec4<f32>(0.0);
            }
        }
    }

    // March.
    var t = 0.02;
    var hit = Hit(1e9, -1.0);
    var beams = 0.0;
    for (var i = 0u; i < MARCH_STEPS; i += 1u) {
        let p = ray * t;
        let h = sd_cabin(p);
        // Gather the beams' light on the way (only close to the head,
        // where the beams are).
        if (h.d < 0.0018 * max(t, 0.5)) {
            hit = Hit(t, h.kind);
            break;
        }
        t += max(h.d, 0.003);
        if (t > MAX_T) {
            break;
        }
        // Out of steps while still skimming a surface (a grazing ray on the
        // nose, say): call it a hit rather than let the sky show through
        // in a stipple.
        if (i == MARCH_STEPS - 1u && h.d < 0.01) {
            hit = Hit(t, h.kind);
        }
    }

    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;

    if (hit.kind >= 0.0) {
        // Settle onto the surface: the march stops within its epsilon of
        // it, and that last millimetre or two, varying with how many steps
        // the ray took, shows as contours in the shading unless removed.
        var tt = hit.d;
        for (var k = 0; k < 2; k += 1) {
            tt += sd_cabin(ray * tt).d;
        }
        let p = ray * tt;
        let n = cabin_normal(p);
        // Dark metal: graphite with a cool sheen, lit by the Sun through
        // the glass, a fill from the cabin's own light, a fresnel rim.
        let ndl = max(dot(n, sun), 0.0);
        let half_v = normalize(sun - ray);
        let spec = pow(max(dot(n, half_v), 0.0), 48.0);
        let fresnel = pow(1.0 - max(dot(n, -ray), 0.0), 4.0);
        // What the metal reflects: a cheap sky — black space above the
        // sill, the planet's blue-grey glow below, a warm band for the
        // Sun — seen in the mirror of the surface, strongest at the rim.
        let refl = reflect(ray, n);
        let env = mix(vec3<f32>(0.05, 0.07, 0.10), vec3<f32>(0.012, 0.014, 0.02), smoothstep(-0.3, 0.3, refl.y))
            + vec3<f32>(1.0, 0.9, 0.7) * pow(max(dot(refl, sun), 0.0), 24.0) * 0.6;
        // Contact shadow: how open the space above the surface is, from
        // one distance sample a hand off it — the sockets, the lip under
        // the hood and the foot of the stick all darken.
        let ao = clamp(sd_cabin(p + n * 0.08).d / 0.08, 0.15, 1.0);
        // Linear albedos: dark graphite reads as mid-grey once tonemapped
        // and gamma'd, so these are low.
        var base = vec3<f32>(0.030, 0.032, 0.038);
        if (hit.kind > 0.5 && hit.kind < 1.5) {
            base = vec3<f32>(0.018, 0.02, 0.024);
        } else if (hit.kind > 1.5 && hit.kind < 2.5) {
            base = vec3<f32>(0.05, 0.052, 0.06);
        }
        var lit = (base * (0.22 + 0.9 * ndl) + env * (0.15 + 0.55 * fresnel)) * ao
            + vec3<f32>(0.9, 0.95, 1.0) * spec * 0.35;
        // The socket rims are lit from within (TRON); a JET bezel is a
        // brushed ring with a thread of light at its inner edge.
        if (hit.kind > 2.5) {
            lit = select(cyan * 1.0 * glow_k, lit * 1.6 + cyan * 0.12 * glow_k, ck.misc.y > 0.5);
        }
        // Emissive lines where the surface runs near one of the light
        // lines; a hint of the cabin light everywhere.
        let line = exp(-sd_lines(p) / 0.008);
        lit += cyan * line * 0.9 * glow_k;
        // Engine light on the nacelles seen from behind: not from here.
        colour = tonemap(lit * metal_k * 1.5, exposure);
        alpha = 1.0;
    }

    // The beams, up to where the ray stopped (TRON only: a JET dial sits
    // in its bowl). Saturating: a beam seen end-on is a bright point, not
    // a white-out.
    if (ck.misc.w > 0.5) {
        let reach = min(select(MAX_T, hit.d, hit.kind >= 0.0), 1.6);
        beams = beam_light(ray, reach);
    }
    colour += cyan * (1.0 - exp(-beams)) * 0.55 * glow_k;

    if (alpha < 0.002 && dot(colour, colour) < 1e-6) {
        return vec4<f32>(0.0);
    }
    // Dither: dark metal in smooth gradients bands in eight bits.
    colour += vec3<f32>(dither_px(in.pos.xy)) * alpha;
    return vec4<f32>(colour * on, alpha * on);
}
