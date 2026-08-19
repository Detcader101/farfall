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

// Surface colour at a point on the unit sphere, before lighting.
fn surface_albedo(n: vec3<f32>, elevation: f32) -> vec3<f32> {
    let land_amount = smoothstep(SEA_LEVEL - 0.006, SEA_LEVEL + 0.006, elevation);

    // Ocean: deep basins darken away from the coast.
    let deep = vec3<f32>(0.010, 0.042, 0.115);
    let shallow = vec3<f32>(0.045, 0.200, 0.330);
    let ocean = mix(deep, shallow, smoothstep(SEA_LEVEL - 0.07, SEA_LEVEL, elevation));

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
    if (distance_to_centre <= radius) {
        discard; // inside the planet: nothing sensible to draw yet
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
    let rim_falloff = max(limb * 0.05, 1e-6);
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
    let rim_colour = vec3<f32>(0.28, 0.48, 0.95) * rim * rim_lit * 0.5;

    // --- surface -------------------------------------------------------
    // Only shade pixels the disc actually covers; the noise here is the
    // expensive part of the frame.
    var surface = vec3<f32>(0.0);
    if (coverage > 0.001) {
        // Ray-sphere: the origin is the camera, which camera-relative rendering
        // puts at exactly zero, so the usual origin terms vanish.
        let along = dot(ray, centre);
        let discriminant = max(along * along - (dot(centre, centre) - radius * radius), 0.0);
        let hit = ray * (along - sqrt(discriminant));
        let normal = normalize(hit - centre);

        // Value-noise fbm is a sum of independent samples, so it clusters hard
        // around 0.5 (central limit). Used raw against a sea level of 0.5 it
        // gives a knife-edge coastline everywhere and almost no dry land.
        // Expanding the deviation about the midpoint turns noise into
        // continents.
        let elevation = 0.5 + (fbm5(normal * 1.7 + 11.3) - 0.5) * 2.9;
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

        // Limb darkening: the line of sight leaves at a grazing angle near the
        // edge. The atmospheric haze is NOT added here — it is one additive
        // layer applied on both sides of the limb below, which is what keeps
        // the two sides agreeing.
        surface *= mix(1.0, 0.80, smoothstep(0.5, 1.0, angle / max(limb, 1e-6)));
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
    let alpha = coverage;
    if (alpha < 0.002 && rim <= 0.0) {
        discard;
    }

    // Tonemap each layer before combining: tonemapping an already-blended
    // premultiplied colour is a different operation and darkens the edge.
    let surface_ldr = tonemap(surface, exposure);
    let rim_ldr = tonemap(rim_colour, exposure);
    var rgb = surface_ldr * coverage + rim_ldr;
    rgb += vec3<f32>(dither_px(in.pos.xy)) * max(alpha, rim);
    return vec4<f32>(rgb, alpha);
}
