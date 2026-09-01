// gauge.wgsl — the arc instrument: speed, altitude, G (pass: gauge)
//
// Lane: A. Cost class: trivial (SDF arithmetic over a small screen region;
// pixels outside the gauge's disc exit after one distance check).
//
// A generic arc instrument: the same SDF drawing serves velocity, altitude
// and load — an instrument is data, not code. Drawn entirely as signed
// distance fields — arc ring, tick ladder, needle, a three-digit
// seven-segment readout, the range multiplier — and every stroke is a
// constant-width line with one pixel of anti-aliasing, so it is as crisp
// at 2880×1800 as at 800×600.
//
// Two natures, one drawing. On the glass it is a hologram: additive light,
// cyan structure with a soft halo, white-hot needle and digits, scanlines,
// a sweep tape. On the dash (DIAL, WARTHOG) it is a steam gauge: printed
// markings on a black plate — no halo, no tape, a tapered pointer, a red
// arc painted on the scale where the warning is — lit by nothing but the
// cabin. Where the gauge is dark it simply does not exist.
//
// Visibility is a uniform driven by flight state (render/src/gauge.rs): the
// instrument surfaces when speed is high or changing and fades to nothing in
// settled flight — holograms appear when relevant, which is what makes a
// cockpit feel seamless instead of cluttered.

struct Gauge {
    // The glass numbers (64 bytes), then the placement for a DIAL on the
    // dash: p0 right, p1 up, p2 fwd (w: tan half fov) of the head in ship
    // frame, p3 the dial's centre (w: metres per drawing unit; 0 means the
    // instrument is on the glass, not on the dash).
    // x: arc value, y: visibility 0..1, z: time (s), w: aspect (w/h)
    a: vec4<f32>,
    // x: arc full-scale value, y: target height in px, zw: anchor in NDC
    b: vec4<f32>,
    // x: readout digits (0..999) + 1000 × exponent, y: decimal dot after
    // digit 1|2 (0: none), z: warning sense — 0 warns at the TOP of the
    // arc (overspeed), 1 warns at the BOTTOM (low altitude) — plus 2 for
    // JET and 4 for WARTHOG. w: range multiplier, mantissa + 10 × exponent
    // (0: base range, nothing shown).
    c: vec4<f32>,
    // xy: hologram sway (canopy units), z: mach-alert flash 0..1,
    // w: mach number (negative: no mach readout on this instrument).
    d: vec4<f32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
    // x: sideways lean, y: in-plane rotation (radians); zw unused.
    e: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gauge: Gauge;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

// Six vertices: a quad around the instrument, not a fullscreen triangle.
// The instrument lives on the canopy at a known anchor and a known radius,
// so its screen footprint is the inverse canopy projection of that box —
// generous by a margin for the sway and the shock ring, and the fragment
// stage still makes the exact cut. Same output, ~3% of the fragments.
const QUAD_HALF: f32 = 0.30;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let aspect = gauge.a.w;
    let centre = canopy(gauge.b.zw, aspect);
    // A dial on the dash is seen in perspective: a wider quad to be safe.
    // On the glass, its own size (p0.w, 1 = stock).
    let size = select(max(gauge.p0.w, 0.25), 1.8, gauge.p3.w > 0.0);
    let half = QUAD_HALF * size;
    let xy = canopy_inverse(centre + corners[vi] * half, aspect);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

const TAU_G: f32 = 6.28318531;
// The canopy warp itself lives in common.wgsl: one function, one piece of
// glass, shared with every other HUD pass.
// Sweep: 0 at 7 o'clock, full scale at 5 o'clock — 240 degrees of arc
// with the gap at the bottom, jet-gauge fashion. Angles measured from
// 12 o'clock, clockwise positive.
const SWEEP_HALF: f32 = 2.0943951; // 120 degrees
const SWEEP: f32 = 4.1887902;
const RADIUS: f32 = 0.155;

// c.z carries the warning sense (0 high, 1 low) plus 2 for the JET style.
fn sense_of(cz: f32) -> f32 {
    return cz - 2.0 * floor(cz / 2.0);
}

// A crisp stroke: solid within half_w of the distance field, then one
// pixel of ramp. Every line on the instrument is one of these.
fn stroke(d: f32, half_w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, d - half_w);
}

// The × glyph: two crossed strokes in a cell.
fn cross_dist(p: vec2<f32>, s: f32) -> f32 {
    return min(
        seg_dist(p, vec2<f32>(-s, -s), vec2<f32>(s, s)),
        seg_dist(p, vec2<f32>(-s, s), vec2<f32>(s, -s)),
    );
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
    let in_dash = gauge.p3.w > 0.0;
    var p = (canopy(in.ndc, aspect) - canopy(anchor, aspect)) / max(gauge.p0.w, 0.25);
    if (in_dash) {
        // DIAL: the face lies on the dash; map this pixel's ray onto it,
        // through the face's whole orientation (tilt, lean, rotation).
        let duv = dial_plane_uv(in.ndc, aspect, gauge.p0, gauge.p1, gauge.p2, gauge.p3,
                                DIAL_DASH_N, gauge.e.x, gauge.e.y);
        if (duv.z < 0.5) {
            discard;
        }
        p = duv.xy;
    } else {
        // On the glass the hologram is turned any way the pilot likes:
        // tilt and lean foreshorten, the rotation turns plate and
        // markings together (common.wgsl).
        p = dial_glass_uv(p, gauge.p1.w, gauge.e.x, gauge.e.y);
    }
    let radius = RADIUS;

    // Early out: everything (shock ring included) lives inside this.
    if (length(p) > radius * 1.5 + 0.06) {
        discard;
    }

    // c.z packs the warning sense (bit 0), JET (+2) and WARTHOG (+4).
    let warthog = gauge.c.z >= 4.0;
    let cz = gauge.c.z - select(0.0, 4.0, warthog);
    let sense = sense_of(cz);
    let jet = cz >= 2.0;
    let low_warn = sense > 0.5;
    // A steam gauge on the dash: printed markings, no hologram light.
    let steam = in_dash;

    // Depth layers. The instrument is not a decal: the dial face sits at the
    // back, the needle floats in front of it, the readout floats nearest the
    // pilot — and the sway vector (hologram inertia, from Rust) displaces
    // each layer by its depth. Under rotation the layers disagree slightly,
    // and that disagreement is parallax: flat SDFs become a thing with
    // shape. At rest all three collapse to the same place.
    // A dial on the dash does not sway: it is bolted down.
    let sway = select(gauge.d.xy, vec2<f32>(0.0), in_dash);
    let p_face = p - sway * 0.18;
    let p_mid = p - sway * 0.55;
    let p_near = p - sway * 1.0;
    // Static extrusion offset: every floating element casts a dim second
    // image a hair down-right, like the other face of a thick pane.
    let extrude = vec2<f32>(0.0032, -0.0032);

    // Pixel footprint for AA, in gauge units: one pixel of ramp.
    let aa = max(fwidth(p.x), 1e-5) * 1.1;

    let value = max(gauge.a.x, 0.0);
    // Full scale already carries the range: base × m × 10^k, chosen on the
    // CPU so the value always fits. The multiplier on the face says what a
    // lap is worth; the readout, which never lies, carries the number alone.
    let full = max(gauge.b.x, 1e-6);
    let frac = clamp(value / full, 0.0, 1.0);
    let packed = u32(max(round(gauge.c.w), 0.0));
    let mult_m = packed % 10u;
    let mult_e = packed / 10u;
    let mach = gauge.d.w;

    // Dial-face polar frame.
    let theta = atan2(p_face.x, p_face.y);
    let r = length(p_face);
    let in_sweep = abs(theta) < SWEEP_HALF;
    // Mid-layer polar frame, for the moving parts.
    let theta_m = atan2(p_mid.x, p_mid.y);
    let r_m = length(p_mid);
    let in_sweep_m = abs(theta_m) < SWEEP_HALF;
    let needle_theta = -SWEEP_HALF + SWEEP * frac;

    var glow = 0.0;      // crisp structure, in the face colour
    var halo = 0.0;      // soft hologram light around it (none on a steam gauge)
    var hot = 0.0;       // white-hot accents (needle core)
    var ro = 0.0;        // the readout's digits
    var warn_glow = 0.0; // always-warning-coloured marks (mach tick, red arc, ×m)

    // Outer ring: a thin crisp arc with a fainter halo outside it.
    if (in_sweep) {
        let ring = abs(r - radius);
        glow += stroke(ring, 0.0018, aa);
        halo += 0.18 * (1.0 - smoothstep(0.0, 0.012, ring));
    }

    // Tick ladder: minor every 1/30 of scale, major every 1/6, at 0 and at
    // full scale inclusive — so a major sits under every mach bar. Radial
    // strokes of constant width: the angular distance to the nearest tick
    // times the radius is arc length, which is what a pixel measures.
    if (in_sweep && r < radius - 0.0015 && r > radius - 0.034) {
        let t = (theta + SWEEP_HALF) / SWEEP; // 0..1 along the sweep
        let minor_d = abs(fract(t * 30.0 + 0.5) - 0.5) / 30.0 * SWEEP * r;
        let major_d = abs(fract(t * 6.0 + 0.5) - 0.5) / 6.0 * SWEEP * r;
        if (r > radius - 0.016) {
            glow += 0.75 * stroke(minor_d, 0.0011, aa);
        }
        glow += stroke(major_d, 0.0021, aa);
    }

    // The warning arc, painted on the scale outside the ring: the top of
    // the base range for a speed or G gauge (overspeed, the hull's limit),
    // the bottom for an altimeter (the ground coming up). A fixed mark on
    // the dial, as on a real instrument: the needle entering it is the
    // warning.
    {
        let w0 = select(0.85, 0.0, low_warn);
        let w1 = select(1.0, 0.14, low_warn);
        let ta0 = -SWEEP_HALF + SWEEP * w0;
        let ta1 = -SWEEP_HALF + SWEEP * w1;
        let ang_aa = aa / max(r, 1e-3);
        let in_band = smoothstep(ta0 - ang_aa, ta0 + ang_aa, theta)
            * (1.0 - smoothstep(ta1 - ang_aa, ta1 + ang_aa, theta));
        let band = stroke(abs(r - (radius + 0.0085)), 0.0035, aa) * in_band;
        warn_glow += select(0.55, 0.9, steam) * band;
    }

    // The sound barrier, marked on the dial: amber bars across the ring at
    // every mach on this lap — mach 1 halfway, mach 2 at the end. Speed
    // gauge only (sense 0), and only when a mach number exists at all —
    // 340 m/s here must match MACH1_MPS in the app, which owns the "in
    // atmosphere" gate. On later laps the same bars mean mach 3 and 4, 5
    // and 6: the multiplier says which.
    // Only while a mach is a readable slice of the dial (to ×5: ten bars).
    if (!low_warn && mach >= 0.0 && full <= 3400.5) {
        for (var m = 1.0; m * 340.0 <= full + 0.5; m += 1.0) {
            let mfrac = m * 340.0 / full;
            let mth = -SWEEP_HALF + SWEEP * mfrac;
            let mdir = vec2<f32>(sin(mth), cos(mth));
            let md = seg_dist(p_face, mdir * (radius - 0.034), mdir * (radius + 0.012));
            warn_glow += stroke(md, 0.0022, aa);
        }
    }

    // Range multiplier: "×m" on the face under the hub, "Ek" beside it
    // once the decades climb — base × m × 10^k, the legend a real dial
    // prints on its face. Warning-coloured, like everything that says
    // "more than the dial". There is no top: the numbers in this game do
    // not end, and neither does the instrument.
    if (packed > 0u) {
        let dh = 0.011;
        let dw = 0.006;
        let pitch = 0.019;
        let tens = mult_e / 10u;
        let ncell = 2u + select(0u, select(2u, 3u, tens > 0u), mult_e > 0u);
        var cell = vec2<f32>(-0.5 * f32(ncell - 1u) * pitch, -0.031);
        var md = cross_dist(p_near - cell, 0.0048);
        cell.x += pitch;
        md = min(md, digit_dist(p_near - cell, digit_mask(mult_m), dw, dh));
        if (mult_e > 0u) {
            cell.x += pitch;
            md = min(md, digit_dist(p_near - cell, 121u, dw, dh)); // E: a, d, e, f, g
            if (tens > 0u) {
                cell.x += pitch;
                md = min(md, digit_dist(p_near - cell, digit_mask(tens), dw, dh));
            }
            cell.x += pitch;
            md = min(md, digit_dist(p_near - cell, digit_mask(mult_e % 10u), dw, dh));
        }
        warn_glow += stroke(md, 0.0014, aa);
        halo += 0.15 * (1.0 - smoothstep(0.0, 0.006, md));
    }

    // Sweep fill: a translucent band from zero to the needle — the "tape"
    // — hologram light only; a printed dial has none.
    if (!steam && r_m < radius - 0.020 && r_m > radius - 0.052 && in_sweep_m && theta_m < needle_theta) {
        // Brighter toward the needle end, so the tape reads as motion.
        let along = (theta_m + SWEEP_HALF) / max(needle_theta + SWEEP_HALF, 1e-4);
        halo += 0.22 + 0.30 * along * along;
    }

    // Needle. On the glass: a capsule of light from the hub to the ring
    // with a hot core and a dim extruded twin behind it — the needle has
    // thickness. On the dash: a tapered pointer, wide at the boss, fine
    // at the tip, with a short tail — a steam gauge's.
    let dir = vec2<f32>(sin(needle_theta), cos(needle_theta));
    if (steam) {
        let tip = radius - 0.010;
        let along = clamp(dot(p_mid, dir) / tip, 0.0, 1.0);
        let nd = seg_dist(p_mid, dir * -0.030, dir * tip);
        let half_w = mix(0.0050, 0.0012, along);
        glow += stroke(nd, half_w, aa);
        // The boss over the tail.
        glow += stroke(r_m, 0.013, aa);
    } else {
        let nd = seg_dist(p_mid, dir * 0.035, dir * (radius - 0.006));
        let nd_ghost = seg_dist(p_mid + extrude, dir * 0.035, dir * (radius - 0.006));
        halo += 0.8 * (1.0 - smoothstep(0.0, 0.010, nd));
        halo += 0.22 * (1.0 - smoothstep(0.0, 0.006, nd_ghost));
        hot += stroke(nd, 0.0013, aa);
        // Hub dot.
        glow += 0.6 * stroke(r_m, 0.009, aa);
    }

    // Seven-segment readout, three digits, centred in the lower gap: the
    // nearest layer, with the same extruded second face on the glass.
    {
        let dh = 0.030;
        let dw = 0.016;
        let pitch = 0.052;
        let base = vec2<f32>(0.0, -0.080);
        let packed_ro = u32(max(round(gauge.c.x), 0.0));
        let shown = packed_ro % 1000u;
        let ro_exp = packed_ro / 1000u;
        let digits = array<u32, 3>(shown / 100u, (shown / 10u) % 10u, shown % 10u);
        let stroke_w = select(0.0019, 0.0024, steam);
        // Leading zeros stay lit: instruments read as instruments.
        var dd = 1e9;
        var dg = 1e9;
        for (var i = 0u; i < 3u; i += 1u) {
            let off = base + vec2<f32>((f32(i) - 1.0) * pitch, 0.0);
            dd = min(dd, digit_dist(p_near - off, digit_mask(digits[i]), dw, dh));
            dg = min(dg, digit_dist(p_near + extrude - off, digit_mask(digits[i]), dw, dh));
        }
        // Decimal dot: auto-ranging readouts (altitude in km) park it after
        // digit 1 or 2; a dot at the baseline between cells.
        let dot_after = gauge.c.y;
        if (dot_after > 0.5) {
            let dot_pos = base + vec2<f32>((dot_after - 1.5) * pitch, -dh);
            dd = min(dd, length(p_near - dot_pos) - 0.0026);
        }
        // The readout's exponent, small, to the right of the digits: past
        // three digits the number keeps its three and says how many more.
        if (ro_exp > 0u) {
            let eh = 0.012;
            let ew = 0.0065;
            let ep = 0.019;
            var cell = base + vec2<f32>(2.0 * pitch - 0.006, -dh + eh);
            var ed = digit_dist(p_near - cell, 121u, ew, eh);
            cell.x += ep;
            let tens = ro_exp / 10u;
            if (tens > 0u) {
                ed = min(ed, digit_dist(p_near - cell, digit_mask(tens), ew, eh));
                cell.x += ep;
            }
            ed = min(ed, digit_dist(p_near - cell, digit_mask(ro_exp % 10u), ew, eh));
            dd = min(dd, ed + 0.0005);
        }
        ro += stroke(dd, stroke_w, aa);
        halo += 0.25 * (1.0 - smoothstep(0.0, 0.008, dd));
        halo += 0.16 * (1.0 - smoothstep(0.0, 0.004, dg));
    }

    // Mach readout: two small digits and a dot under the main readout,
    // fading in as the barrier becomes a live concern. Absent entirely when
    // mach is meaningless (negative: no atmosphere).
    var mach_amber = 0.0;
    if (!low_warn && mach >= 0.0) {
        let mfade = smoothstep(0.30, 0.45, mach);
        if (mfade > 0.001) {
            let dh = 0.015;
            let dw = 0.008;
            let pitch = 0.028;
            let base = vec2<f32>(0.0, -0.138);
            let m10 = u32(clamp(round(mach * 10.0), 0.0, 99.0));
            let md = array<u32, 2>(m10 / 10u, m10 % 10u);
            var d = 1e9;
            for (var i = 0u; i < 2u; i += 1u) {
                let off = base + vec2<f32>((f32(i) - 0.5) * pitch, 0.0);
                d = min(d, digit_dist(p_near - off, digit_mask(md[i]), dw, dh));
            }
            d = min(d, length(p_near - (base + vec2<f32>(0.0, -dh))) - 0.0015);
            let lit = mfade * stroke(d, 0.0014, aa);
            // Past the barrier the mach digits go amber: supersonic is a
            // state, not a number.
            mach_amber = step(1.0, mach);
            ro += lit * 0.7 * (1.0 - mach_amber);
            warn_glow += lit * mach_amber;
            halo += 0.15 * mfade * (1.0 - smoothstep(0.0, 0.006, d));
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

    // Palette. On the glass: hologram cyan, white-hot accents, amber for
    // the warning marks — and the whole hologram shifts toward amber as
    // the needle enters the warning arc. On the dash: a period instrument
    // — DIAL ivory markings and a cream needle on the black plate, red for
    // the warning; WARTHOG the A-10's white on black, red-orange warning —
    // where only the readout takes the warning colour, as a lit annunciator
    // would.
    let warn_high = smoothstep(0.85, 1.0, frac);
    let warn_low = 1.0 - smoothstep(0.04, 0.14, frac);
    let warning = mix(warn_high, warn_low, clamp(sense, 0.0, 1.0));
    var face_rgb = vec3<f32>(0.22, 0.85, 1.0);
    var warn_rgb = vec3<f32>(1.0, 0.62, 0.18);
    var hot_rgb = vec3<f32>(1.0);
    if (in_dash) {
        face_rgb = vec3<f32>(0.82, 0.78, 0.62);
        warn_rgb = vec3<f32>(0.95, 0.20, 0.08);
        hot_rgb = vec3<f32>(0.96, 0.92, 0.80);
    }
    if (warthog) {
        face_rgb = vec3<f32>(0.94, 0.94, 0.90);
        warn_rgb = vec3<f32>(1.0, 0.30, 0.08);
        hot_rgb = vec3<f32>(1.0, 1.0, 0.97);
    }
    // JET: the glass over the dial — a soft glint arcing across the top
    // left, and a faint full-circle face ring — so the dial reads as a
    // round instrument under glass, not a projection.
    if (jet) {
        let rf = length(p_face);
        let gdir = normalize(vec2<f32>(-0.55, 0.8));
        let along_g = dot(p_face, gdir);
        let glint = (1.0 - smoothstep(0.0, 0.05, abs(rf - radius * 0.78))) * smoothstep(0.2, 0.8, along_g / radius);
        hot += 0.35 * glint;
        glow += 0.25 * stroke(abs(rf - radius * 1.06), 0.0012, aa);
    }
    let tint = mix(face_rgb, warn_rgb, warning * select(1.0, 0.0, steam));
    let ro_rgb = mix(hot_rgb, warn_rgb, warning * select(0.0, 1.0, steam));
    // A steam gauge has no light of its own: no halo, and its markings
    // are paint, not a lamp.
    if (steam) {
        halo = 0.0;
        glow *= 0.85;
        ro *= 0.9;
    }

    // Scanlines: static spatial modulation — hologram texture without
    // temporal noise (P1: no shimmer, no smear).
    let scan = select(0.90 + 0.10 * sin(in.ndc.y * gauge.b.y * 1.7), 1.0, in_dash);

    let glass = select(canopy_glass(in.ndc, aspect), 1.0, in_dash);

    // The whole instrument surges with the flash, then settles. Hologram
    // accents run a little over 1: real radiance, for the bloom to catch.
    let surge = 1.0 + 1.1 * alert * alert;
    let accent = select(1.25, 0.95, steam);
    let colour = (tint * glow + tint * halo + hot_rgb * hot * accent + ro_rgb * ro * accent
        + warn_rgb * warn_glow) * scan * glass * vis * surge;

    // Additive blend: what is black costs nothing and shows nothing.
    return vec4<f32>(colour, 1.0);
}
