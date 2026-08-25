// scar.wgsl — craters on the rocks (pass: scar)
//
// Where a slug struck and the rock held: a crater on its face, riding
// with the rock, glowing with what the slug left in it — white-hot at
// the centre with cracks running out, then orange, then the dull red of
// stone cooling, then nothing. Light over the belt, so the rock's own
// shading shows through; hidden by a nearer rock.

struct Scar {
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // exposure, glow, scars in use, rocks in use
    look: vec4<f32>,
    // xyz rock centre (ship frame, m), w rock radius
    centres: array<vec4<f32>, 32>,
    // xyz direction on the rock, w crater radius (m)
    dirs: array<vec4<f32>, 32>,
    // heat 0..1, seed, -, -
    info: array<vec4<f32>, 32>,
    rocks: array<vec4<f32>, 48>,
}

@group(0) @binding(0) var<uniform> sc: Scar;

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

const SCARS: u32 = 32u;
const ROCKS: u32 = 48u;

fn rock_depth(ray: vec3<f32>) -> f32 {
    let n = u32(sc.look.w);
    var best = 1e30;
    for (var i = 0u; i < ROCKS; i += 1u) {
        if (i >= n) { break; }
        let rk = sc.rocks[i];
        let c = rk.xyz;
        let r = rk.w * 0.98;
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - r * r);
        if (disc < 0.0) { continue; }
        let t = b - sqrt(disc);
        if (t > 0.0 && t < best) { best = t; }
    }
    return best;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = u32(sc.look.z);
    if (n == 0u) {
        discard;
    }
    let aspect = sc.right.w;
    let tan_half = sc.up.w;
    let ray = normalize(sc.fwd.xyz + sc.right.xyz * (in.ndc.x * tan_half * aspect) + sc.up.xyz * (in.ndc.y * tan_half));
    let nearest = rock_depth(ray);
    var colour = vec3<f32>(0.0);
    for (var i = 0u; i < SCARS; i += 1u) {
        if (i >= n) { break; }
        let ck = sc.centres[i];
        let c = ck.xyz;
        let R = ck.w * 0.98;
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - R * R);
        if (disc < 0.0) { continue; }
        let t = b - sqrt(disc);
        if (t <= 0.0 || t > nearest + 0.05) { continue; }
        let p = ray * t;
        let nrm = normalize(p - c);
        let dk = sc.dirs[i];
        let size = max(dk.w, 0.05);
        // How far round the rock from the crater's centre, in crater radii.
        let ang = acos(clamp(dot(nrm, dk.xyz), -1.0, 1.0));
        let u = ang * R / size;
        if (u > 1.6) { continue; }
        let heat = sc.info[i].x;
        let seed = sc.info[i].y;
        // The crater: a hot floor, cracks running out, a rim; a halo of
        // its light on the stone around.
        let floor_g = exp(-u * u * 2.5);
        let cracks = pow(vnoise((p - c) * (5.0 / size) + seed * 13.0), 3.0) * smoothstep(1.3, 0.6, u);
        let rim = smoothstep(0.18, 0.0, abs(u - 1.0)) * 0.5;
        let halo = exp(-u * 1.6) * 0.18;
        // The colour of its heat: white, then orange, then dull red.
        let red = vec3<f32>(0.62, 0.07, 0.015);
        let orange = vec3<f32>(1.0, 0.48, 0.12);
        let white = vec3<f32>(1.0, 0.94, 0.82);
        var col = mix(red, orange, smoothstep(0.08, 0.45, heat));
        col = mix(col, white, smoothstep(0.5, 1.0, heat));
        let strength = pow(heat, 0.7) * (floor_g * 1.2 + cracks * 1.4 + rim * heat + halo);
        colour += col * strength;
    }
    colour *= sc.look.y * sc.look.x;
    if (max(colour.r, max(colour.g, colour.b)) < 0.002) {
        discard;
    }
    return vec4<f32>(colour, 1.0);
}
