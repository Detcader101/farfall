// gauge.wgsl — holographic velocity gauge (SPEC §6.5, pass: gauge)
//
// Lane: A. Cost class: trivial (SDF arithmetic over a small screen region;
// pixels outside the gauge's disc exit after one distance check).
//
// A generic arc instrument: the same SDF drawing serves velocity and
// altitude (and whatever joins the cluster next) — an instrument is data,
// not code. Drawn entirely as
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
    // x: arc value, y: visibility 0..1, z: time (s), w: aspect (w/h)
    a: vec4<f32>,
    // x: arc full-scale value, y: target height in px, zw: anchor in NDC
    b: vec4<f32>,
    // x: readout digits (0..999), y: decimal dot after digit 1|2 (0: none),
    // z: warning sense — 0 warns at the TOP of the arc (overspeed), 1 warns
    // at the BOTTOM (low altitude). w: unused.
    c: vec4<f32>,
    // xy: hologram sway (canopy units), z: mach-alert flash 0..1,
    // w: mach number (negative: no mach readout on this instrument).
    d: vec4<f32>,
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
// The canopy warp itself lives in common.wgsl: one function, one piece of
// glass, shared with every other HUD pass.
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
    // The barrier flash can surface a faded instrument: an alert on an
    // invisible gauge would be a sound with no source.
    let alert = clamp(gauge.d.z, 0.0, 1.0);
    let vis = max(gauge.a.y, alert);
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

    // Early out: everything (shock ring included) lives inside this.
    if (length(p) > radius * 1.5 + 0.06) {
        discard;
    }

    // Depth layers. The instrument is not a decal: the dial face sits at the
    // back, the needle floats in front of it, the readout floats nearest the
    // pilot — and the sway vector (hologram inertia, from Rust) displaces
    // each layer by its depth. Under rotation the layers disagree slightly,
    // and that disagreement is parallax: flat SDFs become a thing with
    // shape. At rest all three collapse to the same place.
    let sway = gauge.d.xy;
    let p_face = p - sway * 0.18;
    let p_mid = p - sway * 0.55;
    let p_near = p - sway * 1.0;
    // Static extrusion offset: every floating element casts a dim second
    // image a hair down-right, like the other face of a thick pane.
    let extrude = vec2<f32>(0.0032, -0.0032);

    // Pixel footprint for AA, in gauge units.
    let aa = max(fwidth(p.x), 1e-5) * 0.9;

    let value = max(gauge.a.x, 0.0);
    let full = max(gauge.b.x, 1.0);
    let frac = clamp(value / full, 0.0, 1.0);
    let mach = gauge.d.w;

    // Dial-face polar frame.
    let theta = atan2(p_face.x, p_face.y);
    let r = length(p_face);
    let in_sweep = abs(theta) < SWEEP_HALF;
    // Mid-layer polar frame, for the moving parts.
    let theta_m = atan2(p_mid.x, p_mid.y);
    let r_m = length(p_mid);
    let in_sweep_m = abs(theta_m) < SWEEP_HALF;
    let needle_theta = -SWEEP_HALF + 2.0 * SWEEP_HALF * frac;

    var glow = 0.0;      // cyan structure
    var hot = 0.0;       // white-hot accents (needle core, digits)
    var warn_glow = 0.0; // always-amber marks (mach tick, shock ring)

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

    // The sound barrier, marked on the dial: an amber gate tick across the
    // ring where mach 1 lives on this scale. Speed gauge only (c.z = 0), and
    // only when a mach number exists at all — 340 m/s here must match
    // MACH1_MPS in the app, which owns the "in atmosphere" gate.
    if (gauge.c.z < 0.5 && mach >= 0.0) {
        let mfrac = 340.0 / full;
        let mth = -SWEEP_HALF + 2.0 * SWEEP_HALF * mfrac;
        let mdir = vec2<f32>(sin(mth), cos(mth));
        let md = seg_dist(p_face, mdir * (radius - 0.026), mdir * (radius + 0.010));
        warn_glow += 0.85 * (1.0 - smoothstep(0.0, aa * 1.8 + 0.0012, md - 0.0014));
    }

    // Sweep fill: a translucent band from zero to the needle — the "tape".
    if (r_m < radius - 0.020 && r_m > radius - 0.052 && in_sweep_m && theta_m < needle_theta) {
        // Brighter toward the needle end, so the tape reads as motion.
        let along = (theta_m + SWEEP_HALF) / max(needle_theta + SWEEP_HALF, 1e-4);
        glow += 0.22 + 0.30 * along * along;
    }

    // Needle: capsule from hub to ring at the value angle, hot core, and a
    // dim extruded twin behind it — the needle has thickness.
    let dir = vec2<f32>(sin(needle_theta), cos(needle_theta));
    let nd = seg_dist(p_mid, dir * 0.035, dir * (radius - 0.006));
    let nd_ghost = seg_dist(p_mid + extrude, dir * 0.035, dir * (radius - 0.006));
    glow += 0.8 * (1.0 - smoothstep(0.0, 0.010, nd));
    glow += 0.22 * (1.0 - smoothstep(0.0, 0.006, nd_ghost));
    hot += 1.0 - smoothstep(0.0, aa * 1.8, nd - 0.0012);

    // Hub dot.
    glow += 0.5 * (1.0 - smoothstep(0.008, 0.012, r_m));

    // Seven-segment readout, three digits, centred in the lower gap: the
    // nearest layer, with the same extruded second face.
    {
        let dh = 0.030;
        let dw = 0.016;
        let pitch = 0.052;
        let base = vec2<f32>(0.0, -0.075);
        let shown = u32(clamp(round(gauge.c.x), 0.0, 999.0));
        let digits = array<u32, 3>(shown / 100u, (shown / 10u) % 10u, shown % 10u);
        // Leading zeros stay lit: instruments read as instruments.
        for (var i = 0u; i < 3u; i += 1u) {
            let off = base + vec2<f32>((f32(i) - 1.0) * pitch, 0.0);
            let dd = digit_dist(p_near - off, digit_mask(digits[i]), dw, dh);
            let dg = digit_dist(p_near + extrude - off, digit_mask(digits[i]), dw, dh);
            hot += 0.9 * (1.0 - smoothstep(0.0, aa * 1.8, dd - 0.0018));
            glow += 0.25 * (1.0 - smoothstep(0.0, 0.008, dd));
            glow += 0.16 * (1.0 - smoothstep(0.0, 0.004, dg));
        }
        // Decimal dot: auto-ranging readouts (altitude in km) park it after
        // digit 1 or 2; a dot at the baseline between cells.
        let dot_after = gauge.c.y;
        if (dot_after > 0.5) {
            let dot_pos = base + vec2<f32>((dot_after - 1.5) * pitch, -dh);
            let dr = length(p_near - dot_pos);
            hot += 0.9 * (1.0 - smoothstep(0.004, 0.004 + aa * 1.8, dr));
            glow += 0.25 * (1.0 - smoothstep(0.004, 0.012, dr));
        }
    }

    // Mach readout: two small digits and a dot under the main readout,
    // fading in as the barrier becomes a live concern. Absent entirely when
    // mach is meaningless (negative: no atmosphere).
    if (gauge.c.z < 0.5 && mach >= 0.0) {
        let mfade = smoothstep(0.30, 0.45, mach);
        if (mfade > 0.001) {
            let dh = 0.017;
            let dw = 0.009;
            let pitch = 0.030;
            let base = vec2<f32>(0.0, -0.134);
            let m10 = u32(clamp(round(mach * 10.0), 0.0, 99.0));
            let md = array<u32, 2>(m10 / 10u, m10 % 10u);
            // Past the barrier the mach digits go amber: supersonic is a
            // state, not a number.
            let m_amber = step(1.0, mach);
            for (var i = 0u; i < 2u; i += 1u) {
                let off = base + vec2<f32>((f32(i) - 0.5) * pitch, 0.0);
                let dd = digit_dist(p_near - off, digit_mask(md[i]), dw, dh);
                let lit = mfade * (1.0 - smoothstep(0.0, aa * 1.8, dd - 0.0012));
                hot += 0.55 * lit * (1.0 - m_amber);
                warn_glow += 1.1 * lit * m_amber;
                glow += 0.15 * mfade * (1.0 - smoothstep(0.0, 0.006, dd));
            }
            let dot_pos = base + vec2<f32>(0.0, -dh);
            let dr = length(p_near - dot_pos);
            let dlit = mfade * (1.0 - smoothstep(0.003, 0.003 + aa * 1.8, dr));
            hot += 0.55 * dlit * (1.0 - m_amber);
            warn_glow += 1.1 * dlit * m_amber;
        }
    }

    // The barrier flash: a shock ring that leaves the dial and expands as it
    // fades — the visual twin of the boom, fired by the same edge.
    if (alert > 0.01) {
        let rr = radius * (1.0 + (1.0 - alert) * 0.55);
        let width = 0.004 + 0.020 * (1.0 - alert);
        let sd = abs(length(p_face) - rr);
        warn_glow += 2.2 * alert * alert * (1.0 - smoothstep(0.0, width, sd));
    }

    // Palette: hologram cyan, shifting toward warning amber at the hot end
    // of the arc — the top for a speed gauge (overspeed), the bottom for an
    // altimeter (ground coming up).
    let warn_high = smoothstep(0.85, 1.0, frac);
    let warn_low = 1.0 - smoothstep(0.04, 0.14, frac);
    let warning = mix(warn_high, warn_low, clamp(gauge.c.z, 0.0, 1.0));
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    let tint = mix(cyan, amber, warning);

    // Scanlines: static spatial modulation — hologram texture without
    // temporal noise (P1: no shimmer, no smear).
    let scan = 0.90 + 0.10 * sin(in.ndc.y * gauge.b.y * 1.7);

    let glass = canopy_glass(in.ndc, aspect);

    // The whole instrument surges with the flash, then settles.
    let surge = 1.0 + 1.1 * alert * alert;
    var colour = (tint * glow + vec3<f32>(1.0, 1.0, 1.0) * hot * 0.9 + amber * warn_glow)
        * scan * glass * vis * surge;

    // Additive blend: what is black costs nothing and shows nothing.
    return vec4<f32>(colour, 1.0);
}
