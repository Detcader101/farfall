// debris.wgsl — shards of rock (pass: debris)
//
// What a hit chips off and a break throws out: small pieces of the same
// rock, tumbling away on the rock's own velocity, lit by the Sun like
// the belt itself, their fresh faces glowing for a moment — white, then
// orange, then the dull red of cooling stone — and thinning out as they
// go. Each is an oriented box, ray-traced; a nearer rock hides it. Alpha
// over the belt, not light: these are solids.

struct Debris {
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // exposure, ring fill, shards in use, rocks in use
    look: vec4<f32>,
    // xyz sun (ship frame), w ember strength
    sun: vec4<f32>,
    // xyz at (ship frame, m), w half the longest side
    at: array<vec4<f32>, 64>,
    // xyz tumble axis, w angle
    tumble: array<vec4<f32>, 64>,
    // age over life, seed, -, -
    info: array<vec4<f32>, 64>,
    // xyz centre, w radius
    rocks: array<vec4<f32>, 48>,
}

@group(0) @binding(0) var<uniform> db: Debris;

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

const SHARDS: u32 = 64u;
const ROCKS: u32 = 48u;

fn rock_depth(ray: vec3<f32>) -> f32 {
    let n = u32(db.look.w);
    var best = 1e30;
    for (var i = 0u; i < ROCKS; i += 1u) {
        if (i >= n) { break; }
        let rk = db.rocks[i];
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

// Rodrigues: rotate v about the unit axis by the angle.
fn rotate(v: vec3<f32>, axis: vec3<f32>, ang: f32) -> vec3<f32> {
    let c = cos(ang);
    let s = sin(ang);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

// A ray from the origin against a box centred at c with half-sides h,
// posed by the tumble: the hit depth and the face normal (ship frame),
// or a negative depth for a miss.
fn hit_box(ray: vec3<f32>, c: vec3<f32>, h: vec3<f32>, axis: vec3<f32>, ang: f32) -> vec4<f32> {
    // Into the box's frame: undo the tumble.
    let o = rotate(-c, axis, -ang);
    let d = rotate(ray, axis, -ang);
    let inv = 1.0 / d;
    let t0 = (-h - o) * inv;
    let t1 = (h - o) * inv;
    let tmin = min(t0, t1);
    let tmax = max(t0, t1);
    let t_in = max(max(tmin.x, tmin.y), tmin.z);
    let t_out = min(min(tmax.x, tmax.y), tmax.z);
    if (t_in > t_out || t_out < 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    var n = vec3<f32>(0.0);
    if (t_in == tmin.x) { n = vec3<f32>(-sign(d.x), 0.0, 0.0); }
    else if (t_in == tmin.y) { n = vec3<f32>(0.0, -sign(d.y), 0.0); }
    else { n = vec3<f32>(0.0, 0.0, -sign(d.z)); }
    return vec4<f32>(rotate(n, axis, ang), t_in);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n = u32(db.look.z);
    if (n == 0u) {
        discard;
    }
    let aspect = db.right.w;
    let tan_half = db.up.w;
    let ray = normalize(db.fwd.xyz + db.right.xyz * (in.ndc.x * tan_half * aspect) + db.up.xyz * (in.ndc.y * tan_half));
    let sun = db.sun.xyz;

    var best_t = 1e30;
    var best_n = vec3<f32>(0.0);
    var best_i = 0u;
    for (var i = 0u; i < SHARDS; i += 1u) {
        if (i >= n) { break; }
        let a = db.at[i];
        let c = a.xyz;
        let size = a.w;
        // Cheap reject: a sphere about the box.
        let b = dot(ray, c);
        if (b < 0.0) { continue; }
        let r2 = size * size * 3.0;
        if (dot(c, c) - b * b > r2) { continue; }
        let seed = db.info[i].y;
        // A slab of a shape: longest side, then two shorter, by the seed.
        let h = vec3<f32>(size, size * (0.35 + 0.4 * seed), size * (0.25 + 0.35 * fract(seed * 7.3)));
        let tb = db.tumble[i];
        let hit = hit_box(ray, c, h, tb.xyz, tb.w);
        if (hit.w > 0.0 && hit.w < best_t) {
            best_t = hit.w;
            best_n = hit.xyz;
            best_i = i;
        }
    }
    if (best_t >= 1e29) {
        discard;
    }
    if (rock_depth(ray) < best_t) {
        discard;
    }
    let info = db.info[best_i];
    let age = info.x;
    let seed = info.y;
    let p = ray * best_t;
    // The same stone as the belt: grey-brown, grained.
    let light = max(dot(best_n, sun), 0.0);
    let fill = db.look.y * (0.35 + 0.65 * max(dot(best_n, -ray), 0.0));
    let grain = 0.85 + 0.3 * (vnoise(p * 3.0 + seed * 11.0) - 0.5);
    let albedo = vec3<f32>(0.32, 0.30, 0.27) * grain;
    var lit = albedo * (light * 1.6 + fill * 0.12);
    // The fresh face: hot at first, cooling through orange to a dull red
    // that dies. Fast, then slow.
    let heat = pow(max(1.0 - age * 1.6, 0.0), 1.5) * (0.6 + 0.6 * seed) * db.sun.w;
    let ember = mix(vec3<f32>(0.55, 0.06, 0.01), vec3<f32>(1.0, 0.55, 0.2), heat);
    let ember2 = mix(ember, vec3<f32>(1.0, 0.95, 0.85), heat * heat);
    // Only the fresh, broken face glows: the side away from the Sun's
    // old, and the pattern by the seed picks which faces.
    let face = 0.5 + 0.5 * sin(dot(best_n, vec3<f32>(1.3, 2.1, 0.7)) * 4.0 + seed * 20.0);
    lit += ember2 * heat * (0.3 + 0.7 * face) * 2.6;
    let colour = radiance(lit, db.look.x);
    // Thinning out at the end of the shard's life: it is gone.
    let alpha = 1.0 - smoothstep(0.8, 1.0, age);
    return vec4<f32>(colour * alpha + vec3<f32>(dither_px(in.pos.xy)) * alpha, alpha);
}
