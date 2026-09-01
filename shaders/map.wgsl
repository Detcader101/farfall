// map.wgsl — the system map in three dimensions (pass: map)
//
// Lane: A. Cost class: moderate — a short SDF march per pixel, but only
// inside the pane and only while the MAP page is open.
//
// The system at log scale: the Moon is 60 planet radii out and the Sun
// 23,000, so a linear map shows either nothing or a dot. One map unit per
// decade of distance from the planet, in every direction — a 3D log-radial
// warp of the world about the planet. The reference plane is the Moon's
// orbital plane (XZ), drawn as a grid; every body and the ship stand on a
// VERTICAL POLE from the plane to where they are, so height above or
// below the plane reads at a glance (the Elite convention). The ship is a
// small dart in its true attitude. Rings around each body on the plane
// (how many is the pilot's setting); the destination's arrival ring in
// amber. A camera orbits the planet: drag to turn, wheel to zoom.
//
// Everything is signed-distance geometry marched in the fragment shader:
// no mesh crosses from the CPU, the ship included.

struct Map {
    // xyz: camera eye, map units. w: visibility 0..1
    eye: vec4<f32>,
    // camera basis; w of right: dim the screen round the pane (1) or not
    // (0, a gauge); w of fwd: tan(fov/2)
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // x: pane centre x, y: pane centre y, z: pane half width (NDC), w: aspect
    pane: vec4<f32>,
    // bodies: xyz map position, w: drawn radius (map units)
    planet: vec4<f32>,
    moon: vec4<f32>,
    sun: vec4<f32>,
    // xyz: ship map position, w: ship size (map units)
    ship: vec4<f32>,
    // the ship's attitude: its axes in world frame
    ship_right: vec4<f32>,
    ship_up: vec4<f32>,
    ship_fwd: vec4<f32>,
    // xyz: destination map position, w: arrival ring radius (map units)
    dest: vec4<f32>,
    // x: time, y: rings per body (0..6), z: moon orbit radius (map units),
    // w: grid on (1) / off (0)
    misc: vec4<f32>,
    // xyz: Uranus' map position, w: drawn radius
    uranus: vec4<f32>,
}

@group(0) @binding(0) var<uniform> map: Map;

const DIM_ALPHA: f32 = 0.74;
// The pane is the top layer: fully opaque, so a bright rock or boom
// behind it never ghosts through the chart.
const PANE_ALPHA: f32 = 1.0;
const POLE_R: f32 = 0.012;
const MARCH_STEPS: u32 = 72u;

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

// ----------------------------------------------------------------- SDFs

fn sd_sphere(p: vec3<f32>, c: vec3<f32>, r: f32) -> f32 {
    return length(p - c) - r;
}

fn sd_capsule(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

fn sd_box(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sd_ellipsoid(p: vec3<f32>, r: vec3<f32>) -> f32 {
    let k0 = length(p / r);
    let k1 = length(p / (r * r));
    return k0 * (k0 - 1.0) / max(k1, 1e-6);
}

fn sd_torus_y(p: vec3<f32>, c: vec3<f32>, big: f32, small: f32) -> f32 {
    let q = p - c;
    let d = vec2<f32>(length(q.xz) - big, q.y);
    return length(d) - small;
}

// The ship: the same fighter the pilot sits in (common.wgsl), scaled from
// metres to the map's unit size — about fourteen metres long, so a unit
// here is seven metres there.
fn sd_ship_local(q: vec3<f32>) -> f32 {
    return sd_fighter_exterior(q * 7.0) / 7.0;
}

fn to_ship(p: vec3<f32>) -> vec3<f32> {
    let d = p - map.ship.xyz;
    let s = max(map.ship.w, 1e-4);
    return vec3<f32>(dot(d, map.ship_right.xyz), dot(d, map.ship_up.xyz), -dot(d, map.ship_fwd.xyz)) / s;
}

struct Hit {
    d: f32,
    // 0 body (white), 1 sun, 2 ship, 3 pole, 4 dest ring, 5 uranus
    kind: f32,
}

fn pole(p: vec3<f32>, top: vec3<f32>) -> f32 {
    return sd_capsule(p, vec3<f32>(top.x, 0.0, top.z), top, POLE_R);
}

fn scene(p: vec3<f32>) -> Hit {
    var h = Hit(sd_sphere(p, map.planet.xyz, map.planet.w), 0.0);
    let dm = sd_sphere(p, map.moon.xyz, map.moon.w);
    if (dm < h.d) { h = Hit(dm, 0.0); }
    let ds = sd_sphere(p, map.sun.xyz, map.sun.w);
    if (ds < h.d) { h = Hit(ds, 1.0); }
    let du = sd_sphere(p, map.uranus.xyz, map.uranus.w);
    if (du < h.d) { h = Hit(du, 5.0); }
    let dship = sd_ship_local(to_ship(p)) * max(map.ship.w, 1e-4);
    if (dship < h.d) { h = Hit(dship, 2.0); }
    let dp = min(min(pole(p, map.moon.xyz), pole(p, map.sun.xyz)), min(pole(p, map.ship.xyz), pole(p, map.uranus.xyz)));
    if (dp < h.d) { h = Hit(dp, 3.0); }
    let dr = sd_torus_y(p, map.dest.xyz, map.dest.w, 0.008);
    if (dr < h.d) { h = Hit(dr, 4.0); }
    return h;
}

fn normal_at(p: vec3<f32>) -> vec3<f32> {
    let e = 0.002;
    let dx = scene(p + vec3<f32>(e, 0.0, 0.0)).d - scene(p - vec3<f32>(e, 0.0, 0.0)).d;
    let dy = scene(p + vec3<f32>(0.0, e, 0.0)).d - scene(p - vec3<f32>(0.0, e, 0.0)).d;
    let dz = scene(p + vec3<f32>(0.0, 0.0, e)).d - scene(p - vec3<f32>(0.0, 0.0, e)).d;
    return normalize(vec3<f32>(dx, dy, dz));
}

// ------------------------------------------------------------- the plane

fn ring2(p: vec2<f32>, c: vec2<f32>, r: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, abs(length(p - c) - r) - w);
}

// Light on the reference plane at xz: the grid, the rings around each
// body, the Moon's orbit, and a soft pool under each body.
fn plane_light(xz: vec2<f32>, aa: f32) -> vec3<f32> {
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    var glow = 0.0;
    var warn = 0.0;
    // Grid: one line per map unit (one per decade), fading with distance
    // from the planet so the far corners stay quiet.
    if (map.misc.w > 0.5) {
        let g = abs(fract(xz + 0.5) - 0.5);
        let line = 1.0 - smoothstep(0.0, aa * 1.5, min(g.x, g.y) - 0.004);
        glow += 0.10 * line * (1.0 - smoothstep(3.0, 7.0, length(xz)));
    }
    // Rings around the bodies: the planet's are the decades themselves;
    // the Moon's and the Sun's are tighter, a scale for their own hills.
    let n = i32(clamp(map.misc.y, 0.0, 6.0));
    for (var i = 0; i < n; i += 1) {
        let k = f32(i) + 1.0;
        glow += 0.16 * ring2(xz, map.planet.xz, k, 0.003, aa);
        glow += 0.10 * ring2(xz, map.moon.xz, 0.12 * k, 0.002, aa);
        glow += 0.10 * ring2(xz, map.sun.xz, 0.18 * k, 0.002, aa);
        glow += 0.10 * ring2(xz, map.uranus.xz, 0.14 * k, 0.002, aa);
    }
    // The Moon's orbit.
    glow += 0.30 * ring2(xz, map.planet.xz, map.misc.z, 0.003, aa);
    // Pools under the bodies, where their poles land.
    glow += 0.25 * (1.0 - smoothstep(0.0, 0.10, length(xz - map.moon.xz)));
    glow += 0.25 * (1.0 - smoothstep(0.0, 0.10, length(xz - map.ship.xz)));
    warn += 0.35 * (1.0 - smoothstep(0.0, 0.20, length(xz - map.sun.xz)));
    glow += 0.25 * (1.0 - smoothstep(0.0, 0.14, length(xz - map.uranus.xz)));
    // The destination's arrival ring, projected, and a dashed line from
    // the ship to it.
    warn += 0.5 * ring2(xz, map.dest.xz, max(map.dest.w, 0.03), 0.004, aa);
    let a = map.ship.xz;
    let b = map.dest.xz;
    let ab = b - a;
    let t = clamp(dot(xz - a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0);
    let seg = length(xz - (a + ab * t));
    let dash = step(0.5, fract(t * 24.0 - map.misc.x * 0.5));
    warn += 0.45 * (1.0 - smoothstep(0.0, aa, seg - 0.004)) * dash;
    return cyan * glow + amber * warn;
}

// ----------------------------------------------------------------- frame

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = map.eye.w;
    if (vis < 0.01) {
        discard;
    }
    // A gauge (the mini map) dims nothing round itself.
    let dim_on = map.right.w;
    let aspect = map.pane.w;
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);

    // The pane: inside it the map, outside it the dim.
    let local = (in.ndc - map.pane.xy) * vec2<f32>(aspect, 1.0);
    let half = vec2<f32>(map.pane.z * aspect);
    let box_d = max(abs(local.x) - half.x, abs(local.y) - half.y);
    let aa_ndc = max(fwidth(local.x), 1e-4) * 1.2;
    let inside = 1.0 - smoothstep(0.0, aa_ndc, box_d);
    let frame_w = 0.004;
    let edge = 1.0 - smoothstep(0.0, aa_ndc, abs(box_d) - frame_w);
    let corner = step(0.82, min(abs(local.x) / half.x, abs(local.y) / half.y));
    let frame = edge * (0.35 + 0.65 * corner);
    if (inside < 0.001 && frame < 0.001) {
        if (dim_on < 0.5) {
            discard;
        }
        return vec4<f32>(vec3<f32>(0.0), DIM_ALPHA * vis);
    }

    // A ray into the map from the orbiting camera.
    let uv = local / half.y;
    let tan_half = map.fwd.w;
    let ray = normalize(map.fwd.xyz + map.right.xyz * uv.x * tan_half + map.up.xyz * uv.y * tan_half);
    let eye = map.eye.xyz;

    // March the solids.
    var t = 0.0;
    var hit = Hit(1e9, -1.0);
    for (var i = 0u; i < MARCH_STEPS; i += 1u) {
        let p = eye + ray * t;
        let h = scene(p);
        if (h.d < 0.0015 * max(t, 0.5)) {
            hit = Hit(t, h.kind);
            break;
        }
        t += h.d;
        if (t > 40.0) {
            break;
        }
    }

    // The plane.
    var t_plane = -1.0;
    if (abs(ray.y) > 1e-5) {
        t_plane = -eye.y / ray.y;
    }

    var colour = vec3<f32>(0.0);
    var solid = 0.0;
    if (hit.kind >= 0.0) {
        let p = eye + ray * hit.d;
        let nrm = normal_at(p);
        let key = normalize(vec3<f32>(0.5, 0.8, 0.3));
        let lit = 0.35 + 0.65 * max(dot(nrm, key), 0.0);
        let rim = pow(1.0 - max(dot(nrm, -ray), 0.0), 3.0);
        if (hit.kind < 0.5) {
            colour = vec3<f32>(0.85, 0.9, 1.0) * lit + cyan * rim * 0.6;
        } else if (hit.kind < 1.5) {
            colour = amber * (0.8 + 0.4 * lit) + vec3<f32>(1.0, 0.9, 0.6) * rim;
        } else if (hit.kind < 2.5) {
            let pulse = 0.85 + 0.15 * sin(map.misc.x * 4.0);
            colour = cyan * (0.5 + 0.7 * lit) * pulse + vec3<f32>(1.0) * rim * 0.5;
        } else if (hit.kind < 3.5) {
            colour = cyan * 0.45;
        } else if (hit.kind < 4.5) {
            colour = amber * 0.9;
        } else {
            colour = vec3<f32>(0.56, 0.8, 0.88) * (0.4 + 0.7 * lit) + cyan * rim * 0.5;
        }
        solid = 1.0;
    }

    // The plane, over or under the solids depending on the order along the
    // ray; translucent either way, so what lies beneath it shows through.
    if (t_plane > 0.0 && (hit.kind < 0.0 || t_plane < hit.d)) {
        let xz = (eye + ray * t_plane).xz;
        let aa = max(fwidth(xz.x), 1e-4) * 1.2 + 0.002;
        let light = plane_light(xz, aa);
        colour = colour * 0.55 + light;
    } else if (t_plane > 0.0 && hit.kind >= 0.0) {
        // Solid in front of the plane: a hint of the plane still reads
        // through the poles (thin) but not through the bodies.
        let xz = (eye + ray * t_plane).xz;
        let aa = max(fwidth(xz.x), 1e-4) * 1.2 + 0.002;
        let through = select(0.0, 0.35, hit.kind > 2.5 && hit.kind < 3.5);
        colour += plane_light(xz, aa) * through;
    }

    let ground = vec3<f32>(0.01, 0.02, 0.04);
    let alpha = mix(DIM_ALPHA * dim_on, PANE_ALPHA, inside) * vis;
    let lit = (colour * inside + cyan * frame * 0.9) * vis;
    return vec4<f32>(ground * alpha * inside + lit, alpha);
}
