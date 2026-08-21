// bodies.wgsl — the Sun and the Moon (pass: bodies)
//
// Lane: A (vertex+fragment only). Cost class: cheap — one ray-sphere test
// and two angle tests per pixel; the Moon's surface noise runs only inside
// its half-degree disc.
//
// Both at the world's own 1:100 scale, which keeps the sky honest: the Sun
// is 6,960 km across at 1.5 million km and the Moon 17.4 km across at
// 3,844 km, and each subtends the half degree it really does. The Sun is a
// direction (its parallax from anywhere near this planet is nothing) with
// a disc and a glare; the Moon is a sphere in the world — it has a
// position, a phase lit by the same sun that lights the planet, and a limb
// antialiased analytically like the planet's. Drawn between the stars and
// the planet, so the planet occludes both.

struct Bodies {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: moon centre relative to the camera, metres. w: radius, m.
    moon: vec4<f32>,
    // xyz: unit vector toward the sun. w: its angular radius, rad.
    sun: vec4<f32>,
}

@group(0) @binding(0) var<uniform> bd: Bodies;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let exposure = bd.params.w;
    let ray = view_ray(
        in.ndc, bd.right.xyz, bd.up.xyz, bd.forward.xyz, bd.params.x, bd.params.y,
    );
    let sun = normalize(bd.sun.xyz);

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;

    // ---- the Sun ---------------------------------------------------------
    let cos_sun = dot(ray, sun);
    let ang = acos(clamp(cos_sun, -1.0, 1.0));
    let grad = vec2<f32>(dpdx(ang), dpdy(ang));
    let px = max(0.5 * length(grad), 1e-7);
    let disc = 1.0 - smoothstep(bd.sun.w - px, bd.sun.w + px, ang);
    // The disc is the brightest thing in the sky: it saturates, as it
    // should. Around it, glare — the canopy's own scattering of it — fading
    // over a few degrees.
    let glare = exp(-ang / 0.02) * 0.9 + exp(-ang / 0.10) * 0.12;
    let sun_rgb = vec3<f32>(1.0, 0.96, 0.90);
    rgb += sun_rgb * (disc * 60.0 + glare * 3.0);
    alpha = max(alpha, disc);

    // ---- the Moon --------------------------------------------------------
    let centre = bd.moon.xyz;
    let radius = bd.moon.w;
    let d = length(centre);
    if (radius > 0.0 && d > radius) {
        let to_c = centre / d;
        let angle = acos(clamp(dot(ray, to_c), -1.0, 1.0));
        let limb = asin(clamp(radius / d, 0.0, 1.0));
        let mgrad = vec2<f32>(dpdx(angle), dpdy(angle));
        let mpx = max(0.5 * length(mgrad), 1e-7);
        let cover = 1.0 - smoothstep(limb - mpx, limb + mpx, angle);
        if (cover > 0.001) {
            let along = dot(ray, centre);
            let disc_b = along * along - (d * d - radius * radius);
            let t = along - sqrt(max(disc_b, 0.0));
            let n = normalize(ray * t - centre);
            // Maria and highlands: low-frequency albedo, fine crater grain.
            let maria = smoothstep(0.48, 0.62, fbm3(n * 3.0 + 7.0));
            let grain = fbm3(n * 40.0) * 0.25 + 0.75;
            let albedo = mix(0.16, 0.07, maria) * grain;
            let light = max(dot(n, sun), 0.0);
            // A sliver of earthshine on the night side.
            let moon_rgb = vec3<f32>(albedo) * (light * 1.4 + 0.012);
            rgb = mix(rgb, moon_rgb, cover);
            alpha = max(alpha, cover);
        }
    }

    if (alpha < 0.002 && dot(rgb, rgb) < 1e-6) {
        discard;
    }
    let out = tonemap(rgb, exposure);
    // Premultiplied: the glare adds over the stars, the discs replace them.
    return vec4<f32>(out + dither_px(in.pos.xy) * alpha, alpha);
}
