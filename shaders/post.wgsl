// post.wgsl — the picture: bloom, exposure, tonemap, the drive's distortion
// and the glass rim, done to the HDR world before the ship is drawn over it
// (pass: post; PLAN.md art rule 1).
//
// Lane: A (fragment only). Cost class: cheap — the bloom chain is a
// handful of small draws that shrink with the render scale (it starts at
// half the scene's size), and the main draw is one fetch per pixel when
// the drive is idle and the rim is clean, a few when it is not.
//
// Entry points, in the order the pass runs them:
//   fs_prefilter  world → bloom level 0 (half res): a 13-tap downsample with
//                 a soft knee at the threshold, in radiance, and a partial
//                 Karis average so a lone sub-pixel star seeds a soft halo
//                 and not a firefly. Alpha carries the mean log luminance
//                 of the frame, unthresholded — the exposure meter.
//   fs_down       level n → n+1: the same 13 taps, rgb and alpha.
//   fs_up         level n+1 → n: a 9-tap tent, added (blend ONE + ONE), so
//                 level 0 ends up the sum of every width — wide and soft,
//                 never a blob.
//   fs_adapt      the smallest level's alpha, averaged, eased toward over
//                 a second or two: the adapted luminance the exposure
//                 drifts on.
//   fs_main       the world fetched through the drive's distortion — the
//                 FLIP's mirror sphere, the field's liquid, the speed
//                 streaks and their chromatic split — plus the glass rim's
//                 own hair of fringing; the bloom added; exposure (the
//                 setting times the drift); the tonemap; dithered to 8 bits.
//
// The ship — cabin, dials, holo3PP — is drawn AFTER this, into the same
// target, so none of it is bloomed, tonemapped or warped: the drive is a
// thing seen through the glass, and the gauges hold still on the dash.

@group(0) @binding(0) var src_tex: texture_2d<f32>;
@group(0) @binding(1) var src_samp: sampler;
@group(0) @binding(2) var<uniform> post: Post;
// fs_main: bloom level 0. fs_adapt: last frame's adapted luminance.
@group(0) @binding(3) var aux_tex: texture_2d<f32>;
// fs_main: this frame's adapted luminance (1×1).
@group(0) @binding(4) var adapt_tex: texture_2d<f32>;

struct Post {
    // x: fisheye 0..1, y: invert 0..1, z: flow 0..1 (the liquid field),
    // w: charge 0..1
    fx: vec4<f32>,
    // x: aspect, y: time s, z: speed 0..1 (streaks, the cool rim),
    // w: bloom strength (1 = stock)
    misc: vec4<f32>,
    // x: exposure (1 = stock), y: tonemap (0 off, 1 soft, 2 AgX),
    // z: fringe (1 = stock), w: adaptation blend this frame 0..1
    look: vec4<f32>,
    // x: bloom threshold in the world's radiance, y: knee width,
    // z: bypass (profiling: 1 = one fetch, nothing done), w: unused
    knee: vec4<f32>,
}

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

const LUM_W: vec3<f32> = vec3<f32>(0.2126, 0.7152, 0.0722);

// ------------------------------------------------------------- the bloom

fn tap(uv: vec2<f32>, texel: vec2<f32>, o: vec2<f32>) -> vec4<f32> {
    return textureSampleLevel(src_tex, src_samp, uv + o * texel, 0.0);
}

// The 13-tap downsample (Jimenez 2014): four inner taps at half weight,
// and four boxes of the centre, two edge taps and a corner at an eighth
// each. Every tap lands on a texel corner, so each is already a 2×2
// average: 13 fetches cover a 6×6 texel footprint without a gap.
fn down13(uv: vec2<f32>, texel: vec2<f32>) -> vec4<f32> {
    let c = tap(uv, texel, vec2<f32>(0.0, 0.0));
    let tl = tap(uv, texel, vec2<f32>(-2.0, -2.0));
    let tr = tap(uv, texel, vec2<f32>(2.0, -2.0));
    let bl = tap(uv, texel, vec2<f32>(-2.0, 2.0));
    let br = tap(uv, texel, vec2<f32>(2.0, 2.0));
    let t = tap(uv, texel, vec2<f32>(0.0, -2.0));
    let b = tap(uv, texel, vec2<f32>(0.0, 2.0));
    let l = tap(uv, texel, vec2<f32>(-2.0, 0.0));
    let r = tap(uv, texel, vec2<f32>(2.0, 0.0));
    let itl = tap(uv, texel, vec2<f32>(-1.0, -1.0));
    let itr = tap(uv, texel, vec2<f32>(1.0, -1.0));
    let ibl = tap(uv, texel, vec2<f32>(-1.0, 1.0));
    let ibr = tap(uv, texel, vec2<f32>(1.0, 1.0));
    return (itl + itr + ibl + ibr) * 0.125
        + c * 0.125
        + (t + b + l + r) * 0.0625
        + (tl + tr + bl + br) * 0.03125;
}

// The soft knee: nothing below the threshold, everything well above it,
// and a quadratic ease between — so a star just over the line does not
// pop in and out of the halo as it drifts across pixels.
fn knee(c: vec3<f32>) -> vec3<f32> {
    let t = post.knee.x;
    let k = max(post.knee.y, 1e-3);
    let br = max(c.r, max(c.g, c.b));
    var soft = clamp(br - t + k, 0.0, 2.0 * k);
    soft = soft * soft / (4.0 * k + 1e-5);
    let contribution = max(soft, br - t) / max(br, 1e-5);
    return c * contribution;
}

// Karis weight: a lone brilliant pixel counts for less than its neighbours,
// so it seeds a soft halo instead of a flickering firefly. Partial (a
// quarter of the luminance) so the brightest stars still reach.
fn karis(c: vec3<f32>) -> f32 {
    return 1.0 / (1.0 + dot(c, LUM_W) * 0.25);
}

@fragment
fn fs_prefilter(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src_tex));
    // The five groups of the 13 taps, each thresholded and Karis-weighted
    // as a group.
    let c = tap(in.uv, texel, vec2<f32>(0.0, 0.0)).rgb;
    let tl = tap(in.uv, texel, vec2<f32>(-2.0, -2.0)).rgb;
    let tr = tap(in.uv, texel, vec2<f32>(2.0, -2.0)).rgb;
    let bl = tap(in.uv, texel, vec2<f32>(-2.0, 2.0)).rgb;
    let br = tap(in.uv, texel, vec2<f32>(2.0, 2.0)).rgb;
    let t = tap(in.uv, texel, vec2<f32>(0.0, -2.0)).rgb;
    let b = tap(in.uv, texel, vec2<f32>(0.0, 2.0)).rgb;
    let l = tap(in.uv, texel, vec2<f32>(-2.0, 0.0)).rgb;
    let r = tap(in.uv, texel, vec2<f32>(2.0, 0.0)).rgb;
    let itl = tap(in.uv, texel, vec2<f32>(-1.0, -1.0)).rgb;
    let itr = tap(in.uv, texel, vec2<f32>(1.0, -1.0)).rgb;
    let ibl = tap(in.uv, texel, vec2<f32>(-1.0, 1.0)).rgb;
    let ibr = tap(in.uv, texel, vec2<f32>(1.0, 1.0)).rgb;

    let g0 = (itl + itr + ibl + ibr) * 0.25;
    let g1 = (c + t + l + tl) * 0.25;
    let g2 = (c + t + r + tr) * 0.25;
    let g3 = (c + b + l + bl) * 0.25;
    let g4 = (c + b + r + br) * 0.25;
    let w0 = karis(g0) * 0.5;
    let w1 = karis(g1) * 0.125;
    let w2 = karis(g2) * 0.125;
    let w3 = karis(g3) * 0.125;
    let w4 = karis(g4) * 0.125;
    let sum = (g0 * w0 + g1 * w1 + g2 * w2 + g3 * w3 + g4 * w4) / (w0 + w1 + w2 + w3 + w4);
    // The meter: the log of the luminance of the plain average — every
    // pixel, stars and black alike — so the geometric mean of the frame
    // comes out of the chain.
    let plain = g0 * 0.5 + (g1 + g2 + g3 + g4) * 0.125;
    let loglum = log(dot(plain, LUM_W) + 1e-4);
    return vec4<f32>(knee(sum), loglum);
}

@fragment
fn fs_down(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src_tex));
    return down13(in.uv, texel);
}

// The 9-tap tent, one texel of the SOURCE (coarser) level wide: bilinear
// makes each tap a small box, so the tent is a smooth bump, and summing
// the tents of every level gives the wide, soft fall-off.
@fragment
fn fs_up(in: VsOut) -> @location(0) vec4<f32> {
    let texel = 1.0 / vec2<f32>(textureDimensions(src_tex));
    var acc = vec3<f32>(0.0);
    acc += tap(in.uv, texel, vec2<f32>(-1.0, -1.0)).rgb;
    acc += tap(in.uv, texel, vec2<f32>(0.0, -1.0)).rgb * 2.0;
    acc += tap(in.uv, texel, vec2<f32>(1.0, -1.0)).rgb;
    acc += tap(in.uv, texel, vec2<f32>(-1.0, 0.0)).rgb * 2.0;
    acc += tap(in.uv, texel, vec2<f32>(0.0, 0.0)).rgb * 4.0;
    acc += tap(in.uv, texel, vec2<f32>(1.0, 0.0)).rgb * 2.0;
    acc += tap(in.uv, texel, vec2<f32>(-1.0, 1.0)).rgb;
    acc += tap(in.uv, texel, vec2<f32>(0.0, 1.0)).rgb * 2.0;
    acc += tap(in.uv, texel, vec2<f32>(1.0, 1.0)).rgb;
    // Alpha untouched: the blend adds rgb only (alpha ZERO + ONE).
    return vec4<f32>(acc / 16.0, 0.0);
}

// ---------------------------------------------------------- the exposure

// The smallest level's alpha is the frame's mean log luminance, near
// enough; a 4×4 of bilinear taps over it finishes the average. Eased
// toward from the last frame's value by the blend, so the exposure
// drifts rather than flicks — and lands at once on the first frame.
@fragment
fn fs_adapt(in: VsOut) -> @location(0) vec4<f32> {
    var sum = 0.0;
    for (var j = 0; j < 4; j += 1) {
        for (var i = 0; i < 4; i += 1) {
            let uv = (vec2<f32>(f32(i), f32(j)) + 0.5) / 4.0;
            sum += textureSampleLevel(src_tex, src_samp, uv, 0.0).a;
        }
    }
    let now = sum / 16.0;
    let last = textureLoad(aux_tex, vec2<i32>(0, 0), 0).r;
    let a = clamp(post.look.w, 0.0, 1.0);
    return vec4<f32>(mix(last, now, a), 0.0, 0.0, 1.0);
}

// The drift: a starry sky is dark and stays dark (lifted by a third of a
// stop at most); a sunlit planet filling the glass is held back half a
// stop. Fixed exposure between — the setting is the picture, the drift
// is the eye settling.
fn drift(adapted_loglum: f32) -> f32 {
    let lum = exp(adapted_loglum);
    return clamp(pow(0.12 / max(lum, 1e-4), 0.35), 0.70, 1.30);
}

// ------------------------------------------------------------ the tonemap

// AgX (Sobotka; the minimal fit by Wrensch): an inset toward the primaries
// so hues survive the shoulder, a log2 encoding over 16.5 stops, a sixth
// order sigmoid, then the outset and the display's 2.2 power back to
// linear, since the target is an sRGB framebuffer. Highlights roll into
// white through their own hue instead of clipping to a flat one.
fn agx_contrast(x: vec3<f32>) -> vec3<f32> {
    let x2 = x * x;
    let x4 = x2 * x2;
    return 15.5 * x4 * x2 - 40.14 * x4 * x + 31.96 * x4 - 6.868 * x2 * x
        + 0.4298 * x2 + 0.1191 * x - 0.00232;
}

fn agx(val: vec3<f32>) -> vec3<f32> {
    let inset = mat3x3<f32>(
        vec3<f32>(0.842479062253094, 0.0423282422610123, 0.0423756549057051),
        vec3<f32>(0.0784335999999992, 0.878468636469772, 0.0784336),
        vec3<f32>(0.0792237451477643, 0.0791661274605434, 0.879142973793104),
    );
    let min_ev = -12.47393;
    let max_ev = 4.026069;
    var v = inset * max(val, vec3<f32>(0.0));
    v = clamp(log2(max(v, vec3<f32>(1e-10))), vec3<f32>(min_ev), vec3<f32>(max_ev));
    v = (v - min_ev) / (max_ev - min_ev);
    v = agx_contrast(v);
    // The look: a touch of punch — a little more contrast about middle
    // grey and a little more colour — the way a film stock is not flat.
    let luma = dot(v, LUM_W);
    v = pow(max(v, vec3<f32>(0.0)), vec3<f32>(1.12));
    v = luma + (v - luma) * 1.18;
    let outset = mat3x3<f32>(
        vec3<f32>(1.19687900512017, -0.0528968517574562, -0.0529716355144438),
        vec3<f32>(-0.0980208811401368, 1.15190312990417, -0.0980434501171241),
        vec3<f32>(-0.0990297440797205, -0.0989611768448433, 1.15107367264116),
    );
    v = outset * v;
    return pow(max(v, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn tonemap_mode(c: vec3<f32>, mode: f32) -> vec3<f32> {
    if (mode < 0.5) {
        return clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    }
    if (mode < 1.5) {
        return vec3<f32>(1.0) - exp(-max(c, vec3<f32>(0.0)));
    }
    return agx(c);
}

// ------------------------------------------------------ the drive's look

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

fn world(uv: vec2<f32>) -> vec3<f32> {
    return textureSampleLevel(src_tex, src_samp, clamp(uv, vec2<f32>(0.001), vec2<f32>(0.999)), 0.0).rgb;
}

// The world through a chromatic split: red fetched a little out along
// `radial`, blue a little in.
fn world_split(uv: vec2<f32>, radial: vec2<f32>, split: f32) -> vec3<f32> {
    if (split < 1e-6) {
        return world(uv);
    }
    return vec3<f32>(
        world(uv + radial * split).r,
        world(uv).g,
        world(uv - radial * split).b,
    );
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (post.knee.z > 0.5) {
        return vec4<f32>(world(in.uv), 1.0);
    }
    let fisheye = post.fx.x;
    let invert = post.fx.y;
    let flow = post.fx.z;
    let charge = post.fx.w;
    let aspect = post.misc.x;
    let time = post.misc.y;
    let speed = post.misc.z;
    let bloom_k = post.misc.w;
    let fringe = post.look.z;

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

    // Radial: the line out from the centre of the view, in uv units.
    let q = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let rq = length(q);
    let radial = select(vec2<f32>(0.0), q / rq, rq > 1e-4) / vec2<f32>(aspect, 1.0);
    // The chromatic split: the drive's, and the glass's own — a hair of
    // it, only toward the rim, where the canopy is thick and oblique.
    let glass = smoothstep(0.55, 1.45, rq);
    let split = (0.004 * charge + 0.010 * speed) * rq + 0.0035 * fringe * glass * glass;

    var c: vec3<f32>;
    if (speed > 1e-4) {
        // Speed: radial streaks — a few taps out along the line from the
        // centre, longer at the rim.
        let reach = 0.05 * speed * rq;
        var acc = vec3<f32>(0.0);
        for (var k = 0; k < 4; k += 1) {
            let s = (f32(k) / 3.0 - 0.5) * reach;
            acc += world_split(uv + radial * s, radial, split);
        }
        c = acc / 4.0;
    } else {
        c = world_split(uv, radial, split);
    }

    // The bloom, fetched through the same distortion so it warps with the
    // world: added, never mixed, so the picture underneath keeps its
    // contrast — the halo is light on top of light.
    if (bloom_k > 1e-4) {
        c += textureSampleLevel(aux_tex, src_samp, uv, 0.0).rgb * (0.065 * bloom_k);
    }

    // Exposure: the setting, and the eye's drift about it.
    let adapted = textureLoad(adapt_tex, vec2<i32>(0, 0), 0).r;
    c *= post.look.x * drift(adapted);

    // The drive's light in the glass: a cold bloom at the rim, and the
    // liquid's crests catching a thread of it — radiance, before the curve,
    // so it rolls to white like everything else.
    if (charge > 1e-4) {
        let rim = smoothstep(0.3, 0.95, rq) * charge;
        c += vec3<f32>(0.80, 0.92, 1.0) * rim * 0.14;
    }
    if (speed > 1e-4) {
        c += vec3<f32>(0.35, 0.6, 1.0) * clamp(length(off) * 6.0 - 0.08, 0.0, 0.35) * speed * 1.5;
    }

    c = tonemap_mode(c, post.look.y);

    // Speed: the view cools and closes in at the edges.
    if (speed > 1e-4) {
        let vig = smoothstep(0.35, 1.05, rq) * speed;
        c = mix(c, c * vec3<f32>(0.55, 0.75, 1.0) * 0.35, vig * 0.7);
    }
    c = mix(c, vec3<f32>(1.0) - c, invert);
    c = mix(c, vec3<f32>(0.02, 0.03, 0.06), beyond);
    // Eight bits from here on: dither once, here, where the picture is
    // finished — the ship draws over this and the blit only scales it.
    c += vec3<f32>(dither_px(in.pos.xy));
    return vec4<f32>(max(c, vec3<f32>(0.0)), 1.0);
}
