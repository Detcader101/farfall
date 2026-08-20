// cluster.wgsl — the holographic instrument cluster (SPEC §6.5, pass: cluster)
//
// Lane: A. Cost class: trivial-to-low. Six instruments share one fullscreen
// triangle; each owns a bounding disc or box, so a pixel that belongs to no
// instrument leaves after a handful of distance checks. One pass rather than
// six is the whole reason this is affordable: six fullscreen draws would pay
// six full-screen fragment invocations to light a strip of glass.
//
// A helicopter panel rebuilt for a spaceship — speed and altitude dials
// flanking an attitude hologram, thrust and vertical-speed tapes at the
// edges, a compass ribbon overhead. Everything is signed-distance fields,
// additively blended, so it reads as light projected on air: no quad, no
// panel, black is absence.
//
// The centrepiece is the attitude holo: the ship itself as a wireframe — 17
// vertices and 29 edges held in this file — projected through a chase view
// and banking inside a ring that marks the local horizontal plane. It is an
// artificial horizon that happens to be your own hull.
//
// Rust decides, the shader draws. Every needle arrives here as a pre-computed
// fraction (render/src/cluster.rs) because a scale rule written in two
// languages is a scale rule that will eventually disagree with itself.

struct Cluster {
    // x: visibility 0..1, y: time s, z: aspect (w/h), w: target height px
    frame: vec4<f32>,
    // x: speed m/s, y: speed fraction, z: altitude m, w: altitude fraction
    speed_alt: vec4<f32>,
    // x: vertical speed m/s, y: VSI fraction (signed), z: throttle, w: g fraction
    rate: vec4<f32>,
    // x: heading rad, y: pitch rad, z: roll rad, w: flag bits
    att: vec4<f32>,
    // Ship body -> heading-relative local-horizontal rotation, by column.
    m0: vec4<f32>,
    m1: vec4<f32>,
    m2: vec4<f32>,
    // xyz: velocity direction in the same frame, w: 1 if worth drawing
    prograde: vec4<f32>,
}

@group(0) @binding(0) var<uniform> cl: Cluster;

const FLAG_FC: u32 = 1u;
const FLAG_BOOST: u32 = 2u;
const FLAG_BRAKE: u32 = 4u;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

const TAU_C: f32 = 6.28318531;
// Canopy radius, in aspect-corrected screen units. Smaller bends harder.
const CANOPY_R: f32 = 1.55;

// The canopy projection: the cluster is not painted on the screen, it is
// painted on the inside of a spherical shell centred on the pilot's eye, and
// the screen shows that shell in perspective. Content at shell angle t lands
// on screen at tan(t), so this inverse — screen pixel back to shell angle —
// is atan: near the centre it is the identity, and toward the rim each shell
// unit covers ever more screen, so instruments there stretch and bow the way
// the inside of a dome does. (asin here is the opposite surface: the outside
// of a ball, rim compressed.)
//
// Instrument anchors below are positions on the shell, not in NDC: the
// cockpit is a fixed object and the viewport crops it, which is why the
// cluster keeps its proportions when the window changes shape.
fn canopy(ndc: vec2<f32>, aspect: f32) -> vec2<f32> {
    let v = vec2<f32>(ndc.x * aspect, ndc.y);
    let r = length(v);
    if (r < 1e-4) {
        return v;
    }
    let x = r / CANOPY_R;
    return v * (atan(x) / x);
}

// ------------------------------------------------------------------ layout
//
// Shell units: 1.0 is half the screen height at the centre of the canopy. At
// 16:9 the visible shell spans about x +-1.33, y +-0.89; at 4:3, x +-1.10.
// Everything below sits inside the narrower of the two.

const HOLO_C: vec2<f32> = vec2<f32>(0.0, -0.44);
const HOLO_R: f32 = 0.200;
const SPD_C: vec2<f32> = vec2<f32>(-0.50, -0.42);
const ALT_C: vec2<f32> = vec2<f32>(0.50, -0.42);
const DIAL_R: f32 = 0.135;
const THR_C: vec2<f32> = vec2<f32>(-0.76, -0.42);
const VSI_C: vec2<f32> = vec2<f32>(0.76, -0.42);
const TAPE_H: f32 = 0.155;
const TAPE_W: f32 = 0.030;
const HDG_C: vec2<f32> = vec2<f32>(0.0, 0.60);
const HDG_HW: f32 = 0.40;
const HDG_HH: f32 = 0.055;

// Dial sweep: zero at 7 o'clock, full scale at 5 o'clock — 240 degrees with
// the gap at the bottom, jet-gauge fashion. Angles from 12 o'clock, clockwise.
const SWEEP_HALF: f32 = 2.0943951;

// Chase-view elevation for the hologram: 20 degrees above the horizontal.
const SIN_PHI: f32 = 0.34202015;
const COS_PHI: f32 = 0.93969262;

// ------------------------------------------------------------------- sdfs

// Capsule: distance to segment ab. The workhorse — needles, ticks, digit
// segments and every wireframe edge are this function.
fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn box_dist(p: vec2<f32>, half: vec2<f32>) -> f32 {
    let d = abs(p) - half;
    return length(max(d, vec2<f32>(0.0))) + min(max(d.x, d.y), 0.0);
}

// First-order distance to the ellipse x^2/a^2 + y^2/b^2 = 1. Exact distance
// to an ellipse needs a root solve; the gradient estimate is within a pixel
// at the eccentricities here and costs a tenth as much.
fn ellipse_ring(p: vec2<f32>, a: f32, b: f32) -> f32 {
    let k = vec2<f32>(p.x / a, p.y / b);
    let f = dot(k, k) - 1.0;
    let g = 2.0 * vec2<f32>(k.x / a, k.y / b);
    return abs(f) / max(length(g), 1e-5);
}

// Seven-segment digit: bit i of `mask` lights segment i.
// 0 top, 1 top-right, 2 bottom-right, 3 bottom, 4 bottom-left, 5 top-left,
// 6 middle. Cell is 2w wide, 2h tall, centred on the origin.
fn digit_dist(p: vec2<f32>, mask: u32, w: f32, h: f32) -> f32 {
    var d = 1e9;
    let ends: array<vec4<f32>, 7> = array<vec4<f32>, 7>(
        vec4<f32>(-w, h, w, h),
        vec4<f32>(w, h, w, 0.0),
        vec4<f32>(w, 0.0, w, -h),
        vec4<f32>(-w, -h, w, -h),
        vec4<f32>(-w, 0.0, -w, -h),
        vec4<f32>(-w, h, -w, 0.0),
        vec4<f32>(-w, 0.0, w, 0.0),
    );
    for (var i = 0u; i < 7u; i += 1u) {
        if ((mask & (1u << i)) != 0u) {
            let e = ends[i];
            d = min(d, seg_dist(p, e.xy, e.zw));
        }
    }
    return d;
}

fn digit_mask(n: u32) -> u32 {
    let masks = array<u32, 10>(63u, 6u, 91u, 79u, 102u, 109u, 125u, 7u, 127u, 111u);
    return masks[min(n, 9u)];
}

// Right-aligned integer readout, `count` digits, centred on p. Leading zeros
// stay lit: instruments read as instruments, not as text. Returns
// (structure glow, hot core) like every draw helper here.
fn number(p: vec2<f32>, value: f32, count: u32, w: f32, h: f32, pitch: f32, aa: f32) -> vec2<f32> {
    var acc = vec2<f32>(0.0);
    let span = f32(count) * pitch * 0.5;
    if (abs(p.x) > span + pitch || abs(p.y) > h * 2.0) {
        return acc;
    }
    var v = u32(clamp(round(value), 0.0, 99999.0));
    for (var i = 0u; i < count; i += 1u) {
        let d = v % 10u;
        v = v / 10u;
        let x = (f32(count - 1u) * 0.5 - f32(i)) * pitch;
        let cell = p - vec2<f32>(x, 0.0);
        if (abs(cell.x) > pitch) {
            continue;
        }
        let dd = digit_dist(cell, digit_mask(d), w, h);
        acc.y += 0.9 * (1.0 - smoothstep(0.0, aa * 1.8, dd - 0.0016));
        acc.x += 0.25 * (1.0 - smoothstep(0.0, 0.008, dd));
    }
    return acc;
}

// ------------------------------------------------------------------ dials

// Arc dial: ring, tick ladder, sweep tape, needle, hub. `frac` is 0..1 and
// arrives pre-scaled — this function has no opinion about what it measures,
// which is why speed (linear) and altitude (logarithmic) can share it.
fn dial(p: vec2<f32>, radius: f32, frac: f32, aa: f32) -> vec2<f32> {
    var acc = vec2<f32>(0.0);
    let theta = atan2(p.x, p.y);
    let r = length(p);
    let in_sweep = abs(theta) < SWEEP_HALF;
    let needle_theta = -SWEEP_HALF + 2.0 * SWEEP_HALF * clamp(frac, 0.0, 1.0);

    if (in_sweep) {
        let ring = abs(r - radius);
        acc.x += 0.9 * (1.0 - smoothstep(0.0, aa * 1.6, ring - 0.0016));
        acc.x += 0.18 * (1.0 - smoothstep(0.0, 0.012, ring));
    }

    // Ticks: angular slots cut radially inward from the ring, minor every
    // 1/30 of scale and major every 1/6.
    if (in_sweep && r < radius && r > radius - 0.030) {
        let t = (theta + SWEEP_HALF) / (2.0 * SWEEP_HALF);
        let minor = abs(fract(t * 30.0) - 0.5);
        let major = abs(fract(t * 6.0) - 0.5);
        let ang_aa = aa / max(r, 1e-4) * 30.0 / TAU_C * 6.0;
        if (r > radius - 0.014) {
            acc.x += 0.45 * (1.0 - smoothstep(0.0, 0.06 + ang_aa, minor));
        }
        acc.x += 0.7 * (1.0 - smoothstep(0.0, 0.035 + ang_aa, major));
    }

    // Sweep tape from zero to the needle, brightening toward the needle end
    // so it reads as motion rather than as a painted band.
    if (r < radius - 0.020 && r > radius - 0.050 && in_sweep && theta < needle_theta) {
        let along = (theta + SWEEP_HALF) / max(needle_theta + SWEEP_HALF, 1e-4);
        acc.x += 0.22 + 0.30 * along * along;
    }

    let dir = vec2<f32>(sin(needle_theta), cos(needle_theta));
    let nd = seg_dist(p, dir * 0.032, dir * (radius - 0.006));
    acc.x += 0.8 * (1.0 - smoothstep(0.0, 0.010, nd));
    acc.y += 1.0 - smoothstep(0.0, aa * 1.8, nd - 0.0012);
    acc.x += 0.5 * (1.0 - smoothstep(0.007, 0.011, r));
    return acc;
}

// ------------------------------------------------------------------ tapes

// Vertical bar growing from the bottom: thrust demand. `over` lights the
// bar's overshoot band, which is what boost looks like on an instrument
// whose scale stops at "everything the engine has".
fn tape_unipolar(p: vec2<f32>, frac: f32, over: f32, aa: f32) -> vec2<f32> {
    var acc = vec2<f32>(0.0);
    let outline = abs(box_dist(p, vec2<f32>(TAPE_W, TAPE_H)));
    acc.x += 0.5 * (1.0 - smoothstep(0.0, aa * 1.6, outline - 0.0014));

    let top = -TAPE_H + 2.0 * TAPE_H * clamp(frac, 0.0, 1.0);
    if (abs(p.x) < TAPE_W - 0.006 && p.y < top && p.y > -TAPE_H + 0.005) {
        let along = (p.y + TAPE_H) / max(top + TAPE_H, 1e-4);
        acc.x += 0.30 + 0.45 * along * along + 0.5 * over;
    }
    // Graduations every fifth of scale.
    let g = abs(fract((p.y + TAPE_H) / (2.0 * TAPE_H) * 5.0) - 0.5);
    if (abs(p.x) > TAPE_W - 0.012) {
        acc.x += 0.35 * (1.0 - smoothstep(0.0, 0.05, g));
    }
    return acc;
}

// Vertical bar growing from the centre, either way: vertical speed. Centre
// zero is the whole point — a climb and a descent must not look alike.
fn tape_bipolar(p: vec2<f32>, frac: f32, aa: f32) -> vec2<f32> {
    var acc = vec2<f32>(0.0);
    let outline = abs(box_dist(p, vec2<f32>(TAPE_W, TAPE_H)));
    acc.x += 0.5 * (1.0 - smoothstep(0.0, aa * 1.6, outline - 0.0014));

    // Datum: the zero line, brighter than the frame it crosses.
    acc.y += 0.55 * (1.0 - smoothstep(0.0, aa * 1.8, abs(p.y) - 0.0012));
    acc.x += 0.5 * (1.0 - smoothstep(0.0, 0.006, abs(p.y)));

    let end = TAPE_H * clamp(frac, -1.0, 1.0);
    if (abs(p.x) < TAPE_W - 0.006 && p.y * sign(end) > 0.0 && abs(p.y) < abs(end)) {
        acc.x += 0.30 + 0.55 * abs(p.y / max(abs(end), 1e-4));
    }
    // Decade marks: this tape is logarithmic, so the graduations are too.
    let g = abs(fract((p.y / TAPE_H) * 3.0) - 0.5);
    if (abs(p.x) > TAPE_W - 0.012) {
        acc.x += 0.35 * (1.0 - smoothstep(0.0, 0.06, g));
    }
    return acc;
}

// ---------------------------------------------------------------- compass

// Heading ribbon: a strip of the compass rose sliding under a fixed caret.
// Labels are placed by finding the nearest 30-degree mark to this pixel and
// asking where it sits, which draws every visible label without a loop over
// all twelve.
fn compass(p: vec2<f32>, heading_rad: f32, aa: f32) -> vec2<f32> {
    var acc = vec2<f32>(0.0);
    // Shell units per degree: 120 degrees of rose across the full ribbon.
    let k = HDG_HW / 60.0;
    let hdg_deg = heading_rad * 57.2957795;
    let under = hdg_deg + p.x / k;

    // Frame: two rules rather than a box — a closed panel would read as glass.
    acc.x += 0.35 * (1.0 - smoothstep(0.0, aa * 1.6, abs(abs(p.y) - HDG_HH) - 0.0012));

    let minor = abs(fract(under / 5.0) - 0.5) * 5.0 * k;
    let major = abs(fract(under / 15.0) - 0.5) * 15.0 * k;
    if (p.y < 0.0) {
        acc.x += 0.45 * (1.0 - smoothstep(0.0, aa * 1.6, minor - 0.0010))
            * (1.0 - smoothstep(0.0, 0.004, abs(p.y) - 0.020));
        acc.x += 0.75 * (1.0 - smoothstep(0.0, aa * 1.8, major - 0.0014))
            * (1.0 - smoothstep(0.0, 0.004, abs(p.y) - 0.038));
    }

    // Nearest label, in tens of degrees the way a compass card is marked.
    let label = round(under / 30.0) * 30.0;
    let lx = (label - hdg_deg) * k;
    var tens = i32(round(label / 10.0)) % 36;
    if (tens < 0) {
        tens += 36;
    }
    acc += number(p - vec2<f32>(lx, 0.028), f32(tens), 2u, 0.010, 0.018, 0.030, aa);

    // Caret: the ship's own heading, fixed at the centre.
    let c = abs(p.x) + max(p.y - HDG_HH, 0.0) * 0.0;
    let caret = seg_dist(p, vec2<f32>(-0.014, HDG_HH + 0.022), vec2<f32>(0.0, HDG_HH + 0.004))
        * step(0.0, p.y);
    let caret2 = seg_dist(p, vec2<f32>(0.014, HDG_HH + 0.022), vec2<f32>(0.0, HDG_HH + 0.004));
    acc.y += 0.9 * (1.0 - smoothstep(0.0, aa * 1.8, min(caret, caret2) - 0.0014));
    acc.x += 0.0 * c;
    return acc;
}

// ------------------------------------------------------------------- holo

// The ship, in body frame: +X right, +Y up, -Z forward (the nose), matching
// the simulation's convention exactly so a wing drawn here is the wing the
// physics is flying.
const SHIP_V: array<vec3<f32>, 17> = array<vec3<f32>, 17>(
    vec3<f32>(0.00, 0.00, -1.00),   //  0 nose
    vec3<f32>(0.00, -0.10, -0.55),  //  1 chin
    vec3<f32>(0.00, 0.12, -0.52),   //  2 canopy front
    vec3<f32>(0.00, 0.16, -0.10),   //  3 canopy back
    vec3<f32>(0.00, 0.08, 0.62),    //  4 spine end
    vec3<f32>(0.00, -0.12, 0.58),   //  5 belly end
    vec3<f32>(-0.18, 0.00, -0.34),  //  6 hip L
    vec3<f32>(0.18, 0.00, -0.34),   //  7 hip R
    vec3<f32>(-0.24, 0.00, 0.30),   //  8 shoulder L
    vec3<f32>(0.24, 0.00, 0.30),    //  9 shoulder R
    vec3<f32>(-0.66, -0.03, 0.16),  // 10 wingtip L front
    vec3<f32>(0.66, -0.03, 0.16),   // 11 wingtip R front
    vec3<f32>(-0.78, -0.03, 0.62),  // 12 wingtip L back
    vec3<f32>(0.78, -0.03, 0.62),   // 13 wingtip R back
    vec3<f32>(0.00, 0.46, 0.58),    // 14 fin top
    vec3<f32>(-0.15, 0.00, 0.72),   // 15 engine L
    vec3<f32>(0.15, 0.00, 0.72),    // 16 engine R
);

const SHIP_E: array<vec2<u32>, 29> = array<vec2<u32>, 29>(
    vec2<u32>(0u, 2u), vec2<u32>(2u, 3u), vec2<u32>(3u, 4u),
    vec2<u32>(0u, 1u), vec2<u32>(1u, 5u), vec2<u32>(5u, 4u),
    vec2<u32>(0u, 6u), vec2<u32>(0u, 7u),
    vec2<u32>(6u, 8u), vec2<u32>(7u, 9u),
    vec2<u32>(8u, 4u), vec2<u32>(9u, 4u),
    vec2<u32>(6u, 1u), vec2<u32>(7u, 1u),
    vec2<u32>(8u, 5u), vec2<u32>(9u, 5u),
    vec2<u32>(6u, 10u), vec2<u32>(10u, 12u), vec2<u32>(12u, 8u),
    vec2<u32>(7u, 11u), vec2<u32>(11u, 13u), vec2<u32>(13u, 9u),
    vec2<u32>(3u, 14u), vec2<u32>(14u, 4u),
    vec2<u32>(4u, 15u), vec2<u32>(4u, 16u), vec2<u32>(5u, 15u), vec2<u32>(5u, 16u),
    vec2<u32>(15u, 16u),
);

// Orthographic chase projection. The view sits behind and 20 degrees above
// the ship's own heading, so the hologram shows pitch and bank while heading
// lives on the compass — the same division of labour a helicopter panel makes,
// and the reason the hull does not spin on the spot when the pilot turns.
fn holo_project(p: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(p.x, p.y * SIN_PHI + p.z * COS_PHI);
}

// Depth along the view axis, 0 near, 1 far. Only used to dim the far side of
// the hull: a wireframe with no depth cue is an unreadable tangle of lines.
fn holo_depth(p: vec3<f32>) -> f32 {
    return clamp((p.y * COS_PHI - p.z * SIN_PHI) * 0.5 + 0.5, 0.0, 1.0);
}

fn attitude_holo(p: vec2<f32>, aa: f32) -> vec2<f32> {
    var acc = vec2<f32>(0.0);
    let scale = HOLO_R / 1.16;
    let q = p / scale;
    let aaq = aa / scale;
    let flags = u32(cl.att.w);

    // The local horizontal plane, seen edge-on-ish: a unit circle in that
    // plane projects to an ellipse of semi-minor axis sin(phi). This is the
    // horizon the hull banks against.
    let ring = ellipse_ring(q, 1.0, SIN_PHI);
    acc.x += 0.55 * (1.0 - smoothstep(0.0, aaq * 2.0, ring - 0.010));

    // Cardinal marks on that ring. North is doubled rather than lettered:
    // the compass overhead already carries the number, and a glyph this small
    // would be a smudge.
    for (var k = 0u; k < 4u; k += 1u) {
        let psi = f32(k) * 1.57079633 - cl.att.x;
        let dir = vec3<f32>(sin(psi), cos(psi), 0.0);
        let a = holo_project(dir);
        let b = holo_project(dir * 1.16);
        let d = seg_dist(q, a, b);
        let bright = select(0.5, 1.0, k == 0u);
        acc.x += bright * (1.0 - smoothstep(0.0, aaq * 2.0, d - 0.014));
        if (k == 0u) {
            let d2 = seg_dist(q, holo_project(dir * 1.20), holo_project(dir * 1.34));
            acc.y += 0.7 * (1.0 - smoothstep(0.0, aaq * 2.0, d2 - 0.012));
        }
    }

    // The hull. Every edge is a capsule; brightness falls with depth so the
    // far side reads as the far side.
    let m0 = cl.m0.xyz;
    let m1 = cl.m1.xyz;
    let m2 = cl.m2.xyz;
    for (var e = 0u; e < 29u; e += 1u) {
        let idx = SHIP_E[e];
        let va = SHIP_V[idx.x];
        let vb = SHIP_V[idx.y];
        let wa = m0 * va.x + m1 * va.y + m2 * va.z;
        let wb = m0 * vb.x + m1 * vb.y + m2 * vb.z;
        let sa = holo_project(wa);
        let sb = holo_project(wb);
        let d = seg_dist(q, sa, sb);
        if (d > 0.10) {
            continue;
        }
        let fade = mix(1.0, 0.40, holo_depth((wa + wb) * 0.5));
        acc.x += 0.85 * fade * (1.0 - smoothstep(0.0, aaq * 1.8, d - 0.009));
        acc.y += 0.45 * fade * (1.0 - smoothstep(0.0, aaq * 1.6, d - 0.0035));
    }

    // Prograde: where the ship is actually going, against where it is
    // pointing. With the flight computer on and the burn settled these sit on
    // top of each other, which is the clearest statement of what it does.
    if (cl.prograde.w > 0.5) {
        let pg = holo_project(cl.prograde.xyz);
        let rel = q - pg;
        let rr = length(rel);
        acc.y += 0.8 * (1.0 - smoothstep(0.0, aaq * 2.0, abs(rr - 0.075) - 0.008));
        for (var t = 0u; t < 3u; t += 1u) {
            let ang = f32(t) * 2.0943951 + 1.5707963;
            let dir = vec2<f32>(cos(ang), sin(ang));
            let d = seg_dist(rel, dir * 0.075, dir * 0.125);
            acc.y += 0.7 * (1.0 - smoothstep(0.0, aaq * 2.0, d - 0.008));
        }
    }

    // Flight-computer lamp, under the hull. X toggles a mode that changes how
    // the ship answers entirely, and until this existed the only confirmation
    // was a log line invisible in fullscreen.
    let lamp = length(q - vec2<f32>(0.0, -1.02)) - 0.030;
    if ((flags & FLAG_FC) != 0u) {
        acc.y += 0.8 * (1.0 - smoothstep(0.0, aaq * 2.0, lamp));
    } else {
        acc.x += 0.6 * (1.0 - smoothstep(0.0, aaq * 2.0, abs(lamp) - 0.006));
    }
    return acc;
}

// ------------------------------------------------------------------- main

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = cl.frame.x;
    if (vis < 0.01) {
        discard;
    }
    let aspect = cl.frame.z;

    // One shell coordinate, one pixel footprint, computed before any
    // branching: derivatives must not be taken inside non-uniform control
    // flow, and every instrument below wants the same two numbers anyway.
    let s = canopy(in.ndc, aspect);
    let aa = max(fwidth(s.x), 1e-5) * 0.9;

    let flags = u32(cl.att.w);
    var glow = 0.0;  // cyan structure
    var hot = 0.0;   // white-hot accents
    var warn = 0.0;  // how far this pixel leans toward the warning palette

    // ---- speed dial, with the structural-load arc riding outside it
    {
        let p = s - SPD_C;
        if (length(p) < DIAL_R * 1.6) {
            let acc = dial(p, DIAL_R, cl.speed_alt.y, aa);
            glow += acc.x;
            hot += acc.y;
            warn = max(warn, smoothstep(0.85, 1.0, cl.speed_alt.y));

            let r = length(p);
            let theta = atan2(p.x, p.y);
            let g = clamp(cl.rate.w, 0.0, 1.0);
            let g_theta = -SWEEP_HALF + 2.0 * SWEEP_HALF * g;
            if (abs(r - (DIAL_R + 0.016)) < 0.006 && abs(theta) < SWEEP_HALF && theta < g_theta) {
                glow += 0.55;
                warn = max(warn, smoothstep(0.60, 0.85, g));
            }
            glow += acc.x * 0.0;
            let readout = number(p - vec2<f32>(0.0, -0.068), cl.speed_alt.x, 3u, 0.014, 0.026, 0.046, aa);
            glow += readout.x;
            hot += readout.y;
        }
    }

    // ---- altitude dial: four decades of logarithmic scale, exact metres
    {
        let p = s - ALT_C;
        if (length(p) < DIAL_R * 1.6) {
            let acc = dial(p, DIAL_R, cl.speed_alt.w, aa);
            glow += acc.x;
            hot += acc.y;
            let readout = number(p - vec2<f32>(0.0, -0.068), cl.speed_alt.z, 5u, 0.011, 0.022, 0.036, aa);
            glow += readout.x;
            hot += readout.y;
            // Below a hundred metres the altimeter is the instrument that
            // matters; say so before the ground does.
            warn = max(warn, (1.0 - smoothstep(20.0, 120.0, cl.speed_alt.z)) * step(-0.5, cl.rate.x * -1.0));
        }
    }

    // ---- thrust tape
    {
        let p = s - THR_C;
        if (abs(p.x) < TAPE_W * 2.0 && abs(p.y) < TAPE_H * 1.3) {
            let over = select(0.0, 1.0, (flags & FLAG_BOOST) != 0u);
            let acc = tape_unipolar(p, cl.rate.z, over * cl.rate.z, aa);
            glow += acc.x;
            hot += acc.y;
            warn = max(warn, over * smoothstep(0.5, 1.0, cl.rate.z));
        }
    }

    // ---- vertical-speed tape
    {
        let p = s - VSI_C;
        if (abs(p.x) < TAPE_W * 2.0 && abs(p.y) < TAPE_H * 1.3) {
            let acc = tape_bipolar(p, cl.rate.y, aa);
            glow += acc.x;
            hot += acc.y;
        }
    }

    // ---- vertical-speed readout, under its tape, with a sign bar
    {
        let p = s - (VSI_C + vec2<f32>(0.0, -TAPE_H - 0.045));
        if (abs(p.x) < 0.09 && abs(p.y) < 0.04) {
            let v = abs(cl.rate.x);
            let acc = number(p, v, 3u, 0.010, 0.020, 0.034, aa);
            glow += acc.x;
            hot += acc.y;
            if (cl.rate.x < -0.05) {
                let d = seg_dist(p, vec2<f32>(-0.068, 0.0), vec2<f32>(-0.050, 0.0));
                hot += 0.9 * (1.0 - smoothstep(0.0, aa * 1.8, d - 0.0016));
            }
        }
    }

    // ---- compass ribbon
    {
        let p = s - HDG_C;
        if (abs(p.x) < HDG_HW && abs(p.y) < HDG_HH + 0.032) {
            let acc = compass(p, cl.att.x, aa);
            glow += acc.x;
            hot += acc.y;
        }
    }

    // ---- attitude hologram
    {
        let p = s - HOLO_C;
        if (length(p) < HOLO_R * 1.30) {
            let acc = attitude_holo(p, aa);
            glow += acc.x;
            hot += acc.y;
        }
    }

    if (glow + hot < 0.002) {
        discard;
    }

    // Palette: hologram cyan, shifting toward warning amber where an
    // instrument has something to say.
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    let tint = mix(cyan, amber, clamp(warn, 0.0, 1.0));

    // Scanlines: static spatial modulation — hologram texture without
    // temporal noise (P1: no shimmer, no smear).
    let scan = 0.90 + 0.10 * sin(in.ndc.y * cl.frame.w * 1.7);

    // The glass dims toward the rim: light hitting the canopy obliquely reads
    // fainter, which sells the shell harder than the distortion does.
    let rim = length(vec2<f32>(in.ndc.x * aspect, in.ndc.y));
    let glass = 1.0 - 0.38 * smoothstep(0.75, 1.45, rim);

    let colour = (tint * glow + vec3<f32>(1.0, 1.0, 1.0) * hot * 0.9) * scan * glass * vis;

    // Additive blend: what is black costs nothing and shows nothing.
    return vec4<f32>(colour, 1.0);
}
