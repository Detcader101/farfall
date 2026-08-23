// guide.wgsl — the cockpit design guide (pass: guide)
//
// Lane: A. Cost class: trivial — a few lines per pixel, only while on.
//
// A drafting overlay for laying out the cockpit: the glass ruled in
// canopy units, the safe edge (if any) as a box, a ring at every dial's
// anchor with its pick-up reach, and a reticle at the gaze — so the pilot
// can see where a dial sits on the shell and where it will land when
// dropped, locked view or not.

struct Guide {
    // x: aspect, y: on 0..1, z: safe edge fraction, w: unused
    a: vec4<f32>,
    // xy: the gaze on the glass (NDC), z: pick-up reach (NDC), w: looking
    b: vec4<f32>,
    // anchors of dials 1..2 (NDC), each xy; w<-9 means none
    c: vec4<f32>,
    // anchors of dials 3..4
    d: vec4<f32>,
    // anchors of dials 5..6
    e: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gd: Guide;

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

fn ring_at(p: vec2<f32>, c: vec2<f32>, r: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, abs(length(p - c) - r) - w);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let on = gd.a.y;
    if (on < 0.01) {
        discard;
    }
    let aspect = gd.a.x;
    let p = canopy(in.ndc, aspect);
    let aa = max(fwidth(p.x), 1e-5) * 1.2;
    var glow = 0.0;
    var warn = 0.0;

    // The shell ruled every 0.1 canopy units, heavier every 0.5.
    let g = abs(fract(p * 10.0 + 0.5) - 0.5) * 0.1;
    let fine = 1.0 - smoothstep(0.0, aa * 1.5, min(g.x, g.y) - 0.0008);
    let g5 = abs(fract(p * 2.0 + 0.5) - 0.5) * 0.5;
    let coarse = 1.0 - smoothstep(0.0, aa * 1.5, min(g5.x, g5.y) - 0.0012);
    glow += 0.05 * fine + 0.10 * coarse;
    // The axes through the centre.
    glow += 0.18 * (1.0 - smoothstep(0.0, aa * 1.5, min(abs(p.x), abs(p.y)) - 0.0012));

    // The safe edge: the box the slots are pulled into.
    let k = 1.0 - clamp(gd.a.z, 0.0, 0.3);
    let sb = abs(in.ndc) - vec2<f32>(k);
    let box_d = max(sb.x, sb.y);
    let aa_n = max(fwidth(in.ndc.x), 1e-5) * 1.2;
    warn += 0.6 * (1.0 - smoothstep(0.0, aa_n, abs(box_d) - 0.002)) * step(0.01, gd.a.z);

    // Each dial: a ring at its anchor and a dashed circle of its reach.
    let anchors = array<vec2<f32>, 6>(gd.c.xy, gd.c.zw, gd.d.xy, gd.d.zw, gd.e.xy, gd.e.zw);
    for (var i = 0; i < 6; i += 1) {
        let a = anchors[i];
        if (a.x < -9.0) { continue; }
        let ca = canopy(a, aspect);
        glow += 0.9 * ring_at(p, ca, 0.012, 0.002, aa);
        glow += 0.5 * (1.0 - smoothstep(0.0, aa * 1.5, min(abs(p.x - ca.x), abs(p.y - ca.y)) - 0.001))
            * step(length(p - ca), 0.04);
        let reach = gd.b.z;
        let ang = atan2(p.y - ca.y, p.x - ca.x);
        let dash = step(0.5, fract(ang * 16.0 / 6.2832));
        glow += 0.35 * ring_at(p, ca, reach, 0.0015, aa) * dash;
    }

    // The gaze: a reticle where the head points, amber while looking.
    let gz = canopy(gd.b.xy, aspect);
    let rg = length(p - gz);
    let reticle = ring_at(p, gz, 0.02, 0.002, aa)
        + (1.0 - smoothstep(0.0, aa * 1.5, min(abs(p.x - gz.x), abs(p.y - gz.y)) - 0.001)) * step(rg, 0.035) * step(0.012, rg);
    warn += 0.8 * reticle * gd.b.w;
    glow += 0.4 * reticle * (1.0 - gd.b.w);

    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    let colour = (cyan * glow + amber * warn) * on;
    if (dot(colour, colour) < 1e-6) {
        discard;
    }
    return vec4<f32>(colour, 1.0);
}
