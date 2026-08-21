// blit.wgsl — upscale the scene target to the swapchain, and the wormhole
// (SPEC §6.3).
//
// Lane: A. Cost class: trivial when the drive is idle (one fetch per output
// pixel, every effect term multiplied by zero and skipped); a few noise
// taps per pixel for the seconds a jump lasts.
//
// The scene renders at a fraction of native and is stretched here, while the
// HUD is drawn afterwards straight onto the swapchain at full resolution. That
// split is the point: shading cost scales with the scene's pixel count, but
// text and instruments stay pin-sharp at any render scale (P1).
//
// The wormhole sequence lives here too, because it is a thing done to the
// picture: the FLIP turns the view inside out through a mirror sphere —
// every direction reflected through a ball in front of the eye, centre to
// rim and rim to centre — and inverts its colours; the ARRIVAL sees the new
// place through gassy, watery particles, a refracting flow with droplets
// in it, that settle and clear.

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

struct Post {
    // x: fisheye 0..1, y: invert 0..1, z: particles 0..1, w: charge 0..1
    fx: vec4<f32>,
    // x: aspect, y: time s, zw: unused
    misc: vec4<f32>,
}
@group(0) @binding(2) var<uniform> post: Post;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // Clip space has +Y up, texture space has +V down.
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

// Reflection through a sphere: a point at radius r from the centre of the
// view lands at 1/r (scaled), so the centre goes to the rim and the rim
// comes to the centre — the world inside out. Blended by `f` so it can
// swell from nothing and back.
fn mirror_sphere(uv: vec2<f32>, aspect: f32, f: f32) -> vec2<f32> {
    let p = (uv - 0.5) * vec2<f32>(aspect, 1.0) * 2.0;
    let r = length(p);
    let dir = select(vec2<f32>(0.0, 1.0), p / r, r > 1e-4);
    let inverted = 0.45 / (r + 0.25);
    let r2 = mix(r, inverted, f);
    // A twist with it, so the flip reads as a turn through something.
    let a = f * 2.2;
    let rot = vec2<f32>(dir.x * cos(a) - dir.y * sin(a), dir.x * sin(a) + dir.y * cos(a));
    let q = rot * r2 / vec2<f32>(aspect, 1.0) * 0.5 + 0.5;
    return q;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let fisheye = post.fx.x;
    let invert = post.fx.y;
    let particles = post.fx.z;
    let charge = post.fx.w;
    let aspect = post.misc.x;
    let time = post.misc.y;

    if (fisheye + invert + particles + charge < 1e-4) {
        return textureSample(scene_tex, scene_sampler, in.uv);
    }

    var uv = in.uv;
    var beyond = 0.0;
    if (fisheye > 1e-4) {
        uv = mirror_sphere(uv, aspect, fisheye);
        // The very centre of the view reflects to beyond the picture's
        // rim: wrap what is there, and darken it, so the eye of the flip is
        // a dark well rather than a tiling of the edges.
        let q = (uv - 0.5) * vec2<f32>(aspect, 1.0) * 2.0;
        beyond = smoothstep(0.9, 1.8, length(q)) * fisheye;
        uv = fract(uv);
    }

    // Gassy water: a slow refracting flow, and droplets riding it.
    var droplets = 0.0;
    if (particles > 1e-4) {
        let flow = vec3<f32>(uv * 3.5, time * 0.35);
        let n = vec2<f32>(fbm3(flow) - 0.5, fbm3(flow + vec3<f32>(4.1, 7.3, 0.0)) - 0.5);
        uv += n * 0.10 * particles;
        // Droplets: one per cell, drifting up and fading as the gas clears.
        let cell_uv = (uv + vec2<f32>(0.0, time * 0.06)) * vec2<f32>(aspect * 14.0, 14.0);
        let cell = vec2<i32>(floor(cell_uv));
        let h = hash4(cell);
        let centre = vec2<f32>(cell) + h.xy;
        let d = length((cell_uv - centre) * vec2<f32>(1.0, 1.0));
        let radius = 0.08 + 0.25 * h.z * particles;
        let ring = 1.0 - smoothstep(radius * 0.6, radius, d);
        let rim = smoothstep(radius * 0.75, radius, d) * ring;
        droplets = (ring * 0.25 + rim * 0.9) * step(0.4, h.w) * particles;
    }

    var c = textureSample(scene_tex, scene_sampler, uv).rgb;
    // Charge: the picture bleeds toward white at the rim as the drive
    // winds up, chromatic at the edges.
    if (charge > 1e-4) {
        let p = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
        let rim = smoothstep(0.25, 0.9, length(p)) * charge;
        let shift = p * 0.012 * charge;
        let r = textureSample(scene_tex, scene_sampler, fract(uv + shift)).r;
        let b = textureSample(scene_tex, scene_sampler, fract(uv - shift)).b;
        c = vec3<f32>(r, c.g, b);
        c = mix(c, vec3<f32>(0.85, 0.95, 1.0), rim * 0.35);
    }
    c = mix(c, vec3<f32>(1.0) - c, invert);
    c = mix(c, vec3<f32>(0.02, 0.03, 0.06), beyond);
    c += vec3<f32>(0.75, 0.92, 1.0) * droplets;
    return vec4<f32>(c, 1.0);
}
