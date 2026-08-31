// shield.wgsl — the force field (pass: shield)
//
// A shell around the ship, a few metres out, invisible until something
// hits it. Each strike raises a ripple from its point of impact: a ring
// of blue light spreading evenly over the shell at a fixed speed and
// fading as it goes — a liquid wave, its crest white-hot, caustics
// wrinkling the field inside it — and behind the crest the shell's own
// honeycomb shows through for a moment, cell by cell, the field ablating
// Star Trek fashion — a ring of cells round each strike, never the whole
// shell. Under the hyper drive the whole shell is a liquid skin: space
// streaming over it aft from the nose in bands, a refractive sheen
// wandering over it, a violet fringe at the graze; the honeycomb stays
// all but hidden under it.
// Drawn on the world side of the glass (before the cabin), additively:
// where nothing has hit, nothing is there. Written as radiance — the
// crests go well over 1.0 for the post pass's bloom.

const IMPACTS: u32 = 8u;

struct Shield {
    // xyz: the head's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the head's forward axis (-Z is the nose). w: time (s)
    fwd: vec4<f32>,
    // xyz: the shell's centre in ship frame; w: its radius (m)
    shell: vec4<f32>,
    // x: strength 0..2 (the SHIELD setting), y: ripple speed (m/s),
    // z: honeycomb cell size (m), w: impact count in use
    look: vec4<f32>,
    // x: the hyper drive's field 0..1 — the whole shell ablating
    flow: vec4<f32>,
    // Impacts: xyz unit direction from the shell's centre, w packs the
    // strike's time (s) + 1000 × size (0..1).
    hits: array<vec4<f32>, 8>,
}

@group(0) @binding(0) var<uniform> sh: Shield;

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

// Honeycomb: distance to the nearest hex edge in a 2D chart of the shell
// (the two tangent coordinates at the hit), 0 on an edge.
fn hex_edge(p: vec2<f32>) -> f32 {
    let k = vec3<f32>(-0.8660254, 0.5, 0.57735);
    var q = abs(p);
    q -= 2.0 * min(dot(k.xy, q), 0.0) * k.xy;
    q -= vec2<f32>(clamp(q.x, -k.z, k.z), 1.0);
    return abs(length(q) * sign(q.y));
}

fn honeycomb(uv: vec2<f32>, cell: f32) -> f32 {
    // Two offset lattices make the hex tiling.
    let s = vec2<f32>(1.0, 1.7320508) * cell;
    let a = (fract(uv / s) - 0.5) * s;
    let b = (fract((uv - s * 0.5) / s) - 0.5) * s;
    // Adjacent centres sit a cell apart, so a hex's apothem is half a
    // cell: the edge distance comes back in apothems, 0 on an edge and
    // 1 at a centre.
    // hex_edge's hexagon is flat-topped; this tiling is pointy-topped
    // (rows a cell apart across flat sides), so the axes swap.
    let da = hex_edge(a.yx / (cell * 0.5));
    let db = hex_edge(b.yx / (cell * 0.5));
    return min(da, db);
}

// Caustics: two noise fields sliding over each other, their product
// sharpened into the bright ropes light makes through rippling water.
fn caustic(uv: vec2<f32>, now: f32, scale: f32) -> f32 {
    let a = vnoise(vec3<f32>(uv * scale, now * 1.7));
    let b = vnoise(vec3<f32>(uv * scale * 1.9 + vec2<f32>(3.1, 7.7), -now * 2.3));
    return pow(clamp(a * b * 3.4, 0.0, 1.5), 2.4);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let strength = sh.look.x;
    let n_hits = u32(sh.look.w);
    let hyper = sh.flow.x;
    if (strength <= 0.0 || (n_hits == 0u && hyper <= 0.001)) {
        discard;
    }
    let aspect = sh.right.w;
    let tan_half = sh.up.w;
    let ray = normalize(sh.fwd.xyz + sh.right.xyz * (in.ndc.x * tan_half * aspect) + sh.up.xyz * (in.ndc.y * tan_half));
    // From inside the shell: the far hit.
    let c = sh.shell.xyz;
    let rad = sh.shell.w;
    let b = dot(ray, c);
    let disc = b * b - (dot(c, c) - rad * rad);
    if (disc < 0.0) {
        discard;
    }
    let t = b + sqrt(disc);
    let p = ray * t;
    let n = normalize(p - c);
    let now = sh.fwd.w;
    let speed = max(sh.look.y, 0.1);
    let cell = max(sh.look.z, 0.05);

    var crest = 0.0;
    var glow = 0.0;
    var comb = 0.0;
    var wet = 0.0;
    for (var i = 0u; i < IMPACTS; i += 1u) {
        if (i >= n_hits) { break; }
        let h = sh.hits[i];
        let size = floor(h.w / 1000.0) / 1000.0;
        let t0 = h.w - floor(h.w / 1000.0) * 1000.0;
        let age = now - t0;
        if (age < 0.0 || age > 3.0) { continue; }
        let d = acos(clamp(dot(n, h.xyz), -1.0, 1.0)) * rad;
        // The wave: a sharp crest at speed × age, a broader swell behind
        // it, a second crest in its wake — softer and wider as it goes.
        let front = speed * age;
        let width = 0.05 + 0.07 * age;
        let x = (d - front) / width;
        let ring = exp(-x * x);
        let xs = (d - front + width * 2.5) / (width * 4.0);
        let swell = exp(-xs * xs) * 0.5;
        let x2 = (d - front * 0.70) / (width * 2.2);
        let wake = exp(-x2 * x2) * 0.4;
        let fade = exp(-age * 1.1) * (0.5 + 0.5 * size);
        // The wave's energy thins as the ring grows round the shell: the
        // crest and its swell are strongest near the strike.
        let thin = 0.35 + 0.65 * exp(-d / 2.5);
        crest += ring * fade * thin;
        glow += (swell + wake) * fade * thin;
        // The honeycomb shows only in a ring just behind the crest — half
        // a metre to a metre of shell — and at the impact itself, dying
        // with distance from the strike and faster with time than the
        // wave: a few cells lit round each hit, never the whole shell
        // (three strikes used to tile the sky). The field is wet —
        // caustic — through the same ring.
        let band = smoothstep(front - 0.4 - 0.3 * size, front - 0.05, d) * (1.0 - smoothstep(front - 0.05, front + 0.10, d));
        let reach = exp(-d / (0.6 + 0.6 * size));
        let at_hit = (1.0 - smoothstep(0.0, 0.3 + 0.4 * size, d)) * exp(-age * 2.5);
        let comb_fade = exp(-age * 1.8) * (0.5 + 0.5 * size);
        comb += (band * reach * 0.9 + at_hit) * comb_fade;
        wet += band * reach * fade;
    }
    // The hyper drive: space streaming over the whole shell. The field
    // lights from the nose back under bands of light sweeping aft at
    // speed, a liquid sheen wandering over it — a wet skin, the honeycomb
    // no more than a ghost in it (a lattice over the whole sky hid the
    // world it was there to protect).
    var stream = 0.0;
    if (hyper > 0.001) {
        let nose = vec3<f32>(0.0, 0.0, -1.0);
        let head_on = dot(n, nose);
        let from_front = smoothstep(-0.6, 0.9, head_on);
        let along = acos(clamp(head_on, -1.0, 1.0)) * rad;
        let bands = 0.5 + 0.5 * sin(along * 6.0 - now * 40.0);
        let bands2 = 0.5 + 0.5 * sin(along * 13.0 - now * 71.0 + n.x * 9.0);
        let sheen = vnoise(vec3<f32>(n.x * 4.0, n.y * 4.0, along * 1.4 - now * 9.0));
        stream = hyper * (0.25 + 0.75 * from_front) * (0.3 + 0.35 * bands + 0.2 * bands2 + 0.5 * pow(sheen, 3.0));
        comb += stream * 0.04;
        glow += stream * 0.16;
        wet += stream * 0.6;
    }
    if (crest + glow + comb < 0.002) {
        discard;
    }
    // The chart: tangent coordinates about the nearest hit (the first
    // one is fine — the cells only need to look like cells); under the
    // hyper drive alone, about the nose.
    let h0 = select(sh.hits[0].xyz, vec3<f32>(0.0, 0.0, -1.0), n_hits == 0u);
    let e1 = normalize(cross(h0, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(h0.y) > 0.9)));
    let e2 = cross(h0, e1);
    let uv = vec2<f32>(dot(p - c, e1), dot(p - c, e2));
    let edge = honeycomb(uv, cell);
    let aa = max(fwidth(edge), 1e-4);
    let lines = 1.0 - smoothstep(0.0, aa * 1.5 + 0.07, edge);
    let cells = 0.5 * (1.0 - smoothstep(0.0, 0.7, edge));
    let caus = caustic(uv, now, 2.2);
    let blue = vec3<f32>(0.25, 0.60, 1.30);
    let cyan = vec3<f32>(0.45, 0.90, 1.20);
    let white = vec3<f32>(1.10, 1.30, 1.60);
    let violet = vec3<f32>(0.55, 0.30, 1.00);
    // The shell is seen at a grazing angle near its rim: a touch more there.
    let graze = pow(1.0 - abs(dot(n, ray)), 2.0);
    let colour = (
            white * crest * 0.85
            + cyan * crest * caus * 1.0
            + blue * glow * 0.45
            + blue * comb * (lines * 1.6 + cells * (0.35 + 1.4 * caus))
            + cyan * wet * caus * 0.6
            + violet * hyper * graze * 0.5
        )
        * (1.0 + 0.6 * graze)
        * strength;
    return vec4<f32>(colour, 1.0);
}
