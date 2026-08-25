// sight.wgsl — the gun sight (pass: sight)
//
// Lane: A. Cost class: trivial — a handful of distances per pixel.
//
// A hologram on the glass where the guns point. In freelook the guns
// gimbal to the gaze, so the sight rides with the head; past the
// gimbal's reach the aim stops on the ring and the sight stops with it,
// with a line back to the gaze so the pilot sees why. Around it: the
// gimbal ring itself (faint until the aim is on it), the barrels' own
// pips at the convergence range, heat as an arc filling round the
// reticle, the rail's charge as a second arc. Cyan; amber when clamped,
// hot or jammed.

struct Sight {
    // aspect, strength (0 off), time, gimbal ring radius (NDC, vertical)
    a: vec4<f32>,
    // aim NDC xy, gaze NDC xy
    b: vec4<f32>,
    // clamped, heat, charge, kind (0 cannon, 1 rail)
    c: vec4<f32>,
    // jammed, empty, nose NDC xy
    d: vec4<f32>,
    // each barrel's pip NDC xy, z = shown
    pips: array<vec4<f32>, 4>,
}

@group(0) @binding(0) var<uniform> st: Sight;

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

// A soft line of the given half-width: 1 on it, 0 a pixel or so off.
fn line(d: f32, w: f32) -> f32 {
    return smoothstep(w + 0.0025, w, d);
}

fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / max(dot(ab, ab), 1e-8), 0.0, 1.0);
    return length(p - (a + ab * t));
}

// An arc of a ring about the origin from angle 0 (up) clockwise to `frac`
// of the way round: the distance to it.
fn arc(p: vec2<f32>, r: f32, frac: f32) -> f32 {
    let ang = atan2(p.x, p.y);            // 0 up, positive to the right
    let a = select(ang + 6.2831853, ang, ang >= 0.0);
    let on = a <= frac * 6.2831853;
    let ring = abs(length(p) - r);
    // Off the arc: the distance to its end cap.
    let end = frac * 6.2831853;
    let cap = length(p - vec2<f32>(sin(end), cos(end)) * r);
    let start = length(p - vec2<f32>(0.0, r));
    return select(min(cap, start), ring, on);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let strength = st.a.y;
    if (strength <= 0.0) {
        discard;
    }
    let aspect = st.a.x;
    let now = st.a.z;
    // Work in vertical NDC units so circles are round.
    let q = vec2<f32>(in.ndc.x * aspect, in.ndc.y);
    let aim = vec2<f32>(st.b.x * aspect, st.b.y);
    let gaze = vec2<f32>(st.b.z * aspect, st.b.w);
    let nose = vec2<f32>(st.d.z * aspect, st.d.w);
    let clamped = st.c.x > 0.5;
    let heat = st.c.y;
    let charge = st.c.z;
    let rail = st.c.w > 0.5;
    let jammed = st.d.x > 0.5;
    let empty = st.d.y > 0.5;

    let cyan = vec3<f32>(0.35, 0.90, 1.0);
    let amber = vec3<f32>(1.0, 0.72, 0.25);
    let red = vec3<f32>(1.0, 0.30, 0.18);
    var tint = cyan;
    if (clamped || heat > 0.7) { tint = amber; }
    if (jammed || empty) { tint = red; }

    let p = q - aim;
    let r = length(p);
    var lit = 0.0;

    // The reticle: a ring with four gaps, a dot, ticks. The rail's is a
    // finer, larger ring: a long gun's sight.
    let ring_r = select(0.052, 0.068, rail);
    let ang = atan2(p.x, p.y);
    let gaps = smoothstep(0.10, 0.22, abs(sin(ang * 2.0)));
    lit += line(abs(r - ring_r), 0.0028) * gaps;
    lit += line(r, 0.006) * 0.9;
    // Ticks at the four points, outside the ring.
    let ax = min(abs(p.x), abs(p.y));
    let along = max(abs(p.x), abs(p.y));
    lit += line(ax, 0.0022) * step(ring_r + 0.012, along) * step(along, ring_r + 0.032);
    // Heat: an arc filling clockwise just inside the ring; the rail's
    // charge fills a wider arc outside it.
    if (heat > 0.005) {
        let hot = mix(cyan, red, smoothstep(0.4, 1.0, heat));
        lit += line(arc(p, ring_r - 0.012, heat), 0.0032) * 1.2;
        tint = mix(tint, hot, 0.35 * smoothstep(0.3, 1.0, heat));
    }
    if (charge > 0.005) {
        lit += line(arc(p, ring_r + 0.014, charge), 0.0038) * 1.4;
    }

    // The barrels' pips: a small dot each, at the convergence range,
    // with a hair of a line toward the reticle.
    for (var i = 0u; i < 4u; i += 1u) {
        let pip = st.pips[i];
        if (pip.z < 0.5) { continue; }
        let pp = vec2<f32>(pip.x * aspect, pip.y);
        let dpp = length(q - pp);
        lit += line(dpp, 0.004) * 0.8 + line(abs(dpp - 0.011), 0.0018) * 0.5;
    }

    // The gimbal ring about the nose: faint; bright where the aim sits
    // on it when clamped, with a leader from the gaze to the aim.
    let gr = st.a.w;
    let dn = length(q - nose);
    let ring_lit = line(abs(dn - gr), 0.0022);
    let near_aim = exp(-length(q - aim) / 0.25);
    lit += ring_lit * (0.12 + select(0.0, 0.9 * near_aim, clamped));
    if (clamped) {
        // A dashed leader gaze -> aim, and a cross at the gaze.
        let sd = segment(q, gaze, aim);
        let along_t = dot(q - gaze, aim - gaze) / max(dot(aim - gaze, aim - gaze), 1e-8);
        let dash = step(0.5, fract(along_t * 12.0 - now * 2.0));
        lit += line(sd, 0.0018) * dash * 0.7;
        let g = q - gaze;
        let cross = min(abs(g.x), abs(g.y));
        lit += line(cross, 0.002) * step(max(abs(g.x), abs(g.y)), 0.02) * 0.9;
    }

    if (lit <= 0.001) {
        discard;
    }
    // Hologram texture: scanlines on the glass and a faint flicker.
    let scan = 0.9 + 0.1 * sin(in.ndc.y * 700.0);
    let flick = 0.95 + 0.05 * sin(now * 37.0);
    // Jammed or empty: the sight blinks.
    let blink = select(1.0, 0.5 + 0.5 * step(0.5, fract(now * 2.5)), jammed || empty);
    let bright = select(1.0, 1.35, clamped);
    let colour = tint * lit * strength * bright * scan * flick * blink;
    return vec4<f32>(colour, 1.0);
}
