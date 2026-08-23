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
// rim and rim to centre — and inverts its colours; the CHARGE and the
// ARRIVAL, and the Chaos Drive's field, are the quantum superstate seen
// through: a liquid, vortical refraction of the view — a few drifting
// vortices with fine ripples riding them, all analytic (no noise, so it
// costs a handful of operations a pixel at any resolution) — with
// chromatic splitting, radial speed streaks and a cool vignette that say
// "fast" without the field of view having to.

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

struct Post {
    // x: fisheye 0..1, y: invert 0..1, z: flow 0..1 (the liquid field),
    // w: charge 0..1
    fx: vec4<f32>,
    // x: aspect, y: time s, z: speed 0..1 (streaks, the cool rim), w: unused
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

// The liquid: a few vortices drifting about the view, each a swirl that
// falls off with distance, with a fine ripple running round it. Returns
// the displacement of the picture, in aspect-corrected view units.
fn liquid(p: vec2<f32>, t: f32, amount: f32) -> vec2<f32> {
    var off = vec2<f32>(0.0);
    for (var i = 0; i < 4; i += 1) {
        let fi = f32(i);
        // Each vortex wanders on its own slow ellipse.
        let c = vec2<f32>(
            0.30 * cos(t * (0.23 + 0.07 * fi) + fi * 1.9),
            0.22 * sin(t * (0.19 + 0.05 * fi) + fi * 2.6),
        );
        let d = p - c;
        let r = length(d);
        let perp = vec2<f32>(-d.y, d.x) / max(r, 0.03);
        let fall = exp(-r * r / 0.09);
        // The swirl, turning one way then the other, and the ripple: a
        // fine wave running out from the eye of it.
        let swirl = sin(t * 1.3 + fi * 1.1) * 0.045;
        let ripple = sin(r * 60.0 - t * 7.0 + fi) * 0.004;
        off += perp * (swirl * fall) + d / max(r, 0.03) * (ripple * fall);
    }
    return off * amount;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let fisheye = post.fx.x;
    let invert = post.fx.y;
    let flow = post.fx.z;
    let charge = post.fx.w;
    let aspect = post.misc.x;
    let time = post.misc.y;
    let speed = post.misc.z;

    if (fisheye + invert + flow + charge + speed < 1e-4) {
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

    // The liquid field: the charge and the flow both run it, the flow
    // harder; the speed adds a faint one of its own.
    let field = max(flow, max(charge * 0.6, speed * 0.35));
    let p = (uv - 0.5) * vec2<f32>(aspect, 1.0);
    var off = vec2<f32>(0.0);
    if (field > 1e-4) {
        off = liquid(p, time, 1.0);
        uv += off * field / vec2<f32>(aspect, 1.0);
    }
    uv = clamp(uv, vec2<f32>(0.001), vec2<f32>(0.999));

    // Speed: radial streaks — a few taps out along the line from the
    // centre, longer at the rim — and a chromatic split of the same.
    let q = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let rq = length(q);
    let radial = select(vec2<f32>(0.0), q / rq, rq > 1e-4) / vec2<f32>(aspect, 1.0);
    let split = (0.004 * charge + 0.010 * speed) * rq;
    var c: vec3<f32>;
    if (speed > 1e-4) {
        let reach = 0.05 * speed * rq;
        var acc = vec3<f32>(0.0);
        for (var k = 0; k < 4; k += 1) {
            let s = (f32(k) / 3.0 - 0.5) * reach;
            let u = clamp(uv + radial * s, vec2<f32>(0.001), vec2<f32>(0.999));
            acc += vec3<f32>(
                textureSample(scene_tex, scene_sampler, u + radial * split).r,
                textureSample(scene_tex, scene_sampler, u).g,
                textureSample(scene_tex, scene_sampler, u - radial * split).b,
            );
        }
        c = acc / 4.0;
    } else if (charge > 1e-4) {
        c = vec3<f32>(
            textureSample(scene_tex, scene_sampler, clamp(uv + radial * split, vec2<f32>(0.001), vec2<f32>(0.999))).r,
            textureSample(scene_tex, scene_sampler, uv).g,
            textureSample(scene_tex, scene_sampler, clamp(uv - radial * split, vec2<f32>(0.001), vec2<f32>(0.999))).b,
        );
    } else {
        c = textureSample(scene_tex, scene_sampler, uv).rgb;
    }
    // Charge: a cold bloom at the rim, the drive's light in the glass.
    if (charge > 1e-4) {
        let rim = smoothstep(0.3, 0.95, rq) * charge;
        c = mix(c, vec3<f32>(0.80, 0.92, 1.0), rim * 0.22);
    }
    // Speed: the view cools and closes in at the edges, and the liquid's
    // crests catch a thread of light.
    if (speed > 1e-4) {
        let vig = smoothstep(0.35, 1.05, rq) * speed;
        c = mix(c, c * vec3<f32>(0.55, 0.75, 1.0) * 0.35, vig * 0.7);
        c += vec3<f32>(0.35, 0.6, 1.0) * clamp(length(off) * 6.0 - 0.08, 0.0, 0.35) * speed;
    }
    c = mix(c, vec3<f32>(1.0) - c, invert);
    c = mix(c, vec3<f32>(0.02, 0.03, 0.06), beyond);
    return vec4<f32>(c, 1.0);
}
