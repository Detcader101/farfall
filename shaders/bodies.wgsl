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
    // xyz: sun centre relative to the camera, metres. w: its radius, m.
    // From near the planet that is a direction and a half-degree disc;
    // after a jump it is a wall of fire.
    sun: vec4<f32>,
    // x: tags 0..1 — a thin ring around each body so a half-degree disc
    // can be found on a big sky. y: screen height, px. zw: unused.
    look: vec4<f32>,
    // xyz: Uranus' centre relative to the camera, metres. w: radius, m.
    uranus: vec4<f32>,
}

// A body's disc coverage and surface normal along `ray`, or cover 0.
struct Disc {
    cover: f32,
    n: vec3<f32>,
    angle: f32,
    limb: f32,
}

fn disc_of(ray: vec3<f32>, centre: vec3<f32>, radius: f32) -> Disc {
    let d = length(centre);
    var out = Disc(0.0, vec3<f32>(0.0, 0.0, 1.0), 0.0, 0.0);
    if (radius <= 0.0 || d <= radius) {
        return out;
    }
    let to_c = centre / d;
    out.angle = acos(clamp(dot(ray, to_c), -1.0, 1.0));
    out.limb = asin(clamp(radius / d, 0.0, 1.0));
    let g = vec2<f32>(dpdx(out.angle), dpdy(out.angle));
    let px = max(0.5 * length(g), 1e-7);
    out.cover = 1.0 - smoothstep(out.limb - px, out.limb + px, out.angle);
    if (out.cover > 0.001) {
        let along = dot(ray, centre);
        let disc_b = along * along - (d * d - radius * radius);
        let t = along - sqrt(max(disc_b, 0.0));
        out.n = normalize(ray * t - centre);
    }
    return out;
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
    let sun_d = length(bd.sun.xyz);
    let sun = bd.sun.xyz / max(sun_d, 1.0);
    let sun_limb = asin(clamp(bd.sun.w / max(sun_d, bd.sun.w + 1.0), 0.0, 1.0));

    var rgb = vec3<f32>(0.0);
    var alpha = 0.0;

    // ---- the Sun ---------------------------------------------------------
    let cos_sun = dot(ray, sun);
    let ang = acos(clamp(cos_sun, -1.0, 1.0));
    let grad = vec2<f32>(dpdx(ang), dpdy(ang));
    let px = max(0.5 * length(grad), 1e-7);
    let disc = 1.0 - smoothstep(sun_limb - px, sun_limb + px, ang);
    // The disc is the brightest thing in the sky: it saturates, as it
    // should. Around it, glare — the canopy's own scattering of it — fading
    // over a few degrees past the limb, however wide the limb is.
    let past = max(ang - sun_limb, 0.0);
    let glare = exp(-past / 0.02) * 0.9 + exp(-past / 0.10) * 0.12;
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
            // Earthshine on the night side: the planet is a big bright
            // thing from there, and a new moon is a grey disc, not a hole.
            let moon_rgb = vec3<f32>(albedo) * (light * 2.2 + 0.09);
            rgb = mix(rgb, moon_rgb, cover);
            alpha = max(alpha, cover);
        }
    }

    // ---- Uranus ----------------------------------------------------------
    // A pale cyan ice giant, faintly banded about an axis tipped nearly
    // onto its side, with its thin dark rings seen edge-on-ish.
    let ur = disc_of(ray, bd.uranus.xyz, bd.uranus.w);
    if (ur.cover > 0.001) {
        let n = ur.n;
        // The spin axis: tipped 98 degrees — lying in the orbital plane.
        let axis = normalize(vec3<f32>(0.97, 0.14, 0.2));
        let lat = dot(n, axis);
        let bands = 0.5 + 0.5 * sin(lat * 18.0 + fbm3(n * 2.5) * 2.0);
        let base = vec3<f32>(0.56, 0.78, 0.86);
        let band_rgb = mix(base * 0.92, base * 1.05, bands);
        let light = max(dot(n, sun), 0.0);
        let ur_rgb = band_rgb * (light * 1.9 + 0.02);
        rgb = mix(rgb, ur_rgb, ur.cover);
        alpha = max(alpha, ur.cover);
    }
    // The rings: a flat annulus in the plane normal to the spin axis, from
    // 1.6 to 2.0 radii, dark and narrow — a thread, as Uranus' are.
    {
        let c = bd.uranus.xyz;
        let rr = bd.uranus.w;
        let axis = normalize(vec3<f32>(0.97, 0.14, 0.2));
        let denom = dot(ray, axis);
        if (rr > 0.0 && abs(denom) > 1e-5) {
            let t = dot(c, axis) / denom;
            if (t > 0.0) {
                let hit = ray * t - c;
                let rad = length(hit) / rr;
                let in_ring = smoothstep(1.62, 1.66, rad) * (1.0 - smoothstep(1.96, 2.0, rad));
                // Hidden behind the planet itself.
                let behind = ur.cover > 0.5 && t > length(c);
                if (in_ring > 0.001 && !behind) {
                    let lit = max(abs(dot(axis, sun)), 0.15);
                    rgb = mix(rgb, vec3<f32>(0.5, 0.55, 0.6) * lit * 1.2, in_ring * 0.55);
                    alpha = max(alpha, in_ring * 0.55);
                }
            }
        }
    }

    // ---- tags ------------------------------------------------------------
    // A ring a few pixels outside each body's limb, never thinner than the
    // pixel, so the Moon is findable at its honest size. Additive, cyan,
    // like the rest of the glass.
    let tags = bd.look.x;
    if (tags > 0.01) {
        let px_rad = 2.0 * bd.params.x / max(bd.look.y, 1.0);
        let cyan = vec3<f32>(0.22, 0.85, 1.0) * 0.9 * tags;
        // Moon.
        if (radius > 0.0 && d > radius) {
            let to_c = centre / d;
            let angle = acos(clamp(dot(ray, to_c), -1.0, 1.0));
            let limb = asin(clamp(radius / d, 0.0, 1.0));
            let ring_r = max(limb * 1.8, 10.0 * px_rad);
            let ring = 1.0 - smoothstep(0.0, px_rad * 1.5, abs(angle - ring_r) - px_rad * 0.6);
            rgb += cyan * ring;
        }
        // Uranus.
        if (bd.uranus.w > 0.0 && length(bd.uranus.xyz) > bd.uranus.w) {
            let ud = length(bd.uranus.xyz);
            let angle = acos(clamp(dot(ray, bd.uranus.xyz / ud), -1.0, 1.0));
            let limb = asin(clamp(bd.uranus.w / ud, 0.0, 1.0));
            let ring_r = max(limb * 1.8, 10.0 * px_rad);
            let ring = 1.0 - smoothstep(0.0, px_rad * 1.5, abs(angle - ring_r) - px_rad * 0.6);
            rgb += cyan * ring * 0.8;
        }
        // Sun: only while it is still a dot — at a wall of fire a tag is noise.
        if (sun_limb < 0.2) {
            let ring_r = max(sun_limb * 1.8, 10.0 * px_rad);
            let ring = 1.0 - smoothstep(0.0, px_rad * 1.5, abs(ang - ring_r) - px_rad * 0.6);
            rgb += cyan * ring * 0.7;
        }
    }

    if (alpha < 0.002 && dot(rgb, rgb) < 1e-6) {
        discard;
    }
    let out = tonemap(rgb, exposure);
    // Premultiplied: the glare adds over the stars, the discs replace them.
    return vec4<f32>(out + dither_px(in.pos.xy) * alpha, alpha);
}
