// gauge.wgsl — holographic velocity gauge (SPEC §6.5, pass: gauge)
//
// Lane: A. Cost class: trivial (SDF arithmetic over a small screen region;
// pixels outside the gauge's disc exit after one distance check).
//
// The first cockpit instrument: a Tron-style arc gauge, drawn entirely as
// signed-distance fields — arc ring, tick ladder, sweep fill, needle, and a
// three-digit seven-segment readout. Additively blended, so it reads as light
// projected on air rather than a panel: there is no background, no frame, no
// quad — where the gauge is dark it simply does not exist.
//
// Visibility is a uniform driven by flight state (render/src/gauge.rs): the
// instrument surfaces when speed is high or changing and fades to nothing in
// settled flight — holograms appear when relevant, which is what makes a
// cockpit feel seamless instead of cluttered.

struct Gauge {
    // x: speed (m/s), y: visibility 0..1, z: time (s), w: aspect (w/h)
    a: vec4<f32>,
    // x: full-scale speed, y: target height in px, zw: cluster anchor in NDC
    b: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gauge: Gauge;

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

const TAU_G: f32 = 6.28318531;
// Canopy radius, in aspect-corrected screen units. Smaller bends harder.
const CANOPY_R: f32 = 1.55;

// The canopy projection: the HUD is not painted on the screen, it is painted
// on the inside of a spherical shell in front of the pilot, and the screen
// shows that shell in perspective. An equidistant fisheye mapping of screen
// coordinates gives exactly that read: elements near the centre are almost
// flat, elements toward the rim stretch and bow as the shell curves away.
// Every instrument passes through this one function, so the whole future
// cluster shares a single piece of glass.
fn canopy(ndc: vec2<f32>, aspect: f32) -> vec2<f32> {
    let v = vec2<f32>(ndc.x * aspect, ndc.y);
    let r = length(v);
    if (r < 1e-4) {
        return v;
    }
    let x = clamp(r / CANOPY_R, 0.0, 0.999);
    return v * (asin(x) / x);
}
// Sweep: 0 m/s at 7 o'clock, full scale at 5 o'clock — 240 degrees of arc
// with the gap at the bottom, jet-gauge fashion. Angles measured from
// 12 o'clock, clockwise positive.
const SWEEP_HALF: f32 = 2.0943951; // 120 degrees

// Capsule SDF: distance to segment ab, for needle and digit segments.
fn seg_dist(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

// Seven-segment digit: bit i of `mask` lights segment i.
// Segments: 0 top, 1 top-right, 2 bottom-right, 3 bottom, 4 bottom-left,
// 5 top-left, 6 middle. Cell is 2w wide, 2h tall, centred on origin.
fn digit_dist(p: vec2<f32>, mask: u32, w: f32, h: f32) -> f32 {
    var d = 1e9;
    let ends: array<vec4<f32>, 7> = array<vec4<f32>, 7>(
        vec4<f32>(-w, h, w, h),    // top
        vec4<f32>(w, h, w, 0.0),   // top-right
        vec4<f32>(w, 0.0, w, -h),  // bottom-right
        vec4<f32>(-w, -h, w, -h),  // bottom
        vec4<f32>(-w, 0.0, -w, -h),// bottom-left
        vec4<f32>(-w, h, -w, 0.0), // top-left
        vec4<f32>(-w, 0.0, w, 0.0),// middle
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
    // 0..9 in the segment numbering above.
    let masks = array<u32, 10>(63u, 6u, 91u, 79u, 102u, 109u, 125u, 7u, 127u, 111u);
    return masks[min(n, 9u)];
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = gauge.a.y;
    if (vis < 0.01) {
        discard;
    }
    let aspect = gauge.a.w;

    // Both the pixel and the cluster anchor live on the canopy: the gauge is
    // wherever the shell puts it, and its shape inherits the shell's local
    // curvature — slightly bowed at screen edges, flat near centre.
    let anchor = gauge.b.zw;
    let p = canopy(in.ndc, aspect) - canopy(anchor, aspect);
    let radius = 0.155;

    // Early out: everything lives inside 1.5 radii.
    if (length(p) > radius * 1.5) {
        discard;
    }

    // Pixel footprint for AA, in gauge units.
    let aa = max(fwidth(p.x), 1e-5) * 0.9;

    let speed = max(gauge.a.x, 0.0);
    let full = max(gauge.b.x, 1.0);
    let frac = clamp(speed / full, 0.0, 1.0);

    // Angle from 12 o'clock, clockwise, in [-pi, pi].
    let theta = atan2(p.x, p.y);
    let r = length(p);
    let in_sweep = abs(theta) < SWEEP_HALF;
    let needle_theta = -SWEEP_HALF + 2.0 * SWEEP_HALF * frac;

    var glow = 0.0;      // cyan structure
    var hot = 0.0;       // white-hot accents (needle core, digits)

    // Outer ring: a thin bright arc with a fainter halo ring outside it.
    if (in_sweep) {
        let ring = abs(r - radius);
        glow += 0.9 * (1.0 - smoothstep(0.0, aa * 1.6, ring - 0.0016));
        glow += 0.18 * (1.0 - smoothstep(0.0, 0.012, ring));
    }

    // Tick ladder: minor every 1/30 of scale, major every 1/6. Ticks are
    // angular slots cut radially inward from the ring.
    if (in_sweep && r < radius && r > radius - 0.030) {
        let t = (theta + SWEEP_HALF) / (2.0 * SWEEP_HALF); // 0..1 along sweep
        let minor = abs(fract(t * 30.0) - 0.5);
        let major = abs(fract(t * 6.0) - 0.5);
        let ang_aa = aa / max(r, 1e-4) * 30.0 / TAU_G * 6.0;
        if (r > radius - 0.014) {
            glow += 0.45 * (1.0 - smoothstep(0.0, 0.06 + ang_aa, minor));
        }
        glow += 0.7 * (1.0 - smoothstep(0.0, 0.035 + ang_aa, major));
    }

    // Sweep fill: a translucent band from zero to the needle — the "tape".
    if (r < radius - 0.020 && r > radius - 0.052 && in_sweep && theta < needle_theta) {
        // Brighter toward the needle end, so the tape reads as motion.
        let along = (theta + SWEEP_HALF) / max(needle_theta + SWEEP_HALF, 1e-4);
        glow += 0.22 + 0.30 * along * along;
    }

    // Needle: capsule from hub to ring at the speed angle, with a hot core.
    let dir = vec2<f32>(sin(needle_theta), cos(needle_theta));
    let nd = seg_dist(p, dir * 0.035, dir * (radius - 0.006));
    glow += 0.8 * (1.0 - smoothstep(0.0, 0.010, nd));
    hot += 1.0 - smoothstep(0.0, aa * 1.8, nd - 0.0012);

    // Hub dot.
    glow += 0.5 * (1.0 - smoothstep(0.008, 0.012, r));

    // Seven-segment readout, three digits, centred in the lower gap.
    {
        let dh = 0.030;
        let dw = 0.016;
        let pitch = 0.052;
        let base = vec2<f32>(0.0, -0.075);
        let shown = u32(clamp(round(speed), 0.0, 999.0));
        let digits = array<u32, 3>(shown / 100u, (shown / 10u) % 10u, shown % 10u);
        // Leading zeros stay lit: instruments read as instruments.
        for (var i = 0u; i < 3u; i += 1u) {
            let cell = p - base - vec2<f32>((f32(i) - 1.0) * pitch, 0.0);
            let dd = digit_dist(cell, digit_mask(digits[i]), dw, dh);
            hot += 0.9 * (1.0 - smoothstep(0.0, aa * 1.8, dd - 0.0018));
            glow += 0.25 * (1.0 - smoothstep(0.0, 0.008, dd));
        }
    }

    // Palette: hologram cyan, shifting toward warning amber past 85% scale.
    let overspeed = smoothstep(0.85, 1.0, frac);
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    let tint = mix(cyan, amber, overspeed);

    // Scanlines: static spatial modulation — hologram texture without
    // temporal noise (P1: no shimmer, no smear).
    let scan = 0.90 + 0.10 * sin(in.ndc.y * gauge.b.y * 1.7);

    // The projection dims toward the rim of the glass: light hitting the
    // canopy obliquely reads fainter, which sells the shell more than the
    // distortion does.
    let rim = length(vec2<f32>(in.ndc.x * aspect, in.ndc.y));
    let glass = 1.0 - 0.38 * smoothstep(0.75, 1.45, rim);

    var colour = (tint * glow + vec3<f32>(1.0, 1.0, 1.0) * hot * 0.9) * scan * glass * vis;

    // Additive blend: what is black costs nothing and shows nothing.
    return vec4<f32>(colour, 1.0);
}
