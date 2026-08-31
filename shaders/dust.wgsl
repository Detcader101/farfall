// dust.wgsl — space dust and cabin motes (pass: dust)
//
// Lane: A (vertex+fragment, instanced). Cost class: trivial — a couple of
// thousand small quads, most culled in the vertex stage.
//
// Fine motes and ice crystals in the volume about the eye: each instance
// is one mote, placed by hash on a lattice of world cells (REACH cells
// each way from the eye's own), so it holds still as the ship flies
// through and the same mote is in the same place next frame. Sun-lit ice
// glints — a twinkle as each crystal turns — with a forward-scattering
// halo when the Sun is ahead; the belt's grit is duller and warmer. The
// eye's velocity through the dust (relative to the local orbit, so a
// coasting ship sees them hang) draws each into a streak one frame long.
// Dense in the belt and in a planet's air, sparse in deep space (the
// density gates which hashes show), gone under the hyper field.
//
// The cabin's own motes (a second instance range) drift slowly in the
// ship's frame between the head and the dash, in the dash's cyan light —
// drawn after the cabin so they float in front of it.
//
// Additive: light, not paint. Written as radiance; the post pass owns
// exposure and bloom.

struct Dust {
    // The camera's basis in the world frame; w: aspect, tan(fov/2), time
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // xyz: the eye's cell on the lattice
    cell: vec4<i32>,
    // xyz: the eye's offset within its cell (m); w: the cell size (m)
    frac: vec4<f32>,
    // xyz: the eye's velocity through the dust (m/s); w: streak exposure (s)
    vel: vec4<f32>,
    // xyz: the Sun's direction (world); w: strength
    sun: vec4<f32>,
    // x: density 0..1, y: brightness, z: exposure, w: target height (px)
    look: vec4<f32>,
    // xyz: an opaque body from the eye (world, m); w: radius (0 none)
    occluder: vec4<f32>,
    // The view's basis in the ship frame (cabin motes); right.w: cabin
    // motes on; fwd.w: the cabin's light
    cright: vec4<f32>,
    cup: vec4<f32>,
    cfwd: vec4<f32>,
}

@group(0) @binding(0) var<uniform> du: Dust;

const REACH: i32 = 3;
const SIDE: u32 = 7u;
const PER_CELL: u32 = 12u;
const SPACE_MOTES: u32 = 7u * 7u * 7u * 12u;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Capsule coordinates in pixels: x along the streak, y across
    @location(0) uv: vec2<f32>,
    // x: half the streak's length (px), y: its radius (px)
    @location(1) @interpolate(flat) shape: vec2<f32>,
    @location(2) @interpolate(flat) colour: vec3<f32>,
}

fn cull() -> VsOut {
    var out: VsOut;
    // Outside the clip volume: nothing rasterises.
    out.pos = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.shape = vec2<f32>(0.0, 1.0);
    out.colour = vec3<f32>(0.0);
    return out;
}

// A camera-space point (x right, y up, z depth) to pixels about the
// screen's centre.
fn to_px(c: vec3<f32>, tan_half: f32, aspect: f32, half_h: f32) -> vec2<f32> {
    let z = max(c.z, 0.05);
    return vec2<f32>(c.x / (z * tan_half * aspect), c.y / (z * tan_half)) * half_h;
}

// The quad for a capsule between two pixel points with radius r: the
// corner for this vertex, its capsule coordinates.
fn capsule_vertex(vi: u32, p1: vec2<f32>, p2: vec2<f32>, r: f32, half_h: f32, aspect: f32, colour: vec3<f32>) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let seg = p2 - p1;
    let len = length(seg);
    let dir = select(seg / len, vec2<f32>(1.0, 0.0), len < 1e-3);
    let perp = vec2<f32>(-dir.y, dir.x);
    let half = len * 0.5;
    let centre = (p1 + p2) * 0.5;
    let k = corners[vi];
    let uv = vec2<f32>(k.x * (half + r), k.y * r);
    let px = centre + dir * uv.x + perp * uv.y;
    var out: VsOut;
    out.pos = vec4<f32>(px.x / (half_h * aspect), px.y / half_h, 0.5, 1.0);
    out.uv = uv;
    out.shape = vec2<f32>(half, r);
    out.colour = colour;
    return out;
}

fn space_mote(vi: u32, ii: u32) -> VsOut {
    let cell_lin = ii / PER_CELL;
    let j = i32(ii % PER_CELL);
    let cx = i32(cell_lin % SIDE) - REACH;
    let cy = i32((cell_lin / SIDE) % SIDE) - REACH;
    let cz = i32(cell_lin / (SIDE * SIDE)) - REACH;
    let w = du.cell.xyz + vec3<i32>(cx, cy, cz);
    let zj = w.z * 16 + j;
    let h1 = hash31(vec3<i32>(w.x, w.y, zj));
    let h2 = hash31(vec3<i32>(w.y + 1013, zj, w.x));
    let h3 = hash31(vec3<i32>(zj + 77, w.x, w.y));
    let h4 = hash31(vec3<i32>(w.x + 31, w.y + 17, zj));
    let h5 = hash31(vec3<i32>(w.x * 3 + 5, w.y * 5 + 1, zj + 9));
    // The density gates which motes exist here at all.
    if (h5 > du.look.x) {
        return cull();
    }
    let cell = du.frac.w;
    let p = (vec3<f32>(f32(cx), f32(cy), f32(cz)) + vec3<f32>(h1, h2, h3)) * cell - du.frac.xyz;
    let dist = length(p);
    let reach_m = f32(REACH) * cell;
    if (dist < 0.6 || dist > reach_m) {
        return cull();
    }
    let ray = p / dist;
    // Behind an opaque body: gone.
    if (du.occluder.w > 0.0) {
        let oc = du.occluder.xyz;
        let b = dot(ray, oc);
        let disc = b * b - (dot(oc, oc) - du.occluder.w * du.occluder.w);
        if (disc > 0.0) {
            let t_in = b - sqrt(disc);
            if (t_in > 0.0 && t_in < dist) {
                return cull();
            }
        }
    }
    let tan_half = du.up.w;
    let aspect = du.right.w;
    let half_h = du.look.w * 0.5;
    let c1 = vec3<f32>(dot(p, du.right.xyz), dot(p, du.up.xyz), dot(p, du.fwd.xyz));
    // Where it was a frame ago: the streak's other end.
    let p2 = p + du.vel.xyz * du.vel.w;
    let c2 = vec3<f32>(dot(p2, du.right.xyz), dot(p2, du.up.xyz), dot(p2, du.fwd.xyz));
    if (c1.z < 0.2 && c2.z < 0.2) {
        return cull();
    }
    var px1 = to_px(c1, tan_half, aspect, half_h);
    var px2 = to_px(c2, tan_half, aspect, half_h);
    // A streak longer than the screen is a line through it: cap it.
    let seg = px2 - px1;
    let len = length(seg);
    let cap = half_h * 1.2;
    if (len > cap) {
        px2 = px1 + seg * (cap / len);
    }
    let mid = (px1 + px2) * 0.5;
    if (abs(mid.x) > half_h * aspect + cap || abs(mid.y) > half_h + cap) {
        return cull();
    }
    // Size: a crystal a few centimetres across, never under a pixel and
    // a half; what is under a pixel keeps a pixel's share of light.
    let size_m = 0.02 + 0.06 * h4;
    let size_px = size_m / (dist * tan_half) * half_h;
    let r = clamp(size_px, 1.6, 5.0);
    let share = clamp(size_px / 1.6, 0.5, 1.0);
    // Light: the Sun on ice, a glint as the crystal turns, a halo of
    // forward scatter when the Sun is ahead; grit is dull and warm.
    let now = du.fwd.w;
    let grit = h3 < 0.35;
    let twinkle = pow(0.5 + 0.5 * sin(now * (3.0 + 9.0 * h4) + h2 * 40.0), 6.0);
    let toward_sun = max(dot(ray, du.sun.xyz), 0.0);
    let halo = 0.55 + 2.5 * pow(toward_sun, 6.0);
    let fade = 1.0 - smoothstep(0.7, 1.0, dist / reach_m);
    let near = smoothstep(0.6, 2.5, dist);
    var bright = select(0.7 + 2.4 * twinkle, 0.35, grit) * halo * du.sun.w;
    // A streak spreads its light along its length: dimmer per pixel.
    let half = min(len, cap) * 0.5;
    bright *= clamp((r + 2.0) / (r + 2.0 + half * 0.25), 0.35, 1.0);
    bright *= fade * near * share * du.look.y;
    let ice = mix(vec3<f32>(0.80, 0.90, 1.00), vec3<f32>(1.0, 0.96, 0.90), pow(toward_sun, 3.0));
    let colour = select(ice, vec3<f32>(0.75, 0.62, 0.48), grit) * bright;
    return capsule_vertex(vi, px1, px2, r, half_h, aspect, colour);
}

fn cabin_mote(vi: u32, ii: u32) -> VsOut {
    if (du.cright.w < 0.5) {
        return cull();
    }
    let j = i32(ii);
    let h1 = hash31(vec3<i32>(j, 7, 3));
    let h2 = hash31(vec3<i32>(j + 91, 5, 11));
    let h3 = hash31(vec3<i32>(13, j + 17, 2));
    let h4 = hash31(vec3<i32>(23, 29, j + 41));
    let now = du.fwd.w;
    // The room between the head and the dash, ship frame; a slow drift.
    var p = vec3<f32>(-0.55 + 1.1 * h1, -0.42 + 0.78 * h2, -1.05 + 0.75 * h3);
    p += 0.05 * vec3<f32>(
        sin(now * 0.23 + h1 * 30.0),
        0.6 * sin(now * 0.17 + h2 * 30.0),
        cos(now * 0.19 + h3 * 30.0),
    );
    let c = vec3<f32>(dot(p, du.cright.xyz), dot(p, du.cup.xyz), dot(p, du.cfwd.xyz));
    if (c.z < 0.12) {
        return cull();
    }
    let tan_half = du.up.w;
    let aspect = du.right.w;
    let half_h = du.look.w * 0.5;
    let px = to_px(c, tan_half, aspect, half_h);
    if (abs(px.x) > half_h * aspect + 8.0 || abs(px.y) > half_h + 8.0) {
        return cull();
    }
    let r = clamp(0.0016 / (c.z * tan_half) * half_h, 1.5, 3.5);
    let twinkle = 0.5 + 0.5 * sin(now * (0.8 + 1.5 * h4) + h1 * 50.0);
    let light = du.cfwd.w;
    let bright = (0.35 + 0.65 * twinkle) * light * du.look.y * (0.4 + 0.6 * h4);
    let colour = vec3<f32>(0.45, 0.85, 1.0) * bright;
    return capsule_vertex(vi, px, px, r, half_h, aspect, colour);
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    if (ii < SPACE_MOTES) {
        return space_mote(vi, ii);
    }
    return cabin_mote(vi, ii - SPACE_MOTES);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let half = in.shape.x;
    let r = in.shape.y;
    // Distance to the streak's spine, a capsule.
    let d = length(vec2<f32>(max(abs(in.uv.x) - half, 0.0), in.uv.y));
    let a = 1.0 - smoothstep(0.0, r, d);
    if (a <= 0.001) {
        discard;
    }
    // A soft skirt and a sharp core: a point of light, not a disc.
    let skirt = a * a;
    let core = pow(a, 6.0);
    return vec4<f32>(in.colour * (skirt * 0.45 + core * 1.1), 0.0);
}
