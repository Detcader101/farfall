// belt.wgsl — the asteroid belt, up close (pass: belt)
//
// The live rocks near the ship: each a sphere with a craggy surface —
// the normal roughened by noise of its own seed, the silhouette nibbled
// by it — lit by the Sun with a hard terminator, a touch of the ring's
// dust-light in the shade, turning slowly on its own axis. Ray-traced:
// every pixel tests every live rock (a few dozen) and shades the nearest.
// Written opaque where a rock is; nothing elsewhere.

const LIVE: u32 = 48u;

struct Belt {
    // xyz: the head's right axis in ship frame. w: aspect
    right: vec4<f32>,
    // xyz: the head's up axis. w: tan(fov/2)
    up: vec4<f32>,
    // xyz: the head's forward axis (-Z is the nose). w: time (s)
    fwd: vec4<f32>,
    // xyz: the Sun's direction, ship frame; w: rocks in use
    sun: vec4<f32>,
    // xyz: exposure, ring-light, unused
    look: vec4<f32>,
    // Rocks: xyz centre relative to the head (ship frame, m), w radius (m)
    rocks: array<vec4<f32>, 48>,
    // Per rock: x seed, y spin phase (rad), zw unused
    spins: array<vec4<f32>, 48>,
}

@group(0) @binding(0) var<uniform> bt: Belt;

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

// A rock's surface: the unit sphere, its radius varied by noise in its
// own frame — craters and ridges — so the silhouette is not a circle.
fn rock_bump(n: vec3<f32>, seed: f32) -> f32 {
    let q = n * 2.3 + vec3<f32>(seed * 37.0);
    return 0.10 * (fbm3(q) - 0.5) + 0.05 * (vnoise(q * 3.1) - 0.5);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n_rocks = u32(bt.sun.w);
    if (n_rocks == 0u) {
        discard;
    }
    let aspect = bt.right.w;
    let tan_half = bt.up.w;
    let ray = normalize(bt.fwd.xyz + bt.right.xyz * (in.ndc.x * tan_half * aspect) + bt.up.xyz * (in.ndc.y * tan_half));
    let now = bt.fwd.w;

    // Nearest rock along the ray, by its bounding sphere.
    var best_t = 1e30;
    var best = -1;
    for (var i = 0u; i < LIVE; i += 1u) {
        if (i >= n_rocks) { break; }
        let rk = bt.rocks[i];
        let c = rk.xyz;
        let r = rk.w * 1.12;
        let b = dot(ray, c);
        let disc = b * b - (dot(c, c) - r * r);
        if (disc < 0.0) { continue; }
        let t = b - sqrt(disc);
        if (t > 0.0 && t < best_t) {
            best_t = t;
            best = i32(i);
        }
    }
    if (best < 0) {
        discard;
    }
    let rk = bt.rocks[best];
    let c = rk.xyz;
    let r0 = rk.w;
    let seed = bt.spins[best].x;
    let phase = bt.spins[best].y;
    // Refine against the bumped surface: a few steps of the sphere's own
    // distance with the bump folded in, from the bounding hit inward.
    var t = best_t;
    var hit = false;
    var p = ray * t;
    var n = normalize(p - c);
    for (var k = 0u; k < 12u; k += 1u) {
        p = ray * t;
        let rel = p - c;
        let d = length(rel);
        n = rel / max(d, 1e-6);
        // The rock's own frame: turned about its axis by its spin.
        let ca = cos(phase);
        let sa = sin(phase);
        let nl = vec3<f32>(ca * n.x - sa * n.z, n.y, sa * n.x + ca * n.z);
        let surf = r0 * (1.0 + rock_bump(nl, seed));
        let sd = d - surf;
        if (sd < r0 * 0.004) {
            hit = true;
            break;
        }
        t += max(sd * 0.8, r0 * 0.002);
        if (t > best_t + r0 * 2.4) {
            break;
        }
    }
    if (!hit) {
        discard;
    }
    // The normal from the bumped surface: the sphere's, tilted by the
    // bump's gradient over the tangent plane.
    let ca = cos(phase);
    let sa = sin(phase);
    let nl = vec3<f32>(ca * n.x - sa * n.z, n.y, sa * n.x + ca * n.z);
    let e = 0.02;
    let tu = normalize(cross(n, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(n.y) > 0.9)));
    let tv = cross(n, tu);
    let tul = vec3<f32>(ca * tu.x - sa * tu.z, tu.y, sa * tu.x + ca * tu.z);
    let tvl = vec3<f32>(ca * tv.x - sa * tv.z, tv.y, sa * tv.x + ca * tv.z);
    let b0 = rock_bump(nl, seed);
    let bu = rock_bump(normalize(nl + tul * e), seed) - b0;
    let bv = rock_bump(normalize(nl + tvl * e), seed) - b0;
    let nn = normalize(n - (tu * bu + tv * bv) * (8.0 / e) * 0.03);

    let sun = normalize(bt.sun.xyz);
    let light = max(dot(nn, sun), 0.0);
    // The ring's dust glows faintly all round: a little fill in the shade.
    let fill = bt.look.y * (0.35 + 0.65 * max(dot(nn, -ray), 0.0));
    let grain = 0.85 + 0.3 * (vnoise(nl * 9.0 + seed * 11.0) - 0.5);
    let albedo = vec3<f32>(0.32, 0.30, 0.27) * grain;
    let lit = albedo * (light * 1.6 + fill * 0.12);
    let colour = radiance(lit, bt.look.x);
    return vec4<f32>(colour + vec3<f32>(dither_px(in.pos.xy)), 1.0);
}
