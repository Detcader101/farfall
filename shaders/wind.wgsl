// wind.wgsl — streaming wind ribbons in a planet's air (pass: wind)
//
// Lane: A (vertex+fragment, instanced). Cost class: trivial — ~1250 thin
// quads, most culled in the vertex stage, nothing at all above the air.
//
// The wind made visible: each instance is one ribbon of moving air,
// hash-placed on a lattice of world cells about the eye (so it holds its
// place as the ship flies through), stretched along the LOCAL wind vector
// and ridden by a bright packet that runs downwind — direction readable
// at a glance, length and brightness growing with the wind's strength,
// more of them in a gust, all of it fading with the air itself. The two
// wind samples (at the eye, and a gap above it) come from the sim's own
// field on the CPU; this shader only interpolates between them — one
// source of truth, never a second implementation of the weather.
//
// Additive: light, not paint. Written as radiance; the post pass owns
// exposure and bloom.

struct Wind {
    // The camera's basis in the world frame; w: aspect, tan(fov/2), time
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // xyz: the eye's cell on the lattice
    cell: vec4<i32>,
    // xyz: the eye's offset within its cell (m); w: the cell size (m)
    frac: vec4<f32>,
    // xyz: the sim's wind at the eye (m/s, world); w: the air 0..1
    wlow: vec4<f32>,
    // xyz: the sim's wind a gap above the eye; w: the gap (m)
    whigh: vec4<f32>,
    // xyz: the planet's up at the eye (world); w: the WIND setting 0..2
    upw: vec4<f32>,
    // x: ribbon density 0..1, y: brightness, z: wind speed / 60 (0..1+),
    // w: target height (px)
    look: vec4<f32>,
}

@group(0) @binding(0) var<uniform> wu: Wind;

const REACH: i32 = 2;
const SIDE: u32 = 5u;
const PER_CELL: u32 = 10u;
const RIBBONS: u32 = 5u * 5u * 5u * 10u;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // Capsule coordinates in pixels: x along the ribbon, y across
    @location(0) uv: vec2<f32>,
    // x: half the ribbon's length (px), y: its radius (px)
    @location(1) @interpolate(flat) shape: vec2<f32>,
    @location(2) @interpolate(flat) colour: vec3<f32>,
    // x: the packet's centre along the ribbon (px), y: its sigma (px)
    @location(3) @interpolate(flat) flow: vec2<f32>,
}

fn cull() -> VsOut {
    var out: VsOut;
    out.pos = vec4<f32>(0.0, 0.0, 2.0, 1.0);
    out.uv = vec2<f32>(0.0);
    out.shape = vec2<f32>(0.0, 1.0);
    out.colour = vec3<f32>(0.0);
    out.flow = vec2<f32>(0.0, 1.0);
    return out;
}

// A camera-space point (x right, y up, z depth) to pixels about the
// screen's centre.
fn to_px(c: vec3<f32>, tan_half: f32, aspect: f32, half_h: f32) -> vec2<f32> {
    let z = max(c.z, 0.05);
    return vec2<f32>(c.x / (z * tan_half * aspect), c.y / (z * tan_half)) * half_h;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VsOut {
    if (wu.look.x <= 0.0 || wu.wlow.w <= 0.0) {
        return cull();
    }
    let cell_lin = ii / PER_CELL;
    let j = i32(ii % PER_CELL);
    let cx = i32(cell_lin % SIDE) - REACH;
    let cy = i32((cell_lin / SIDE) % SIDE) - REACH;
    let cz = i32(cell_lin / (SIDE * SIDE)) - REACH;
    let w = wu.cell.xyz + vec3<i32>(cx, cy, cz);
    let zj = w.z * 16 + j;
    let h1 = hash31(vec3<i32>(w.x, w.y, zj));
    let h2 = hash31(vec3<i32>(w.y + 1013, zj, w.x));
    let h3 = hash31(vec3<i32>(zj + 77, w.x, w.y));
    let h4 = hash31(vec3<i32>(w.x + 31, w.y + 17, zj));
    let h5 = hash31(vec3<i32>(w.x * 3 + 5, w.y * 5 + 1, zj + 9));
    // The density gates which ribbons exist here at all.
    if (h5 > wu.look.x) {
        return cull();
    }
    let cell = wu.frac.w;
    let p = (vec3<f32>(f32(cx), f32(cy), f32(cz)) + vec3<f32>(h1, h2, h3)) * cell - wu.frac.xyz;
    let dist = length(p);
    let reach_m = f32(REACH) * cell;
    if (dist < 2.0 || dist > reach_m) {
        return cull();
    }
    // The local wind: the sim's two samples, interpolated by how far this
    // ribbon sits above or below the eye. Never a second wind model.
    let gap = max(wu.whigh.w, 1.0);
    let above = clamp(dot(p, wu.upw.xyz) / gap, -1.0, 1.0);
    let wind = mix(wu.wlow.xyz, wu.whigh.xyz, above * 0.5 + 0.5);
    let speed = length(wind);
    if (speed < 0.5) {
        return cull();
    }
    let dir = wind / speed;
    // Length rides the strength: a breeze is a fleck, the jet a streamer.
    let len_m = clamp(speed * 1.2, 4.0, 90.0);
    let p1 = p - dir * len_m * 0.5;
    let p2 = p + dir * len_m * 0.5;
    let tan_half = wu.up.w;
    let aspect = wu.right.w;
    let half_h = wu.look.w * 0.5;
    let c1 = vec3<f32>(dot(p1, wu.right.xyz), dot(p1, wu.up.xyz), dot(p1, wu.fwd.xyz));
    let c2 = vec3<f32>(dot(p2, wu.right.xyz), dot(p2, wu.up.xyz), dot(p2, wu.fwd.xyz));
    if (c1.z < 0.2 && c2.z < 0.2) {
        return cull();
    }
    var px1 = to_px(c1, tan_half, aspect, half_h);
    var px2 = to_px(c2, tan_half, aspect, half_h);
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
    // Geometry: a thin capsule between the two ends.
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let seg2 = px2 - px1;
    let len2 = length(seg2);
    let sdir = select(seg2 / len2, vec2<f32>(1.0, 0.0), len2 < 1e-3);
    let perp = vec2<f32>(-sdir.y, sdir.x);
    let half = len2 * 0.5;
    let centre = (px1 + px2) * 0.5;
    let r = clamp(0.10 / (dist * tan_half) * half_h, 1.0, 2.6);
    let k = corners[vi];
    let uv = vec2<f32>(k.x * (half + r), k.y * r);
    let px = centre + sdir * uv.x + perp * uv.y;
    // The packet: a bright pulse running downwind along the ribbon, each
    // ribbon on its own phase, faster air running visibly faster.
    let now = wu.fwd.w;
    let phase = fract(now * (0.25 + wu.look.z * 0.9) + h4);
    let head = (phase * 2.0 - 1.0) * half;
    // Light: brighter in strong wind, faded by distance and thin air.
    let fade = 1.0 - smoothstep(0.65, 1.0, dist / reach_m);
    let near = smoothstep(2.0, 12.0, dist);
    let bright = (0.25 + 0.9 * clamp(wu.look.z, 0.0, 1.2))
        * wu.wlow.w * fade * near * wu.look.y;
    let colour = vec3<f32>(0.62, 0.86, 1.0) * bright;
    var out: VsOut;
    out.pos = vec4<f32>(px.x / (half_h * aspect), px.y / half_h, 0.5, 1.0);
    out.uv = uv;
    out.shape = vec2<f32>(half, r);
    out.colour = colour;
    out.flow = vec2<f32>(head, max(half * 0.35, 4.0));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let half = in.shape.x;
    let r = in.shape.y;
    // Distance to the ribbon's spine, a capsule.
    let d = length(vec2<f32>(max(abs(in.uv.x) - half, 0.0), in.uv.y));
    let a = 1.0 - smoothstep(0.0, r, d);
    if (a <= 0.001) {
        discard;
    }
    // A faint full-length thread, a comet-bright head toward downwind,
    // and the packet running along it: the direction is the picture.
    let along = clamp(in.uv.x / max(half, 1.0), -1.0, 1.0);
    let comet = 0.35 + 0.65 * (along * 0.5 + 0.5) * (along * 0.5 + 0.5);
    let dp = (in.uv.x - in.flow.x) / in.flow.y;
    let packet = exp(-dp * dp);
    let body = a * a * (comet * 0.5 + packet * 1.4);
    return vec4<f32>(in.colour * body, 0.0);
}
