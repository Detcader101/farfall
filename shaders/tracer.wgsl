// tracer.wgsl — the arms' light (pass: tracer)
//
// Slugs in the air: a white-hot head with a tail of light behind it — the
// cannon's tracers burn orange and gutter, the rail's slug drags a violet
// plasma wake. Muzzle flashes at the wings and the nose. And where a slug
// lands: a spray of sparks off the rock, a ring of shocked dust; a rock
// that goes leaves a cloud of grit and embers that drifts apart and cools.
// All additive light, ray-cast from the head; anything past a rock's face
// along the ray is hidden by it.

const SLUGS: u32 = 32u;
const BURSTS: u32 = 16u;
const LIVE: u32 = 48u;

struct Tracer {
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // exposure, glow, slugs in use, bursts in use
    look: vec4<f32>,
    // xyz sun (ship frame), w rocks in use
    sun: vec4<f32>,
    // xyz head (ship frame, m), w kind (0 cannon, 1 rail)
    heads: array<vec4<f32>, 32>,
    // xyz tail, w age (s)
    tails: array<vec4<f32>, 32>,
    // xyz where, w age (s)
    bursts: array<vec4<f32>, 16>,
    // size, kind (0 flash, 1 hit, 2 break, 3 rail hit), seed, -
    binfo: array<vec4<f32>, 16>,
    // occluders: xyz centre, w radius
    rocks: array<vec4<f32>, 48>,
}

@group(0) @binding(0) var<uniform> tr: Tracer;

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

// The nearest rock face along the ray from the head, or a very long way.
fn rock_depth(ray: vec3<f32>) -> f32 {
    let n = u32(tr.sun.w);
    var best = 1e30;
    for (var i = 0u; i < LIVE; i += 1u) {
        if (i >= n) { break; }
        let rk = tr.rocks[i];
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

// Closest approach between the ray (from the origin) and the segment a-b:
// returns (distance, depth along the ray, fraction along the segment).
fn ray_segment(ray: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> vec3<f32> {
    let u = b - a;
    let uu = dot(u, u);
    let ur = dot(u, ray);
    let ar = dot(a, ray);
    let au = dot(a, u);
    let den = uu - ur * ur;
    var s = 0.0;
    if (den > 1e-6) {
        s = clamp((ur * ar - au) / den, 0.0, 1.0);
    }
    let p = a + u * s;
    let t = max(dot(p, ray), 0.0);
    let d = length(p - ray * t);
    return vec3<f32>(d, t, s);
}

// A glow that reads at any range: its width grows with the depth so a
// slug a kilometre out is still a line, not a subpixel.
fn width_at(t: f32, w0: f32, tan_half: f32) -> f32 {
    return w0 + t * tan_half * 0.0025;
}

fn slug_light(ray: vec3<f32>, i: u32, now: f32, tan_half: f32, rock_t: f32) -> vec3<f32> {
    let head = tr.heads[i].xyz;
    let kind = tr.heads[i].w;
    let tail = tr.tails[i].xyz;
    let age = tr.tails[i].w;
    let rs = ray_segment(ray, tail, head);
    let d = rs.x;
    let t = rs.y;
    let s = rs.z;
    if (t > rock_t) { return vec3<f32>(0.0); }
    let is_rail = kind > 0.5;
    let w = width_at(t, select(0.10, 0.28, is_rail), tan_half);
    let core = exp(-(d * d) / (w * w * 0.35));
    let halo = exp(-d / (w * 2.0)) * 0.16;
    // Along the tail: brightest at the head, guttering behind.
    let along = s * s;
    let flicker = 0.75 + 0.25 * sin(now * 90.0 + s * 40.0 + f32(i) * 3.1);
    // A slug just off the muzzle is beside the glass: it must not wash
    // the whole canopy out.
    let near = smoothstep(3.0, 22.0, t);
    var col = vec3<f32>(0.0);
    if (is_rail) {
        let hot = vec3<f32>(0.85, 0.92, 1.0);
        let wake = vec3<f32>(0.55, 0.30, 1.0);
        let wisp = 0.6 + 0.4 * vnoise(vec3<f32>(s * 30.0, f32(i), now * 7.0));
        col = hot * core * (0.4 + 0.6 * along) * 2.4 + wake * (halo + core * 0.5) * wisp * (0.3 + 0.7 * (1.0 - along)) * 1.4;
        let birth = clamp(age / 0.05, 0.0, 1.0);
        col *= birth;
    } else {
        let hot = vec3<f32>(1.0, 0.95, 0.8);
        let ember = vec3<f32>(1.0, 0.45, 0.10);
        col = hot * core * (0.3 + 0.7 * along) * 1.8 + ember * (halo + core * 0.4) * flicker * (0.4 + 0.6 * along);
    }
    // Fade with age a little: the tracer's compound burns out.
    let burn = clamp(1.4 - age * 0.25, 0.4, 1.0);
    return col * burn * near;
}

// Sparks: lines of light flying out from a point, each its own length
// and brightness, from a hash of its direction — not a lattice.
fn spray(rel: vec3<f32>, r: f32, seed: f32, age: f32, lines: f32) -> f32 {
    let len = length(rel);
    if (len < 1e-3 || len > r) { return 0.0; }
    let dir = rel / len;
    // Two noise fields on the sphere of directions, at scales that do not
    // share a grid: their product has no visible cells.
    let a = vnoise(dir * lines + vec3<f32>(seed * 61.0));
    let b = vnoise(dir * lines * 1.71 + vec3<f32>(seed * 23.0, 7.0, 0.0));
    let streak = pow(a * b, 5.0) * 60.0;
    // Each streak reaches a different distance; they are brightest at
    // their tips and thin toward the point.
    let reach = r * (0.35 + 0.65 * a);
    let along = len / reach;
    let tip = smoothstep(0.0, 0.4, along) * (1.0 - smoothstep(0.85, 1.0, along));
    return streak * tip * (0.5 + 0.5 * along);
}

fn burst_light(ray: vec3<f32>, i: u32, now: f32, tan_half: f32, rock_t: f32) -> vec3<f32> {
    let at = tr.bursts[i].xyz;
    let age = tr.bursts[i].w;
    let size = tr.binfo[i].x;
    let kind = tr.binfo[i].y;
    let seed = tr.binfo[i].z;
    let t = max(dot(at, ray), 0.0);
    if (t > rock_t + size * 6.0) { return vec3<f32>(0.0); }
    let d = length(at - ray * t);
    let dist = length(at);
    // The point on the ray nearest the burst, relative to it: for noise.
    let rel = ray * t - at;
    let near = smoothstep(1.0, 6.0, dist);
    if (kind < 0.5) {
        // A muzzle flash: a hard bloom for a few hundredths of a second,
        // with rays of it.
        let life = 0.09;
        let k = clamp(1.0 - age / life, 0.0, 1.0);
        let r = size * (0.6 + 0.4 * (1.0 - k)) + t * tan_half * 0.01;
        let bloom = exp(-(d * d) / (r * r)) * k * k;
        let dir = normalize(rel + vec3<f32>(1e-4));
        let spikes = pow(0.5 + 0.5 * sin(atan2(dir.y, dir.x) * 7.0 + seed * 40.0), 6.0);
        let rays = exp(-d / (r * 3.0)) * spikes * k * 0.6;
        return vec3<f32>(1.0, 0.85, 0.6) * (bloom * 4.0 + rays) * near;
    }
    if (kind > 1.5 && kind < 2.5) {
        // A rock coming apart: a flash, then a cloud of grit and embers
        // swelling out and cooling, chunks of it flung ahead as streaks,
        // a shock ring running out ahead of it all.
        let life = 1.6;
        let a = age / life;
        let k = clamp(1.0 - a, 0.0, 1.0);
        let r = size * 24.0 * (0.15 + 0.85 * (1.0 - exp(-age * 2.5)));
        let flash = exp(-(d * d) / (size * size * 8.0)) * exp(-age * 10.0) * 6.0;
        let q = rel / max(r, 0.1);
        let grit = fbm3(q * 2.5 + vec3<f32>(seed * 50.0, now * 0.3, 0.0));
        let cloud = exp(-(d * d) / (r * r * 0.7)) * (0.25 + 0.75 * grit) * k * k * 2.2;
        let chunks = spray(rel, r * 1.6, seed, age, 5.0) * k * 1.2;
        let embers = pow(vnoise(q * 9.0 + vec3<f32>(seed * 90.0)) * vnoise(q * 15.7 + vec3<f32>(3.0, seed * 40.0, 1.0)), 5.0) * exp(-(d * d) / (r * r)) * k * 40.0;
        let ring_r = size * 34.0 * (1.0 - exp(-age * 3.0)) + size * 2.0;
        let ring = exp(-((d - ring_r) * (d - ring_r)) / (size * size * 6.0)) * exp(-age * 2.0) * 0.6;
        let hot = vec3<f32>(1.0, 0.72, 0.42);
        let cool = vec3<f32>(0.50, 0.38, 0.30);
        let cloud_col = mix(hot, cool, clamp(a * 2.0, 0.0, 1.0));
        return (vec3<f32>(1.0, 0.9, 0.8) * flash + cloud_col * cloud + vec3<f32>(1.0, 0.55, 0.2) * (embers + chunks) + cool * ring) * near;
    }
    // A hit: sparks spraying off the face, a puff of dust; the rail's is
    // bigger and bluer.
    let rail = kind > 2.5;
    let life = select(0.55, 0.8, rail);
    let k = clamp(1.0 - age / life, 0.0, 1.0);
    let r = size * select(7.0, 14.0, rail) * (0.2 + 0.8 * (1.0 - exp(-age * 6.0))) + t * tan_half * 0.004;
    let flash = exp(-(d * d) / (size * size * select(1.5, 4.0, rail))) * exp(-age * 18.0) * 4.0;
    let sparks = spray(rel, r, seed, age, 9.0) * k * 1.5;
    let q = rel / max(r, 0.1);
    let dust = exp(-(d * d) / (r * r * 1.2)) * (0.4 + 0.6 * fbm3(q * 3.0 + seed * 20.0)) * k * k * 0.5;
    let spark_col = select(vec3<f32>(1.0, 0.6, 0.2), vec3<f32>(0.7, 0.8, 1.0), rail);
    let flash_col = select(vec3<f32>(1.0, 0.8, 0.6), vec3<f32>(0.85, 0.9, 1.0), rail);
    let dust_col = vec3<f32>(0.5, 0.42, 0.36);
    let range = clamp(200.0 / max(dist, 1.0), 0.3, 1.0);
    return (flash_col * flash + spark_col * sparks + dust_col * dust) * range * near;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let n_slugs = u32(tr.look.z);
    let n_bursts = u32(tr.look.w);
    if (n_slugs == 0u && n_bursts == 0u) {
        discard;
    }
    let aspect = tr.right.w;
    let tan_half = tr.up.w;
    let ray = normalize(tr.fwd.xyz + tr.right.xyz * (in.ndc.x * tan_half * aspect) + tr.up.xyz * (in.ndc.y * tan_half));
    let now = tr.fwd.w;
    let rock_t = rock_depth(ray);
    var col = vec3<f32>(0.0);
    for (var i = 0u; i < SLUGS; i += 1u) {
        if (i >= n_slugs) { break; }
        col += slug_light(ray, i, now, tan_half, rock_t);
    }
    for (var i = 0u; i < BURSTS; i += 1u) {
        if (i >= n_bursts) { break; }
        col += burst_light(ray, i, now, tan_half, rock_t);
    }
    let glow = tr.look.y;
    col *= glow * tr.look.x;
    if (max(max(col.r, col.g), col.b) < 0.002) {
        discard;
    }
    return vec4<f32>(col, 1.0);
}
