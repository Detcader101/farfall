// planet.wgsl — analytic planet (SPEC §6.5, pass: planet)
//
// Lane: A (vertex+fragment only). Cost class: moderate (one ray-sphere
// intersection plus ~8 noise evaluations, only on covered pixels).
//
// The planet is not geometry. There is no mesh, no tessellation, no LOD chunking
// and no heightmap texture: a fullscreen triangle casts one ray per pixel at an
// analytic sphere, and everything you see — continents, ice, cities, the
// terminator — is computed at the hit point. Curvature is therefore exact at
// every altitude, and the whole planet costs zero bytes of asset budget (P2).
//
// Antialiasing is analytic, not MSAA. The limb is a *shader* edge, not a
// geometry edge, so multisampling cannot see it (measured: 4x MSAA costs the
// same as 1x and changes nothing). Coverage is instead derived from the angular
// width of one pixel via fwidth, which gives a true one-pixel edge at any
// resolution and any apparent size.

struct Planet {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: planet centre relative to the camera, metres. w: radius, metres.
    // Camera-relative: the subtraction happens in f64 on the CPU, so f32 here
    // only ever carries a *local* offset and never a world coordinate (P3).
    centre_radius: vec4<f32>,
    // xyz: unit vector pointing at the sun.
    sun_dir: vec4<f32>,
    // rgb: atmosphere colour. w: optical density — how much air the line of
    // sight has to cross before the surface disappears into it.
    atmosphere: vec4<f32>,
    // x: cloud coverage in [0,1]. y: cloud shell altitude, metres.
    // z: edge sharpness. w: weather phase (advances the flow field).
    cloud_shape: vec4<f32>,
    // rgb: cloud albedo. w: strength of the shadow clouds cast on the surface.
    cloud_look: vec4<f32>,
}

@group(0) @binding(0) var<uniform> planet: Planet;

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
/// Base frequency of the elevation field, in units of the unit sphere.
const TERRAIN_BASE_FREQ: f32 = 1.7;
/// Ceiling on elevation octaves. This is the quality dial: it bounds the cost
/// of a screen full of ground, which is the worst case for this renderer.
const TERRAIN_MAX_OCTAVES: i32 = 9;

// Cloud density on the shell at direction `n`, in [0,1].
//
// Domain warping is what turns noise into weather: displacing the sample point
// by another noise field bends the bands into fronts, swirls and cyclones
// instead of the isotropic cotton-wool that plain fbm gives. The warp is the
// difference between "procedural texture" and "sky".
// Cost matters here: this is evaluated per pixel over most of the screen, and
// the first version (three warp octaves plus a five-octave field, twice per
// pixel for the shadow) cost more than half the frame. Two warp components and
// a three-octave field give the same read for roughly a third of the taps.
fn cloud_density(n: vec3<f32>, coverage: f32, sharpness: f32, phase: f32) -> f32 {
    let drift = vec3<f32>(phase * 0.05, 0.0, phase * 0.02);
    // Two warp components, and plain vnoise rather than fbm: a warp only has to
    // bend the field, and the extra octaves are invisible after thresholding
    // while costing three lattice evaluations each.
    let warp = vec2<f32>(
        vnoise(n * 2.6 + drift),
        vnoise(n * 2.6 + vec3<f32>(5.2, 1.3, 8.7) + drift),
    ) - vec2<f32>(0.5);

    // Latitude banding, the way a rotating atmosphere organises itself, folded
    // into the warp rather than paid for as a separate octave.
    let warped = n * 5.5
        + vec3<f32>(warp.x, warp.y * 0.45, warp.y) * 1.7
        + drift;
    let field = fbm3(warped);

    // Coverage is a threshold on that field: 0 clears the sky, 1 overcasts it.
    // The soft edge keeps cloud borders from aliasing.
    let cut = 1.0 - coverage;
    let d = smoothstep(cut - 0.20, cut + 0.20, field);
    return pow(clamp(d, 0.0, 1.0), max(sharpness, 0.05));
}

// Surface colour at a point on the unit sphere, before lighting.
fn surface_albedo(n: vec3<f32>, elevation: f32) -> vec3<f32> {
    let land_amount = smoothstep(SEA_LEVEL - 0.006, SEA_LEVEL + 0.006, elevation);

    // Ocean: deep basins darken away from the coast. No noise — the elevation
    // field already sampled is enough.
    let deep = vec3<f32>(0.010, 0.042, 0.115);
    let shallow = vec3<f32>(0.045, 0.200, 0.330);
    let ocean = mix(deep, shallow, smoothstep(SEA_LEVEL - 0.07, SEA_LEVEL, elevation));

    // Roughly half a water world is water, and none of those pixels need the
    // biome or ice fields. Bailing here removes two noise evaluations from
    // every ocean pixel on screen.
    if (land_amount < 0.002) {
        return ocean;
    }

    // Land: a dryness field pushes vegetation toward desert.
    let dryness = 0.5 + (fbm3(n * 4.0 + 4.7) - 0.5) * 2.2;
    let verdant = vec3<f32>(0.085, 0.230, 0.080);
    let arid = vec3<f32>(0.330, 0.265, 0.150);
    var land = mix(verdant, arid, smoothstep(0.35, 0.62, dryness));

    // Higher ground goes grey and bare.
    land = mix(
        land,
        vec3<f32>(0.28, 0.26, 0.24),
        smoothstep(SEA_LEVEL + 0.10, SEA_LEVEL + 0.20, elevation),
    );

    // Ice caps, with a ragged edge so the poles are not two clean circles.
    let latitude = abs(n.y) + (fbm3(n * 7.0) - 0.5) * 0.22;
    let ice = smoothstep(0.70, 0.84, latitude);
    land = mix(land, vec3<f32>(0.90, 0.94, 0.98), ice);

    return mix(ocean, mix(land, vec3<f32>(0.86, 0.92, 0.97), ice * 0.6), land_amount);
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
    let distance_to_centre = length(centre);
    // Below the surface. Until there is collision, the ship can end up inside
    // the planet, and discarding here deleted the world and left the pilot
    // staring at the starfield through solid ground. Filling with dense
    // unlit air reads as "you are inside something", which is at least true.
    if (distance_to_centre <= radius) {
        let murk = planet.atmosphere.rgb * 0.02;
        return vec4<f32>(tonemap(murk, planet.params.w), 1.0);
    }

    // Angular geometry, which is what both the antialiasing and the atmosphere
    // rim are naturally expressed in.
    let to_centre = centre / distance_to_centre;
    let angle = acos(clamp(dot(ray, to_centre), -1.0, 1.0));
    let limb = asin(clamp(radius / distance_to_centre, 0.0, 1.0));

    // Half a pixel of angular width, from the true gradient magnitude.
    //
    // fwidth() is the L1 norm |dpdx| + |dpdy|, which is already about a whole
    // pixel AND over-reads the real gradient by up to sqrt(2) on diagonals — so
    // using it as the smoothstep half-width gives a two-pixel edge that is
    // measurably softer at the disc's diagonals than at its sides. The L2 norm,
    // halved, is a genuine one-pixel edge in every direction.
    //
    // (acos is safe here despite its derivative diverging at 1: `angle` is a
    // geodesic distance on the direction sphere, so |grad| == 1 everywhere.)
    let grad = vec2<f32>(dpdx(angle), dpdy(angle));
    let pixel_angle = max(0.5 * length(grad), 1e-7);
    let coverage = 1.0 - smoothstep(limb - pixel_angle, limb + pixel_angle, angle);

    // Atmosphere rim: a thin shell of glow hugging the limb from BOTH sides,
    // scaled to apparent size so it stays proportionate at any distance. A cheap
    // stand-in for the scattering LUTs of M2, but it reads as air rather than as
    // a hard edge.
    //
    // The falloff must be symmetric about the limb. Using max(angle - limb, 0)
    // makes the exponent zero across the entire disc, so exp() returns 1 and the
    // "rim" floods the whole planet with a flat blue wash — which is exactly
    // what the first version did.
    // Thickness scales with the atmosphere's density, so a soupy world wears a
    // fat halo and a thin one a hairline.
    let rim_falloff = max(limb * 0.05 * (0.5 + planet.atmosphere.w * 2.5), 1e-6);
    // Windowed to reach exactly zero. A bare exponential never does, so the
    // pixel-kill threshold below becomes a visible arc wherever the glow is
    // still bright enough to see when it is cut off — which additive
    // compositing made obvious.
    let from_limb = abs(angle - limb);
    let rim = exp(-from_limb / rim_falloff)
        * smoothstep(8.0 * rim_falloff, 3.0 * rim_falloff, from_limb);
    let sun = normalize(planet.sun_dir.xyz);

    // Only the lit side of the limb glows — which requires the normal at *this
    // pixel's* point on the limb, not the direction to the planet's centre. The
    // latter is constant across the whole screen, so using it makes the ring
    // glow with one uniform brightness the whole way round, night side included.
    // For a grazing ray the limb normal satisfies dot(n, to_centre) = -R/d.
    let sin_limb = radius / distance_to_centre;
    let perp = ray - to_centre * dot(ray, to_centre);
    // No normalize(): perp goes to zero at the disc's centre, where the rim has
    // already faded out anyway, and normalize(0) is a NaN waiting to happen.
    let tangent = perp / max(length(perp), 1e-6);
    let limb_normal = -to_centre * sin_limb
        + tangent * sqrt(max(1.0 - sin_limb * sin_limb, 0.0));
    let rim_lit = smoothstep(-0.25, 0.25, dot(limb_normal, sun));
    // Halved relative to the alpha-blended version: additive compositing
    // delivers the full value instead of a coverage-weighted fraction of it.
    let rim_colour = planet.atmosphere.rgb * rim * rim_lit * 0.5;

    // --- surface -------------------------------------------------------
    // Only shade pixels the disc actually covers; the noise here is the
    // expensive part of the frame.
    var surface = vec3<f32>(0.0);
    if (coverage > 0.001) {
        // Ray-sphere: the origin is the camera, which camera-relative rendering
        // puts at exactly zero, so the usual origin terms vanish.
        let along = dot(ray, centre);
        let discriminant = max(along * along - (dot(centre, centre) - radius * radius), 0.0);
        let hit_t = along - sqrt(discriminant);
        let hit = ray * hit_t;
        let normal = normalize(hit - centre);

        // How much of the world one pixel covers at this hit point, in the
        // noise's own units — the basis for choosing how much detail to
        // compute. Foreshortening counts: a pixel near the horizon smears
        // across far more ground than one directly below.
        let view_cos_lod = max(dot(normal, -ray), 0.08);
        let footprint_m = hit_t * 2.0 * pixel_angle / view_cos_lod;
        let footprint_noise = footprint_m * TERRAIN_BASE_FREQ / radius;
        // Nyquist: anything finer than two pixels is not detail, it is noise.
        let detail_limit = clamp(1.0 / max(footprint_noise * 2.0, 1e-9), 1.0, 4096.0);

        // Value-noise fbm is a sum of independent samples, so it clusters hard
        // around 0.5 (central limit). Used raw against a sea level of 0.5 it
        // gives a knife-edge coastline everywhere and almost no dry land.
        // Expanding the deviation about the midpoint turns noise into
        // continents.
        let elevation = 0.5
            + (fbm_lod(
                normal * TERRAIN_BASE_FREQ + 11.3,
                detail_limit,
                TERRAIN_MAX_OCTAVES,
            ) - 0.5) * 2.9;
        let albedo = surface_albedo(normal, elevation);

        // Lighting. A crisp terminator over a soft one: readability first (P1).
        let n_dot_l = dot(normal, sun);
        let day = smoothstep(-0.05, 0.09, n_dot_l);
        surface = albedo * day * 1.35;

        // Specular on water only — it is what tells you the dark parts are
        // liquid rather than merely dark.
        let land_amount = smoothstep(SEA_LEVEL - 0.006, SEA_LEVEL + 0.006, elevation);
        let half_vec = normalize(sun - ray);
        // Tight exponent: from low altitude a broad lobe reads as a blurry
        // smear across the ocean rather than as the sun's reflection.
        let spec = pow(max(dot(normal, half_vec), 0.0), 420.0) * (1.0 - land_amount) * day;
        surface += vec3<f32>(1.0, 0.95, 0.86) * spec * 0.9;

        // Night side: settlement clusters on habitable land near the coast.
        let night = 1.0 - day;
        let coastal = 1.0 - smoothstep(0.0, 0.10, abs(elevation - (SEA_LEVEL + 0.035)));
        let habitable = 1.0 - smoothstep(0.55, 0.78, abs(normal.y));
        // ~1.4 sigma above the (now correctly centred) mean of fbm3.
    let lights = step(0.67, fbm3(normal * 150.0)) * land_amount * coastal * habitable;
        surface += vec3<f32>(1.0, 0.72, 0.38) * lights * night * 2.2;

        // A little ambient so the dark side is a silhouette, not a hole.
        surface += albedo * 0.02;

        // Clouds cast shadows: sample the shell along the direction of the sun,
        // so the shadow lands offset from the cloud that throws it. Skipped on
        // the night side, where it would be a second full noise evaluation
        // multiplied by zero.
        if (day > 0.01 && planet.cloud_look.w > 0.001) {
            let shadow_dir = normalize(normal + sun * 0.10);
            let shadow = cloud_density(
                shadow_dir,
                planet.cloud_shape.x,
                planet.cloud_shape.z,
                planet.cloud_shape.w,
            );
            surface *= 1.0 - shadow * planet.cloud_look.w * day;
        }

        // Limb darkening: the line of sight leaves at a grazing angle near the
        // edge.
        surface *= mix(1.0, 0.80, smoothstep(0.5, 1.0, angle / max(limb, 1e-6)));

        // Aerial perspective. The air column the view crosses grows as the
        // surface turns away, so the ground fades into the sky toward the limb
        // — Beer-Lambert on an approximate airmass. This is the "fog", and it
        // is what makes a thick atmosphere feel thick rather than merely
        // coloured: at high density the horizon dissolves entirely.
        // The 1/cos airmass diverges at the horizon, and taken literally it
        // erases the whole lower half of the frame from low orbit. Clamping the
        // grazing angle keeps the horizon hazy rather than solid.
        let view_cos = max(dot(normal, -ray), 0.18);
        let airmass = planet.atmosphere.w / view_cos;
        let haze = 1.0 - exp(-airmass);
        let sky = planet.atmosphere.rgb * (0.35 + 0.65 * day);
        surface = mix(surface, sky, clamp(haze, 0.0, 1.0));
    }

    // --- clouds ---------------------------------------------------------
    // A shell above the surface, intersected separately. Giving the clouds
    // their own radius is what buys parallax against the ground: they slide
    // over the terrain as the ship moves instead of being painted onto it.
    var cloud_rgb = vec3<f32>(0.0);
    var cloud_a = 0.0;
    let cloud_radius = radius + planet.cloud_shape.y;
    if (planet.cloud_shape.x > 0.001) {
        let along_c = dot(ray, centre);
        let disc_c = along_c * along_c - (dot(centre, centre) - cloud_radius * cloud_radius);
        if (disc_c > 0.0) {
            let root = sqrt(disc_c);
            // Near root normally; the far one once the ship is inside the deck,
            // so clouds stay overhead rather than vanishing on descent.
            let near = along_c - root;
            let t_c = select(along_c + root, near, near > 0.0);
            if (t_c > 0.0) {
                let shell_normal = normalize(ray * t_c - centre);
                let density = cloud_density(
                    shell_normal,
                    planet.cloud_shape.x,
                    planet.cloud_shape.z,
                    planet.cloud_shape.w,
                );

                // Lit like a diffuse sheet, with a little wrap so the
                // terminator does not cut them off as a hard line.
                let lit = clamp(dot(shell_normal, sun) * 0.5 + 0.5, 0.0, 1.0);
                cloud_rgb = planet.cloud_look.rgb * (lit * lit * 1.25 + 0.03);

                // The shell has its own limb, and it needs the same analytic
                // edge as the body or the cloud deck ends in a hard arc.
                let shell_limb = asin(clamp(cloud_radius / distance_to_centre, 0.0, 1.0));
                let shell_cov = 1.0 - smoothstep(
                    shell_limb - pixel_angle, shell_limb + pixel_angle, angle
                );
                cloud_a = clamp(density, 0.0, 1.0) * shell_cov;
            }
        }
    }

    // --- compositing ----------------------------------------------------
    // Premultiplied alpha. Only the *body* occludes: alpha is the disc's
    // coverage alone, and the atmosphere is added on top of whatever is behind
    // it. An outer glow has an optical depth far below one, so compositing it
    // with `over` would dim the starfield in a wide ring around the planet —
    // and giving the glow a coverage-weighted alpha inside the limb while it
    // composited at full strength outside is what drew a dark ring around the
    // first version.
    //
    // Because the same rim term is added on both sides of the limb, the two
    // sides agree by construction rather than by tuning two coefficients to
    // match.
    // Clouds sit over the body, so they occlude it and each other in order.
    let alpha = cloud_a + coverage * (1.0 - cloud_a);
    if (alpha < 0.002 && rim <= 0.0) {
        discard;
    }

    // Tonemap each layer before combining: tonemapping an already-blended
    // premultiplied colour is a different operation and darkens the edge.
    let surface_ldr = tonemap(surface, exposure);
    let cloud_ldr = tonemap(cloud_rgb, exposure);
    let rim_ldr = tonemap(rim_colour, exposure);
    var rgb = cloud_ldr * cloud_a + surface_ldr * coverage * (1.0 - cloud_a) + rim_ldr;
    rgb += vec3<f32>(dither_px(in.pos.xy)) * max(alpha, rim);
    return vec4<f32>(rgb, alpha);
}
