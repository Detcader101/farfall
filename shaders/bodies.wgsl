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
    // can be found on a big sky. y: screen height, px. z: lens flare
    // strength (0 none). w: unused.
    look: vec4<f32>,
    // xyz: Uranus' centre relative to the camera, metres. w: radius, m.
    uranus: vec4<f32>,
    // xyz: the planet's centre relative to the camera, w: radius — what
    // stands in front of the Sun, for the flare.
    planet: vec4<f32>,
}

// Does the line of sight to the Sun pass through this body?
fn hides_sun(sun_dir: vec3<f32>, body: vec4<f32>) -> bool {
    if (body.w <= 0.0) {
        return false;
    }
    let along = dot(sun_dir, body.xyz);
    if (along <= 0.0) {
        return false;
    }
    let perp2 = dot(body.xyz, body.xyz) - along * along;
    return perp2 < body.w * body.w;
}

// The lens flare: the canopy's and the eye's own artefacts of the Sun on
// the screen — a starburst on the Sun, an anamorphic streak across it,
// and a train of rainbow ghosts down the line through the screen's
// centre. Screen-space, in NDC corrected for the aspect.
fn lens_flare(ndc: vec2<f32>, sun_ndc: vec2<f32>, aspect: f32, strength: f32, t: f32) -> vec3<f32> {
    let p = (ndc - sun_ndc) * vec2<f32>(aspect, 1.0);
    let r = length(p);
    var out = vec3<f32>(0.0);
    // Starburst: six soft rays, turning very slowly, over a warm core.
    let ang = atan2(p.y, p.x);
    let rays = pow(abs(cos(ang * 3.0 + t * 0.05)), 24.0) * exp(-r / 0.45) * 0.9
        + pow(abs(cos(ang * 3.0 + 0.5236)), 60.0) * exp(-r / 0.7) * 0.35;
    out += vec3<f32>(1.0, 0.92, 0.80) * (rays + exp(-r / 0.06) * 0.6);
    // Anamorphic streak: a thin blue line across the width.
    let streak = exp(-abs(p.y) / 0.006) * exp(-abs(p.x) / 0.9) * 0.55;
    out += vec3<f32>(0.45, 0.65, 1.0) * streak;
    // Ghosts: along the line from the Sun through the centre, at set
    // fractions, each a soft ring of its own tint and size.
    let to_centre = -sun_ndc * vec2<f32>(aspect, 1.0);
    let ks = array<f32, 6>(0.35, 0.6, 0.95, 1.25, 1.6, 2.1);
    let rs = array<f32, 6>(0.05, 0.11, 0.035, 0.16, 0.07, 0.22);
    let tints = array<vec3<f32>, 6>(
        vec3<f32>(1.0, 0.55, 0.35), vec3<f32>(0.5, 0.9, 0.6), vec3<f32>(0.6, 0.7, 1.0),
        vec3<f32>(1.0, 0.8, 0.4), vec3<f32>(0.9, 0.5, 0.9), vec3<f32>(0.45, 0.75, 1.0),
    );
    for (var i = 0; i < 6; i += 1) {
        let g = to_centre * ks[i];
        let d = length(p - g) / rs[i];
        // A ring with a dim fill, fading at the rim.
        let ring = (1.0 - smoothstep(0.85, 1.0, d)) * (0.25 + 0.75 * smoothstep(0.55, 0.95, d));
        out += tints[i] * ring * 0.16;
    }
    return out * strength;
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
    // The flare, first: behind everything, it is light in the glass. Only
    // with the Sun in front of the camera and nothing in front of the Sun,
    // fading as it leaves the screen.
    let flare_k = bd.look.z;
    let sun_fwd = dot(sun, bd.forward.xyz);
    if (flare_k > 0.0 && sun_fwd > 0.05
        && !hides_sun(sun, bd.planet) && !hides_sun(sun, bd.moon) && !hides_sun(sun, bd.uranus)) {
        let sx = dot(sun, bd.right.xyz) / sun_fwd / (bd.params.x * bd.params.y);
        let sy = dot(sun, bd.up.xyz) / sun_fwd / bd.params.x;
        let sun_ndc = vec2<f32>(sx, sy);
        let on_screen = (1.0 - smoothstep(0.9, 1.4, abs(sx))) * (1.0 - smoothstep(0.9, 1.4, abs(sy)));
        if (on_screen > 0.001) {
            // A wall of fire needs no starburst: the flare thins as the
            // disc grows.
            let big = smoothstep(0.004, 0.06, sun_limb);
            rgb += lens_flare(in.ndc, sun_ndc, bd.params.y, flare_k * on_screen * (1.0 - 0.85 * big), bd.params.z);
        }
    }
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
    // The disc itself. From the planet it is a half-degree dot and
    // saturates, as it should; resolved (after a jump, or close), it is a
    // surface: limb-darkened, granulated, with sunspots in two bands that
    // turn with the Sun over its month, and prominences on the limb —
    // loops of plasma standing off it — with, now and then, a coronal
    // mass ejection swelling out and away.
    var face = 1.0;
    let t = bd.params.z;
    let resolved = smoothstep(0.004, 0.02, sun_limb);
    if (disc > 0.001 && resolved > 0.001) {
        let along = dot(ray, bd.sun.xyz);
        let disc_s = along * along - (sun_d * sun_d - bd.sun.w * bd.sun.w);
        let ts = along - sqrt(max(disc_s, 0.0));
        let n = normalize(ray * ts - bd.sun.xyz);
        // The Sun turns once in 25 days (1:100 time is not scaled: a
        // slow drift you can watch over a long sit).
        let spin = t * 0.0025;
        let cs = cos(spin);
        let ss = sin(spin);
        let nl = vec3<f32>(cs * n.x - ss * n.z, n.y, ss * n.x + cs * n.z);
        // Limb darkening: the edge is cooler, deeper gas.
        let mu = max(dot(n, -ray), 0.0);
        let limb_dark = 0.45 + 0.55 * sqrt(mu);
        // Granulation: a fine cellular grain.
        let gran = 0.92 + 0.16 * (vnoise(nl * 90.0) - 0.5) + 0.08 * (fbm3(nl * 30.0) - 0.5);
        // Sunspots: in the bands about ±15°–30° latitude, where the noise
        // peaks: a dark umbra in a grey penumbra, a few, drifting.
        let lat = abs(nl.y);
        let band = smoothstep(0.15, 0.3, lat) * (1.0 - smoothstep(0.45, 0.6, lat));
        let sp = fbm3(nl * 6.0 + vec3<f32>(3.0, 0.0, t * 0.0004));
        let penumbra = smoothstep(0.60, 0.68, sp) * band;
        let umbra = smoothstep(0.66, 0.72, sp) * band;
        let spot = 1.0 - 0.55 * penumbra - 0.38 * umbra;
        face = limb_dark * gran * spot;
    }
    // What the disc is worth: saturating from afar, a surface when resolved.
    let disc_k = mix(60.0, 1.35, resolved);
    // Resolved, the glare stays outside the disc: the surface is the thing.
    let glare_k = 3.0 * (1.0 - disc * resolved);
    // Resolved, the surface is warmer than the saturated dot: a yellow-white.
    let face_rgb = mix(sun_rgb, vec3<f32>(1.0, 0.88, 0.66), resolved);
    var sun_out = face_rgb * disc * disc_k * face + sun_rgb * glare * glare_k;
    // Prominences and a CME, just past the limb, when resolved.
    if (resolved > 0.001 && past > 0.0 && past < sun_limb * 0.9) {
        // Where on the limb this is: the angle round the disc.
        let to_c = bd.sun.xyz / max(sun_d, 1.0);
        let side = normalize(cross(to_c, bd.up.xyz));
        let upv = cross(side, to_c);
        let off = ray - to_c * dot(ray, to_c);
        let theta = atan2(dot(off, upv), dot(off, side));
        let h = past / sun_limb; // 0 at the limb, 1 a radius out
        // Loops: a few arcs standing off the limb, slowly changing.
        let loops = fbm3(vec3<f32>(theta * 2.0, h * 6.0, t * 0.02)) ;
        let prom = smoothstep(0.55, 0.7, loops) * (1.0 - smoothstep(0.0, 0.22, h));
        // The CME: once in a while a bubble grows from one side, out to
        // half a radius and beyond, thinning as it goes. A slow cycle.
        let cycle = fract(t / 240.0);
        let grow = smoothstep(0.0, 0.6, cycle) * (1.0 - smoothstep(0.6, 1.0, cycle));
        let where_ = cos(theta - floor(t / 240.0) * 2.4);
        let front = 0.05 + 0.7 * smoothstep(0.0, 0.6, cycle);
        let bubble = smoothstep(0.2, 0.9, where_) * (1.0 - smoothstep(front - 0.25, front, h))
            * smoothstep(front - 0.6, front - 0.2, h) * grow;
        let wisps = 0.6 + 0.4 * fbm3(vec3<f32>(theta * 4.0, h * 9.0, t * 0.05));
        let plasma = vec3<f32>(1.0, 0.45, 0.18);
        sun_out += plasma * (prom * 2.2 + bubble * wisps * 1.6) * resolved;
        alpha = max(alpha, min(prom * 0.8 + bubble * wisps * 0.6, 1.0));
    }
    rgb += sun_out;
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
                    // From inside the ring (the belt, up close) the sheet
                    // is not a wall: a faint haze of dust, the rocks are
                    // what there is to see. The camera's height above the
                    // plane, against the ring's thickness, says how far in.
                    let h = abs(dot(c, axis));
                    let inside = 1.0 - smoothstep(0.0, 0.006 * rr, h);
                    let veil = in_ring * mix(0.55, 0.10, inside);
                    rgb = mix(rgb, vec3<f32>(0.5, 0.55, 0.6) * lit * 1.2, veil);
                    alpha = max(alpha, veil);
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
