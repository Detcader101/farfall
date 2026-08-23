// shield.wgsl — the force field (pass: shield)
//
// A shell around the ship, a few metres out, invisible until something
// hits it. Each strike raises a ripple from its point of impact: a ring
// of blue holographic light spreading evenly over the shell at a fixed
// speed and fading as it goes, and around it the shell's own honeycomb
// shows through for a moment — the field ablating, Star Trek fashion.
// Drawn on the world side of the glass (before the cabin), additively:
// where nothing has hit, nothing is there.

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
    let da = hex_edge(a / cell * 1.0);
    let db = hex_edge(b / cell * 1.0);
    return min(da, db);
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

    var glow = 0.0;
    var comb = 0.0;
    for (var i = 0u; i < IMPACTS; i += 1u) {
        if (i >= n_hits) { break; }
        let h = sh.hits[i];
        let size = floor(h.w / 1000.0) / 1000.0;
        let t0 = h.w - floor(h.w / 1000.0) * 1000.0;
        let age = now - t0;
        if (age < 0.0 || age > 3.0) { continue; }
        let d = acos(clamp(dot(n, h.xyz), -1.0, 1.0)) * rad;
        // The ring: a front at speed × age, a few cells wide, softer and
        // wider as it goes; a faint afterglow inside it.
        let front = speed * age;
        let width = 0.10 + 0.12 * age;
        let x = (d - front) / width;
        let ring = exp(-x * x);
        // A second, fainter ring a little behind the first: the wake.
        let x2 = (d - front * 0.72) / (width * 1.6);
        let wake = exp(-x2 * x2) * 0.35;
        let fade = exp(-age * 1.2) * (0.4 + 0.6 * size);
        glow += (ring + wake) * fade;
        // The honeycomb shows in a band behind the ring and at the impact
        // itself, where the field is working hardest.
        let band = smoothstep(front - 1.2 - 1.5 * size, front, d) * (1.0 - smoothstep(front, front + 0.15, d));
        let at_hit = (1.0 - smoothstep(0.0, 0.5 + 1.0 * size, d)) * exp(-age * 2.0);
        comb += (band * 0.8 + at_hit) * fade;
    }
    // The hyper drive: space streaming over the whole shell. The field
    // lights from the nose back, the honeycomb everywhere under bands of
    // light sweeping aft at speed, brightest where the stream meets it.
    var stream = 0.0;
    if (hyper > 0.001) {
        let nose = vec3<f32>(0.0, 0.0, -1.0);
        let head_on = dot(n, nose);
        let from_front = smoothstep(-0.6, 0.9, head_on);
        let along = acos(clamp(head_on, -1.0, 1.0)) * rad;
        let bands = 0.5 + 0.5 * sin(along * 6.0 - now * 40.0);
        let bands2 = 0.5 + 0.5 * sin(along * 13.0 - now * 71.0 + n.x * 9.0);
        stream = hyper * (0.25 + 0.75 * from_front) * (0.35 + 0.4 * bands + 0.25 * bands2);
        comb += stream * 0.7;
        glow += stream * 0.28;
    }
    if (glow + comb < 0.002) {
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
    let lines = 1.0 - smoothstep(0.0, aa * 2.0 + cell * 0.05, edge);
    let cells = 0.12 * (1.0 - smoothstep(0.0, cell * 0.5, edge));
    let blue = vec3<f32>(0.30, 0.65, 1.0);
    let white = vec3<f32>(0.85, 0.95, 1.0);
    // The shell is seen at a grazing angle near its rim: a touch more there.
    let graze = pow(1.0 - abs(dot(n, ray)), 2.0);
    let colour = (blue * (glow * 0.7 + comb * (lines * 1.2 + cells)) + white * glow * glow * 0.35)
        * (1.0 + 0.6 * graze)
        * strength;
    return vec4<f32>(colour, 1.0);
}
