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

// ---- the belt, from afar --------------------------------------------------
// The same rocks crates/app/src/belt.rs brings live near the ship, laid
// out by the same hash over the same cells of the ring's co-rotating
// coordinates, so the far sheet resolves into exactly the asteroids the
// belt pass draws up close. Mirrors belt.rs::hash / unit / cell_rocks.
const BELT_CELL_M: f32 = 1400.0;
const BELT_HALF_M: f32 = 900.0;
const RING_INNER: f32 = 1.62;
const RING_OUTER: f32 = 1.98;
// The dust haze about the ring plane: half-thickness (the rocks reach ±900 m,
// the dust a little past them) and the run through it that costs one
// optical depth — mirrored by bodies.rs::ring_run_m and its tests.
const RING_HAZE_M: f32 = 1500.0;
const RING_HAZE_FREE_M: f32 = 250000.0;

fn belt_hash(x: i32, y: i32, z: i32, k: u32) -> u32 {
    var h = (u32(x) * 0x8da6b343u) ^ (u32(y) * 0xd8163841u) ^ (u32(z) * 0xcb1ab31fu) ^ (k * 0x9e3779b9u);
    h ^= h >> 15u;
    h = h * 0x2c1b3c6du;
    h ^= h >> 12u;
    h = h * 0x297a2d39u;
    h ^= h >> 15u;
    return h;
}

fn belt_unit(h: u32) -> f32 {
    return f32(h >> 8u) / 16777216.0;
}

// The rocks about a point in the ring (along the ring at the middle
// radius, radial offset, height — metres): the nearest disc's cover and
// shading normal, as a speck seen from afar. `px_m`: a pixel's footprint
// in metres at the hit, so a rock below a pixel still leaves its share.
struct Speck {
    cover: f32,
    n: vec3<f32>,
}

fn belt_specks(along: f32, r_off: f32, px_m: f32, e_along: vec3<f32>, e_rad: vec3<f32>, axis: vec3<f32>) -> Speck {
    var out = Speck(0.0, vec3<f32>(0.0, 0.0, 1.0));
    let ca = floor(along / BELT_CELL_M);
    let cr = floor(r_off / BELT_CELL_M);
    // Which neighbours a rock (up to 300 m) could reach here from.
    let fa = along / BELT_CELL_M - ca;
    let fr = r_off / BELT_CELL_M - cr;
    let da = select(1, -1, fa < 0.5);
    let dr = select(1, -1, fr < 0.5);
    // A rock is 300 m at most, so a neighbour cell can only reach this
    // point from within that of its edge: most points need no neighbour
    // along, across, or both — the same picture for a third of the hashes.
    let reach = 300.0 / BELT_CELL_M;
    let na = select(1, 2, min(fa, 1.0 - fa) < reach);
    let nr = select(1, 2, min(fr, 1.0 - fr) < reach);
    for (var ia = 0; ia < na; ia += 1) {
        for (var ir = 0; ir < nr; ir += 1) {
            for (var iz = -1; iz <= 0; iz += 1) {
                let x = i32(ca) + select(0, da, ia == 1);
                let y = i32(cr) + select(0, dr, ir == 1);
                let z = iz;
                let n = belt_hash(x, y, z, 7u) % 4u;
                for (var k = 0u; k < 3u; k += 1u) {
                    if (k >= n) { break; }
                    let salt = 16u * k;
                    let h1 = belt_unit(belt_hash(x, y, z, 1u + salt));
                    let h2 = belt_unit(belt_hash(x, y, z, 2u + salt));
                    let h3 = belt_unit(belt_hash(x, y, z, 3u + salt));
                    let h7 = belt_unit(belt_hash(x, y, z, 7u + salt));
                    let ra = (f32(x) + h1) * BELT_CELL_M;
                    let rr = (f32(y) + h2) * BELT_CELL_M;
                    let rh = (f32(z) + h3) * BELT_CELL_M;
                    if (abs(rh) > BELT_HALF_M) { continue; }
                    let radius = 5.0 * pow(60.0, pow(h7, 1.4));
                    let d = vec2<f32>(along - ra, r_off - rr);
                    let dist = length(d);
                    // A disc of the rock's size, never under a pixel: what
                    // is smaller than a pixel keeps a pixel's share.
                    let shown = max(radius, px_m * 0.7);
                    let cover = (1.0 - smoothstep(shown - px_m, shown + px_m, dist)) * min(1.0, radius / max(px_m, 1e-3));
                    if (cover > out.cover) {
                        out.cover = cover;
                        let u = d / max(shown, 1e-3);
                        let zz = sqrt(max(1.0 - dot(u, u), 0.0));
                        out.n = normalize(e_along * u.x + e_rad * u.y + axis * zz);
                    }
                }
            }
        }
    }
    return out;
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
    // Restrained: the flare is a hint of the glass, not the picture.
    return out * strength * 0.45;
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
        // The limb angle is measured against the WORLD, not the canopy:
        // prominences and the CME are the Sun's own weather and must hold
        // still while the ship rolls. (The lens flare alone lives in
        // screen space — that one is the canopy's artifact and may turn.)
        let side = normalize(cross(to_c, vec3<f32>(0.0, 1.0, 0.0)));
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
    // 1.6 to 2.0 radii, dark and narrow — a thread, as Uranus' are. Not a
    // sheet of no thickness, though: a SLAB the belt's own height (rocks
    // to ±900 m, a haze of dust a little past them), and every ray is
    // charged for the length it runs inside the slab and the annulus. One
    // rule from anywhere: from afar a face-on crossing is one thickness;
    // from inside the belt a ray to the zenith is out of the haze in a
    // kilometre while a ray along the plane runs tens of kilometres — so
    // the belt is a band of sun-lit dust on its own horizon, brightest
    // where the sight line grazes and toward the Sun. The old model hit
    // the mid-plane and applied the far sheet at the hit: from inside
    // the belt that hit was the camera itself for every ray on one side
    // of the plane and nothing on the other, which drew the plane's
    // great circle across the sky as a hard edge with a grey wash on one
    // side (the "nebula seam" of the 2026-08-31 baseline).
    {
        let c = bd.uranus.xyz;
        let rr = bd.uranus.w;
        let axis = normalize(vec3<f32>(0.97, 0.14, 0.2));
        let denom = dot(ray, axis);
        // The ring centre's height along the axis, relative to the camera:
        // the camera sits at -ch above the ring plane.
        let ch = dot(c, axis);
        let h_cam = abs(ch);
        if (rr > 0.0) {
            // The slab face this ray is heading for: rays up leave through
            // the top, rays down through the bottom. From outside, a ray
            // heading away never reaches it (t < 0); from inside both
            // directions do, and a grazing ray runs until the annulus ends.
            let dn_mag = max(abs(denom), 1e-5);
            let dn = select(dn_mag, -dn_mag, denom < 0.0);
            let eta = select(-RING_HAZE_M, RING_HAZE_M, denom > 0.0);
            let t_face = (eta + ch) / dn;
            // The other face: where a ray from outside enters the slab.
            let t_in = (-eta + ch) / dn;
            // In-plane geometry for the annulus: the camera's offset from
            // the axis and the ray's in-plane part.
            let o_f = -(c - axis * ch);
            let d_f = ray - axis * denom;
            let a = max(dot(d_f, d_f), 1e-8);
            let b = 2.0 * dot(o_f, d_f);
            let oo = dot(o_f, o_f);
            let r_out = rr * RING_OUTER;
            let r_in = rr * RING_INNER;
            let disc_o = b * b - 4.0 * a * (oo - r_out * r_out);
            if (t_face > 0.0 && disc_o > 0.0) {
                let so = sqrt(disc_o);
                var lo = max(max((-b - so) / (2.0 * a), 0.0), min(t_in, t_face));
                var hi = min((-b + so) / (2.0 * a), t_face);
                let disc_i = b * b - 4.0 * a * (oo - r_in * r_in);
                if (disc_i > 0.0) {
                    let si = sqrt(disc_i);
                    let u1 = (-b - si) / (2.0 * a);
                    let u2 = (-b + si) / (2.0 * a);
                    if (u1 > lo) {
                        // The run ends at the hole's near wall; the far
                        // side of the ring beyond it is left out.
                        hi = min(hi, u1);
                    } else if (u2 > lo) {
                        // Starting in the hole: the run begins past it.
                        lo = max(lo, u2);
                    }
                }
                let run = max(hi - lo, 0.0);
                if (run > 0.5) {
                    let lit = max(abs(dot(axis, sun)), 0.15);
                    // Where the ray leaves the slab, in the ring's
                    // co-rotating frame — for the grain and the far rocks.
                    let hit = ray * t_face - c;
                    let e1 = normalize(cross(axis, vec3<f32>(0.0, 1.0, 0.0)));
                    let e2 = normalize(cross(axis, e1));
                    let flat = hit - axis * dot(hit, axis);
                    let rf = length(flat);
                    let theta = atan2(dot(flat, e2), dot(flat, e1)) - bd.look.w;
                    let tau_f = 6.2831853;
                    let along = (theta - tau_f * floor(theta / tau_f)) * (rr * 1.8);
                    let r_off = rf - rr * RING_INNER;
                    // The dust clumps with the rocks, so the veil is a
                    // coarse grain of the belt's own hash (one grain per
                    // twenty cells) — from afar the ring looks made of
                    // something.
                    let grain_m = BELT_CELL_M * 20.0;
                    let ga = along / grain_m;
                    let gr = r_off / grain_m;
                    let g00 = belt_unit(belt_hash(i32(floor(ga)), i32(floor(gr)), 9, 5u));
                    let g10 = belt_unit(belt_hash(i32(floor(ga)) + 1, i32(floor(gr)), 9, 5u));
                    let g01 = belt_unit(belt_hash(i32(floor(ga)), i32(floor(gr)) + 1, 9, 5u));
                    let g11 = belt_unit(belt_hash(i32(floor(ga)) + 1, i32(floor(gr)) + 1, 9, 5u));
                    let gf = vec2<f32>(fract(ga), fract(gr));
                    let gs = gf * gf * (3.0 - 2.0 * gf);
                    let grain = mix(mix(g00, g10, gs.x), mix(g01, g11, gs.x), gs.y);
                    let dust = vec3<f32>(0.42, 0.43, 0.45) * lit * 1.1;
                    // From afar: dim, but not see-through — a belt's worth
                    // of rock and dust hides the stars behind it, otherwise
                    // they read as bright rocks. Only once the camera is
                    // well clear of the plane, and never behind the planet.
                    let rad = rf / rr;
                    let in_ring = smoothstep(1.62, 1.66, rad) * (1.0 - smoothstep(1.96, 2.0, rad));
                    let behind = ur.cover > 0.5 && t_face > length(c);
                    let clear = smoothstep(RING_HAZE_M, 4.0 * RING_HAZE_M, h_cam);
                    if (!behind) {
                        let far_k = in_ring * clear;
                        rgb = mix(rgb, dust, far_k * 0.16 * (0.45 + 1.1 * grain));
                        alpha = max(alpha, far_k * 0.92);
                    }
                    // From inside: the run through the haze as an optical
                    // depth. Sun-lit dust forward-scatters, so the band
                    // brightens toward the Sun; radiance past 1 is the
                    // bloom's to spread.
                    let od = run / RING_HAZE_FREE_M;
                    let near_a = (1.0 - exp(-od)) * (1.0 - clear);
                    if (near_a > 0.001) {
                        let fwd = max(dot(ray, sun), 0.0);
                        let haze = dust * (0.55 + 0.35 * grain) * (1.0 + 3.0 * pow(fwd, 8.0));
                        rgb = mix(rgb, haze, near_a);
                        alpha = max(alpha, near_a);
                    }
                    // The rocks themselves, from here to the horizon of the
                    // ring: the belt's own population in the ring's
                    // co-rotating frame. Out to where they are under a
                    // pixel; fading in where the live rocks take over.
                    // A pixel's footprint on the plane: the hit's distance
                    // over the screen's height in pixels, stretched by the
                    // grazing angle.
                    let px_m = t_face * 2.0 * bd.params.x / bd.params.y / max(dn_mag, 0.05);
                    let tangent = normalize(cross(axis, flat / max(rf, 1.0)));
                    let radial = flat / max(rf, 1.0);
                    let near = smoothstep(2500.0, 6000.0, t_face);
                    if (px_m < 400.0 && near > 0.001 && !behind) {
                        let sp = belt_specks(along, r_off, px_m, tangent, radial, axis);
                        if (sp.cover > 0.001) {
                            let light = max(dot(sp.n, sun), 0.0);
                            // Rock, not star: dull grey-brown, and where a
                            // rock is well under a pixel it melts into the
                            // grain rather than staying a bright point.
                            let rock = vec3<f32>(0.26, 0.24, 0.21) * (light * 1.1 + 0.08);
                            let far = 1.0 - smoothstep(120.0, 400.0, px_m);
                            let k = sp.cover * near * far * in_ring;
                            rgb = mix(rgb, rock, k);
                            alpha = max(alpha, k);
                        }
                    }
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
