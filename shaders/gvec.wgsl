// gvec.wgsl — the G vector: where the load is coming from (pass: gvec)
//
// A cross-plot accelerometer: the felt acceleration in the ship's frame,
// lateral across and vertical up the face, drawn as a line from the hub
// to a dot — the direction of the load and how much — over concentric
// rings of the current range. Longitudinal G (thrust, braking) rides a
// bar up the right edge. Same uniforms as the arc gauge: an instrument is
// data. Additively blended.

struct Gauge {
    // The glass numbers (64 bytes), then the placement for a DIAL in the
    // dash: p0 right, p1 up, p2 fwd (w: tan half fov) of the head in ship
    // frame, p3 the dial's centre (w: metres per drawing unit; 0 means the
    // instrument is on the glass, not in the dash).
    // x: lateral G (+ right), y: visibility 0..1, z: time (s), w: aspect
    a: vec4<f32>,
    // x: full-scale G (the outer ring), y: target height in px, zw: anchor
    b: vec4<f32>,
    // x: vertical G (+ up), y: longitudinal G (+ forward), z: 2 for the JET
    // style, w: the range's packed multiplier (0: none shown)
    c: vec4<f32>,
    // xy: hologram sway (canopy units), z: |G| readout packed (digits +
    // 1000 × exponent), w: readout decimal dot after digit 1|2
    d: vec4<f32>,
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
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
    // A dial in the dash is seen in perspective: a wider quad to be safe.
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


@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = gauge.a.y;
    if (vis < 0.01) {
        discard;
    }
    let aspect = gauge.a.w;
    let anchor = gauge.b.zw;
    let in_dash = gauge.p3.w > 0.0;
    var p = (canopy(in.ndc, aspect) - canopy(anchor, aspect)) / max(gauge.p0.w, 0.25);
    if (in_dash) {
        let duv = dial_plane_uv(in.ndc, aspect, gauge.p0, gauge.p1, gauge.p2, gauge.p3, DIAL_DASH_N);
        if (duv.z < 0.5) {
            discard;
        }
        p = duv.xy;
    }
    if (!in_dash) {
        let tilt = gauge.p1.w;
        let lean = max(cos(tilt), 0.35);
        let persp = 1.0 - 0.35 * sin(tilt) * p.y / 0.2;
        p = vec2<f32>(p.x * persp, p.y / lean * persp);
    }
    let radius = 0.155;
    if (length(p) > radius * 1.5 + 0.06) {
        discard;
    }
    let sway = select(gauge.d.xy, vec2<f32>(0.0), in_dash);
    let p_face = p - sway * 0.18;
    let p_mid = p - sway * 0.55;
    let p_near = p - sway * 1.0;
    let extrude = vec2<f32>(0.0032, -0.0032);
    let aa = max(fwidth(p.x), 1e-5) * 0.9;

    let full = max(gauge.b.x, 1e-6);
    let g_lat = gauge.a.x;
    let g_vert = gauge.c.x;
    let g_long = gauge.c.y;
    let packed = u32(max(round(gauge.c.w), 0.0));
    let mult_m = packed % 10u;
    let mult_e = packed / 10u;
    let warthog = gauge.c.z >= 4.0;
    let jet = (gauge.c.z - select(0.0, 4.0, warthog)) >= 2.0;

    var glow = 0.0;
    var hot = 0.0;
    var warn_glow = 0.0;

    let r = length(p_face);
    // The face: an outer ring, rings at each quarter of the range, and the
    // cross — up, down, left, right — ruled through the hub.
    let ring = abs(r - radius);
    glow += 0.9 * (1.0 - smoothstep(0.0, aa * 1.6, ring - 0.0016));
    glow += 0.18 * (1.0 - smoothstep(0.0, 0.012, ring));
    for (var q = 1.0; q < 4.0; q += 1.0) {
        let rq = abs(r - radius * q * 0.25);
        glow += 0.35 * (1.0 - smoothstep(0.0, aa * 1.6, rq - 0.0008));
    }
    // The cross, broken around the hub and the rings' crossings.
    let cross = min(abs(p_face.x), abs(p_face.y));
    if (r < radius && r > 0.02) {
        glow += 0.3 * (1.0 - smoothstep(0.0, aa * 1.6, cross - 0.0008));
    }
    // Ticks round the rim every 30 degrees, longer at the cardinals.
    if (r < radius && r > radius - 0.03) {
        let theta = atan2(p_face.x, p_face.y);
        let t12 = abs(fract(theta / TAU_G * 12.0) - 0.5);
        let t4 = abs(fract(theta / TAU_G * 4.0) - 0.5);
        let ang_aa = aa / max(r, 1e-4) * 12.0 / TAU_G;
        if (r > radius - 0.014) {
            glow += 0.45 * (1.0 - smoothstep(0.0, 0.06 + ang_aa, t12));
        }
        glow += 0.7 * (1.0 - smoothstep(0.0, 0.035 + ang_aa * 3.0, t4));
    }
    // The multiplier beside the dial, upper right: what the outer ring is
    // worth, ×m E k, when the range has climbed past the stock 2 G.
    if (packed > 0u) {
        let dw = 0.009;
        let dh = 0.016;
        let base = vec2<f32>(radius * 0.72, radius * 0.86);
        let xd = length(p_near - base + vec2<f32>(0.026, 0.0)) - 0.004;
        hot += 0.6 * (1.0 - smoothstep(0.0, aa * 1.8, xd));
        let dd = digit_dist(p_near - base, digit_mask(mult_m), dw, dh);
        hot += 0.8 * (1.0 - smoothstep(0.0, aa * 1.8, dd - 0.0016));
        if (mult_e > 0u) {
            let eb = base + vec2<f32>(0.0, -0.042);
            let ed = digit_dist(p_near - eb, 121u, dw, dh);
            hot += 0.8 * (1.0 - smoothstep(0.0, aa * 1.8, ed - 0.0016));
            let cell = eb + vec2<f32>(0.028, 0.0);
            let d1 = digit_dist(p_near - cell, digit_mask(mult_e % 10u), dw, dh);
            hot += 0.8 * (1.0 - smoothstep(0.0, aa * 1.8, d1 - 0.0016));
        }
    }

    // The load: a line from the hub to the dot, the dot itself, on the
    // middle layer. Lateral across, vertical up; clamped to the rim.
    var v = vec2<f32>(g_lat, g_vert) / full * radius;
    let vl = length(v);
    if (vl > radius * 0.97) {
        v = v / vl * radius * 0.97;
    }
    let line = seg_dist(p_mid, vec2<f32>(0.0), v);
    let stem = 1.0 - smoothstep(0.0, aa * 1.6, line - 0.0012);
    hot += 0.7 * stem;
    glow += 0.25 * (1.0 - smoothstep(0.0, 0.008, line));
    let dd_ = length(p_mid - v);
    let dot_core = 1.0 - smoothstep(0.010, 0.010 + aa * 1.8, dd_);
    let dot_ring = 1.0 - smoothstep(0.0, aa * 1.6, abs(dd_ - 0.016) - 0.0012);
    hot += 0.9 * dot_core + 0.6 * dot_ring;
    glow += 0.35 * (1.0 - smoothstep(0.0, 0.03, dd_));
    let dg = length(p_mid + extrude - v);
    glow += 0.16 * (1.0 - smoothstep(0.010, 0.014, dg));
    // Hub.
    glow += 0.5 * (1.0 - smoothstep(0.008, 0.012, length(p_mid)));

    // Longitudinal G: a bar just outside the ring on the right, the
    // marker up for thrust, down for braking, a notch at zero.
    {
        let bx = radius * 1.22;
        let half = radius * 0.8;
        let bar = seg_dist(p_face, vec2<f32>(bx, -half), vec2<f32>(bx, half));
        glow += 0.5 * (1.0 - smoothstep(0.0, aa * 1.6, bar - 0.0010));
        let notch = seg_dist(p_face, vec2<f32>(bx - 0.012, 0.0), vec2<f32>(bx + 0.012, 0.0));
        glow += 0.5 * (1.0 - smoothstep(0.0, aa * 1.6, notch - 0.0010));
        let my = clamp(g_long / full, -1.0, 1.0) * half;
        let mark = seg_dist(p_mid, vec2<f32>(bx - 0.016, my), vec2<f32>(bx + 0.016, my));
        hot += 0.8 * (1.0 - smoothstep(0.0, aa * 1.6, mark - 0.0016));
        glow += 0.25 * (1.0 - smoothstep(0.0, 0.008, mark));
    }

    // |G| readout in the lower part of the face, three digits with a dot.
    {
        let dh = 0.022;
        let dw = 0.012;
        let pitch = 0.038;
        let base = vec2<f32>(0.0, -radius * 0.55);
        let packed_ro = u32(max(round(gauge.d.z), 0.0));
        let shown = packed_ro % 1000u;
        let digits = array<u32, 3>(shown / 100u, (shown / 10u) % 10u, shown % 10u);
        for (var i = 0u; i < 3u; i += 1u) {
            let off = base + vec2<f32>((f32(i) - 1.0) * pitch, 0.0);
            let dd = digit_dist(p_near - off, digit_mask(digits[i]), dw, dh);
            hot += 0.9 * (1.0 - smoothstep(0.0, aa * 1.8, dd - 0.0016));
            glow += 0.2 * (1.0 - smoothstep(0.0, 0.006, dd));
        }
        let dot_after = gauge.d.w;
        if (dot_after > 0.5) {
            let dot_pos = base + vec2<f32>((dot_after - 1.5) * pitch, -dh);
            let dr = length(p_near - dot_pos);
            hot += 0.9 * (1.0 - smoothstep(0.003, 0.003 + aa * 1.8, dr));
        }
    }

    // The warning: a load past 6 G reddens the rim, however the range reads.
    let g_total = length(vec3<f32>(g_lat, g_vert, g_long));
    let warning = smoothstep(5.0, 7.0, g_total);
    warn_glow += warning * 0.6 * (1.0 - smoothstep(0.0, 0.02, ring));

    if (jet) {
        let gdir = normalize(vec2<f32>(-0.55, 0.8));
        let along_g = dot(p_face, gdir);
        let glint = (1.0 - smoothstep(0.0, 0.05, abs(r - radius * 0.78))) * smoothstep(0.2, 0.8, along_g / radius);
        hot += 0.35 * glint;
        glow += 0.25 * (1.0 - smoothstep(0.0, aa * 1.6, abs(r - radius * 1.06) - 0.0012));
    }

    let period = in_dash || jet;
    var cyan = select(vec3<f32>(0.22, 0.85, 1.0), vec3<f32>(0.82, 0.78, 0.62), period);
    var amber = select(vec3<f32>(1.0, 0.62, 0.18), vec3<f32>(0.85, 0.22, 0.10), period);
    if (warthog) {
        cyan = vec3<f32>(0.92, 0.92, 0.88);
        amber = vec3<f32>(1.0, 0.36, 0.12);
    }
    let tint = mix(cyan, amber, warning);
    if (period) {
        glow *= 0.55;
        hot *= 0.85;
    }
    let scan = select(0.90 + 0.10 * sin(in.ndc.y * gauge.b.y * 1.7), 1.0, period);
    let glass = select(canopy_glass(in.ndc, aspect), 1.0, period);
    let hot_rgb = select(vec3<f32>(1.0), vec3<f32>(0.96, 0.92, 0.80), period);
    let colour = (tint * glow + hot_rgb * hot * 0.9 + amber * warn_glow) * scan * glass * vis;
    return vec4<f32>(colour, 1.0);
}
