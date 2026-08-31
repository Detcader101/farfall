// planet.wgsl — analytic planet with baked fields (SPEC §6.5, pass: planet)
//
// Lane: A (vertex+fragment only). Cost class: cheap — the heavy noise stacks
// were moved to a one-time bake (bake.wgsl); per pixel this pass does a few
// texture fetches and closed-form lighting. See the bake header for why.
//
// The planet is not geometry: a fullscreen triangle casts one ray per pixel at
// an analytic sphere, and the surface, clouds and sky are computed at the hit.
// Antialiasing is analytic (from the angular width of one pixel) because the
// limb is a shader edge MSAA cannot see.
//
// The atmosphere is ONE model, composited strictly back-to-front:
//
//     ground  →  cloud deck  →  air between camera and all of it
//
// Its previous life as three independent patches — an additive rim glow, an
// aerial-perspective mix, and a bolted-on in-scatter term — produced exactly
// the artifacts three unsynchronised systems produce: seams where they met,
// stars through the daytime sky, and a razor-edged cloud ceiling. One optical
// depth, one sky colour, one `over` stack.

struct Planet {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: planet centre relative to the camera, metres (subtracted in f64 on
    // the CPU; f32 here only ever holds a local offset — P3). w: radius, m.
    centre_radius: vec4<f32>,
    // xyz: unit vector toward the sun.
    sun_dir: vec4<f32>,
    // rgb: atmosphere colour. w: optical density.
    atmosphere: vec4<f32>,
    // x: cloud coverage [0,1], y: deck altitude (m), z: edge sharpness,
    // w: weather phase (drifts the deck).
    cloud_shape: vec4<f32>,
    // rgb: cloud albedo. w: cloud shadow strength.
    cloud_look: vec4<f32>,
    // Two solid bodies that may stand between the camera and the planet:
    // xyz centre relative to the camera (m), w radius (m); w <= 0 is none.
    occluder0: vec4<f32>,
    occluder1: vec4<f32>,
}

// Distance along `ray` to the near surface of a sphere, or -1 if missed or
// behind the camera.
fn sphere_near(ray: vec3<f32>, centre: vec3<f32>, radius: f32) -> f32 {
    if (radius <= 0.0) {
        return -1.0;
    }
    let along = dot(ray, centre);
    let disc = along * along - (dot(centre, centre) - radius * radius);
    if (disc < 0.0) {
        return -1.0;
    }
    let t = along - sqrt(disc);
    return select(-1.0, t, t > 0.0);
}

@group(0) @binding(0) var<uniform> planet: Planet;
// Baked fields (bake.wgsl): R elevation, G dryness, B light speckle, A ice.
@group(0) @binding(1) var surface_tex: texture_2d<f32>;
// R: raw cloud field, thresholded live so presets need no re-bake.
@group(0) @binding(2) var cloud_tex: texture_2d<f32>;
@group(0) @binding(3) var maps_samp: sampler;

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

const SEA_LEVEL: f32 = 0.5;
const TAU: f32 = 6.28318531;
const PI: f32 = 3.14159265;

// Equirect sample with wrap-aware gradients. The longitude seam makes uv.x
// jump 1→0 across one pixel; naive derivative-based mip selection reads that
// as an enormous footprint and drops to the smallest mip, drawing a blurry
// meridian down the planet. Wrapping the gradient fixes the seam for the cost
// of two fracts.
fn sample_equirect(tex: texture_2d<f32>, n: vec3<f32>) -> vec4<f32> {
    let uv = vec2<f32>(
        atan2(n.z, n.x) / TAU + 0.5,
        acos(clamp(n.y, -1.0, 1.0)) / PI,
    );
    var du = dpdx(uv);
    var dv = dpdy(uv);
    du.x = fract(du.x + 0.5) - 0.5;
    dv.x = fract(dv.x + 0.5) - 0.5;
    return textureSampleGrad(tex, maps_samp, uv, du, dv);
}

// Cloud density at direction `n`, from the baked field. Coverage is a live
// threshold (0 clears the sky, 1 overcasts it); drift rotates the lookup so
// the deck moves without re-baking.
fn cloud_density(n: vec3<f32>) -> f32 {
    let a = planet.cloud_shape.w * 0.05;
    let rotated = vec3<f32>(
        n.x * cos(a) - n.z * sin(a),
        n.y,
        n.x * sin(a) + n.z * cos(a),
    );
    let field = sample_equirect(cloud_tex, rotated).r;
    // One-sided threshold: density exists only ABOVE the cut. The previous
    // band straddled it, which floated a ~30% veil over the entire sky —
    // "42% coverage" rendered as a ninety-percent white sheet, because a veil
    // everywhere plus banks somewhere covers everything.
    let cut = 1.0 - planet.cloud_shape.x;
    let d = smoothstep(cut, cut + 0.30, field);
    return pow(clamp(d, 0.0, 1.0), max(planet.cloud_shape.z, 0.05));
}

// Surface colour from the baked fields, before lighting.
fn surface_albedo(fields: vec4<f32>) -> vec3<f32> {
    let elevation = fields.r;
    let land_amount = smoothstep(SEA_LEVEL - 0.006, SEA_LEVEL + 0.006, elevation);

    let deep = vec3<f32>(0.010, 0.042, 0.115);
    let shallow = vec3<f32>(0.045, 0.200, 0.330);
    let ocean = mix(deep, shallow, smoothstep(SEA_LEVEL - 0.07, SEA_LEVEL, elevation));
    if (land_amount < 0.002) {
        return ocean;
    }

    let verdant = vec3<f32>(0.085, 0.230, 0.080);
    let arid = vec3<f32>(0.330, 0.265, 0.150);
    var land = mix(verdant, arid, smoothstep(0.35, 0.62, fields.g));
    land = mix(
        land,
        vec3<f32>(0.28, 0.26, 0.24),
        smoothstep(SEA_LEVEL + 0.10, SEA_LEVEL + 0.20, elevation),
    );
    let latitude = abs_lat_with_noise(fields.a);
    let ice = smoothstep(0.70, 0.84, latitude);
    land = mix(land, vec3<f32>(0.90, 0.94, 0.98), ice);
    return mix(ocean, land, land_amount);
}

// The ice band needs |latitude| perturbed by noise; the noise is baked, but
// latitude is per-pixel. Passed through a global set in fs_main because WGSL
// has no closures — kept adjacent so the hack is at least visible.
var<private> current_abs_y: f32;
fn abs_lat_with_noise(ice_noise: f32) -> f32 {
    return current_abs_y + (ice_noise - 0.5) * 0.22;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let exposure = planet.params.w;
    let ray = view_ray(
        in.ndc, planet.right.xyz, planet.up.xyz, planet.forward.xyz,
        planet.params.x, planet.params.y,
    );

    let centre = planet.centre_radius.xyz;
    let radius = planet.centre_radius.w;
    let d_centre = length(centre);
    let sun = normalize(planet.sun_dir.xyz);
    let density = planet.atmosphere.w;
    // Visual scale height of the air, tied to the planet so presets transfer
    // between worlds of different sizes.
    let h_air = radius * 0.03;

    // Below the surface (no collision escape yet): dense unlit murk, opaque.
    if (d_centre <= radius) {
        return vec4<f32>(radiance(planet.atmosphere.rgb * 0.02, exposure), 1.0);
    }
    let h_cam = d_centre - radius;

    // ---- geometry ------------------------------------------------------
    let to_centre = centre / d_centre;
    let angle = acos(clamp(dot(ray, to_centre), -1.0, 1.0));
    let limb = asin(clamp(radius / d_centre, 0.0, 1.0));

    // Analytic AA: half a pixel of angular width from the true gradient
    // magnitude (fwidth is an L1 norm and over-reads diagonals — measured).
    let grad = vec2<f32>(dpdx(angle), dpdy(angle));
    let pixel_angle = max(0.5 * length(grad), 1e-7);
    let coverage = 1.0 - smoothstep(limb - pixel_angle, limb + pixel_angle, angle);

    // Ray-sphere. Camera at the origin by construction (camera-relative).
    let along = dot(ray, centre);
    let disc_body = along * along - (d_centre * d_centre - radius * radius);
    let hit_t = along - sqrt(max(disc_body, 0.0));

    // Occlusion: a body in front of the planet hides it, air and all. What
    // the planet shows along this ray starts at the ground if hit, else at
    // the sightline's closest approach (the air is thickest there); if a
    // body's surface is nearer than that, this pixel is the body's.
    let planet_t = select(max(along, 0.0), hit_t, coverage > 0.5);
    let t0 = sphere_near(ray, planet.occluder0.xyz, planet.occluder0.w);
    let t1 = sphere_near(ray, planet.occluder1.xyz, planet.occluder1.w);
    if ((t0 > 0.0 && t0 < planet_t) || (t1 > 0.0 && t1 < planet_t)) {
        discard;
    }

    // Closest-approach altitude for rays that miss: how deep the sightline
    // dips into the air. Looking away from the planet, the closest point is
    // the camera itself.
    let perp2 = max(d_centre * d_centre - along * along, 0.0);
    let h_min = select(h_cam, max(sqrt(perp2) - radius, 0.0), along > 0.0);

    // ---- ground --------------------------------------------------------
    var surface = vec3<f32>(0.0);
    var ground_normal = to_centre; // placeholder when no hit
    if (coverage > 0.001) {
        let hit = ray * hit_t;
        let normal = normalize(hit - centre);
        ground_normal = normal;
        current_abs_y = abs(normal.y);

        let fields = sample_equirect(surface_tex, normal);
        let albedo = surface_albedo(fields);
        let elevation = fields.r;
        let land_amount = smoothstep(SEA_LEVEL - 0.006, SEA_LEVEL + 0.006, elevation);

        // Crisp terminator over a soft one: readability first (P1).
        let n_dot_l = dot(normal, sun);
        let day = smoothstep(-0.05, 0.09, n_dot_l);
        surface = albedo * day * 1.35;

        // Specular on water only.
        let half_vec = normalize(sun - ray);
        let spec = pow(max(dot(normal, half_vec), 0.0), 420.0) * (1.0 - land_amount) * day;
        surface += vec3<f32>(1.0, 0.95, 0.86) * spec * 0.9;

        // Cloud shadows, offset toward the sun.
        if (day > 0.01 && planet.cloud_look.w > 0.001) {
            let shadow = cloud_density(normalize(normal + sun * 0.10));
            surface *= 1.0 - shadow * planet.cloud_look.w * day;
        }

        // Night-side settlements, on habitable coastal land.
        let night = 1.0 - day;
        if (night > 0.01) {
            let coastal = 1.0 - smoothstep(0.0, 0.10, abs(elevation - (SEA_LEVEL + 0.035)));
            let habitable = 1.0 - smoothstep(0.55, 0.78, abs(normal.y));
            let lights = step(0.67, fields.b) * land_amount * coastal * habitable;
            surface += vec3<f32>(1.0, 0.72, 0.38) * lights * night * 2.2;
        }

        surface += albedo * 0.02;
        // Grazing incidence darkens the limb.
        surface *= mix(1.0, 0.80, smoothstep(0.5, 1.0, angle / max(limb, 1e-6)));
    }

    // ---- cloud deck ----------------------------------------------------
    var cloud_rgb = vec3<f32>(0.0);
    var cloud_a = 0.0;
    let cloud_radius = radius + planet.cloud_shape.y;
    let inside_deck = d_centre < cloud_radius;
    if (planet.cloud_shape.x > 0.001) {
        let disc_c = along * along - (d_centre * d_centre - cloud_radius * cloud_radius);
        if (disc_c > 0.0) {
            let root = sqrt(disc_c);
            let near = along - root;
            // Inside the deck the near root is behind the camera: take the far
            // one, so the deck stays overhead on descent.
            let t_c = select(near, along + root, inside_deck || near <= 0.0);
            // The deck only shows where the ray crosses it BEFORE any ground:
            // from under the deck, rays to the ground never reach it.
            let before_ground = coverage < 0.5 || t_c < hit_t;
            if (t_c > 0.0 && before_ground) {
                let shell_normal = normalize(ray * t_c - centre);
                let dens = cloud_density(shell_normal);
                let lit = clamp(dot(shell_normal, sun) * 0.5 + 0.5, 0.0, 1.0);
                cloud_rgb = planet.cloud_look.rgb * (lit * lit * 1.25 + 0.03);

                // From outside, the deck has its own limb and needs the same
                // analytic edge as the body. From inside it has no edge at all
                // — every direction crosses it — which is precisely the fix
                // for the razor-sharp ceiling line: that line WAS the shell
                // limb being drawn from underneath.
                var shell_cov = 1.0;
                if (!inside_deck) {
                    let shell_limb = asin(clamp(cloud_radius / d_centre, 0.0, 1.0));
                    shell_cov = 1.0 - smoothstep(
                        shell_limb - pixel_angle, shell_limb + pixel_angle, angle
                    );
                }
                cloud_a = clamp(dens, 0.0, 1.0) * shell_cov;
            }
        }
    }

    // ---- the air, once -------------------------------------------------
    // One optical-depth model, evaluated per path segment from the endpoint
    // altitudes of an exponential shell:
    //
    //   vertical column between h0 and h1  ∝  |e^(-h0/H) − e^(-h1/H)|
    //   slanted                            ∝  that / |cos of the path|
    //   tangent chord at depth h_min       ∝  e^(-h_min/H) · sqrt(2R/H)
    //
    // and split at the cloud deck: the air IN FRONT of the deck veils it, the
    // air BEHIND it sits over the stars but under the clouds. The previous
    // version applied the whole column in front of everything, which whited
    // out a deck 200 m overhead with air that was almost entirely beyond it.
    let cos_up = -dot(ray, to_centre);
    let rho_cam = exp(-h_cam / h_air);
    // sqrt(2R/H) for this planet ≈ 8: how much longer a grazing chord runs
    // than a vertical column.
    let chord_boost = sqrt(2.0 * radius / h_air) * 0.98;

    var od_total: f32;
    if (coverage > 0.001) {
        // Camera to ground: the column below the camera, along the slant.
        // The 0.6 keeps the nadir view readable (P1): full physical depth
        // washed the map out from any altitude above the scale height.
        od_total = density * 0.6 * max(1.0 - rho_cam, 0.015) / max(abs(cos_up), 0.08);
    } else {
        // Through the shell and out: steep rays cross the column above the
        // camera once; grazing rays run the tangent chord at their lowest dip.
        // The daytime dome: low down, the whole sky is the air's colour
        // and the stars are gone; it thins on its own, longer scale height
        // (the sky stays blue well above the weather) and is black by the
        // top of the air. sun_dir.w is the pilot's SKY setting, 1 = stock.
        let h_sky = radius * 0.10;
        let dome = planet.sun_dir.w * 4.0 * exp(-h_cam / h_sky);
        let up_od = density * (rho_cam + dome) / max(cos_up, 0.10);
        let chord_od = density * exp(-h_min / h_air) * chord_boost;
        od_total = mix(chord_od, up_od, smoothstep(0.03, 0.25, cos_up));
    }

    // Split at the deck.
    var od_front = od_total;
    if (cloud_a > 0.001) {
        let rho_deck = exp(-planet.cloud_shape.y / h_air);
        od_front = min(
            density * abs(rho_cam - rho_deck) / max(abs(cos_up), 0.10),
            od_total,
        );
    }
    let od_behind = od_total - od_front;

    // The sky's own light, shared by both segments.
    let air_normal = select(
        normalize(-to_centre * 0.7 + ray * 0.3),
        ground_normal,
        coverage > 0.001,
    );
    let air_day = smoothstep(-0.15, 0.25, dot(air_normal, sun));
    let toward_sun = max(dot(ray, sun), 0.0);
    let air_light = planet.atmosphere.rgb
        * air_day
        * (0.75 + 0.9 * toward_sun * toward_sun);

    // Bright air hides what is behind it: occlusion follows the segment's own
    // scattered luminance, so a brilliant noon sky erases stars while a night
    // sky of identical depth — no light — leaves them alone.
    let lum_w = vec3<f32>(0.2126, 0.7152, 0.0722);
    let emit_front = air_light * (1.0 - exp(-od_front));
    let emit_behind = air_light * (1.0 - exp(-od_behind));
    let over_front = clamp(1.0 - exp(-dot(emit_front, lum_w) * 13.0), 0.0, 1.0);
    let over_behind = clamp(1.0 - exp(-dot(emit_behind, lum_w) * 13.0), 0.0, 1.0);

    // ---- compose: ground, far air, deck, near air ----------------------
    let surface_ldr = radiance(surface, exposure);
    let cloud_ldr = radiance(cloud_rgb, exposure);
    let front_ldr = radiance(emit_front / max(over_front, 1e-4), exposure);
    let behind_ldr = radiance(emit_behind / max(over_behind, 1e-4), exposure);

    var rgb = surface_ldr * coverage;
    var alpha = coverage;
    rgb = behind_ldr * over_behind + rgb * (1.0 - over_behind);
    alpha = over_behind + alpha * (1.0 - over_behind);
    rgb = cloud_ldr * cloud_a + rgb * (1.0 - cloud_a);
    alpha = cloud_a + alpha * (1.0 - cloud_a);
    rgb = front_ldr * over_front + rgb * (1.0 - over_front);
    alpha = over_front + alpha * (1.0 - over_front);

    if (alpha < 0.002) {
        discard;
    }
    rgb += vec3<f32>(dither_px(in.pos.xy)) * alpha;
    return vec4<f32>(rgb, alpha);
}
