// planet.wgsl — analytic planet: baked continents, live relief, a scattering
// atmosphere, a cloud deck and night-side cities (SPEC §6.5, pass: planet)
//
// Lane: A (vertex+fragment only). Cost class: moderate and distance-banded —
// from orbit a few fetches and an eight-sample scattering march per covered
// pixel; low down the relief adds noise octaves only as the pixel's
// footprint on the ground shrinks (fbm-LOD in `relief`), never more than the
// pixel can show. The worst case is a screen full of ground at a few hundred
// metres; the number is in docs/polish/world.md.
//
// The planet is not geometry: a fullscreen triangle casts one ray per pixel at
// an analytic sphere, and the surface, clouds and sky are computed at the hit.
// Antialiasing is analytic (from the angular width of one pixel) because the
// limb is a shader edge MSAA cannot see.
//
// The atmosphere is single scattering, marched. An exponential shell with a
// Rayleigh component — the preset's colour is its scattering tint, so the sky
// is that colour and the sunsets its complement — and a low Mie haze for the
// horizon's glow and the Sun's forward scatter. Eight samples along the ray,
// clustered at its lowest point, each lit by the Sun through the air above it
// (an analytic Chapman column, zero in the planet's shadow). The one integral
// gives the daytime dome, its thinning with altitude, the aerial perspective
// on the ground, the glowing limb seen from orbit and the twilight at the
// terminator; the previous model had each of these as a separate term and
// showed the seams between them (a limb that darkened into space, a dome
// that stayed full blue at twelve kilometres).

struct Planet {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: planet centre relative to the camera, metres (subtracted in f64 on
    // the CPU; f32 here only ever holds a local offset — P3). w: radius, m.
    centre_radius: vec4<f32>,
    // xyz: unit vector toward the sun. w: the SKY setting (1 = stock).
    sun_dir: vec4<f32>,
    // rgb: atmosphere colour (the scattering tint). w: optical density —
    // the vertical optical depth of the strongest channel.
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
    // x: TERRAIN DETAIL (0 baked only, 1 stock, 2 an octave finer),
    // y: CLOUDS (a multiplier on the preset's coverage, 0 clears the sky),
    // z: CITY LIGHTS (0 off, 1 stock), w: unused.
    detail: vec4<f32>,
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
// Baked fields (bake.wgsl): R elevation, G dryness, B settlement, A ice.
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

// ---- the air, relative to the planet's radius so presets transfer -------
// Rayleigh scale height: the dome is full at 3 km over a 64 km world, pale
// at 8, dim at 15, gone by 25 (the SKY feature's contract).
const H_RAY: f32 = 0.07;
// The haze: a thin layer that gives the horizon its glow.
const H_MIE: f32 = 0.012;
// The haze's vertical optical depth as a share of the Rayleigh one.
const MIE_K: f32 = 0.15;
const MIE_G: f32 = 0.72;
// The shell's top, six Rayleigh scale heights up.
const TOP: f32 = 0.42;
// The Sun's radiance at the top of the air, in the scene's units.
const SUN_I: f32 = 26.0;
// Samples per half-segment of the march (eight per segment).
const SAMPLES: i32 = 4;

// ---- the ground -----------------------------------------------------------
// Metres of height per unit of the elevation field: sea level is 0.5 and
// the baked continents peak near 0.7, so a two-kilometre range.
const RELIEF_M: f32 = 10000.0;
// The relief's base octave, cycles per radian — 1.5 km hills on a 64 km
// world; six octaves take it to fifty metres.
const DETAIL_FREQ: f32 = 42.0;
// Hills and mountains: how much of the field's range the live relief adds.
const HILL_AMP: f32 = 0.035;
const MOUNTAIN_AMP: f32 = 0.11;
// Cloud edge detail: octaves above the baked field's finest.
const CLOUD_DETAIL_FREQ: f32 = 110.0;
// The cities: the street grid's block, and the lit-point lattice, metres.
const BLOCK_M: f32 = 240.0;
const LOT_M: f32 = 60.0;

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

// The relief at direction `n`: hills as plain fbm, mountains as ridged
// noise (sharp crests), octaves added only up to `max_freq` (in units of
// the base octave) — the pixel's footprint decides, so from orbit this is
// one octave and at a few hundred metres it is six, with the last fading
// in so nothing pops. Mean 0.5 at any count.
fn relief(n: vec3<f32>, max_freq: f32, mountain: f32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    let p = n * DETAIL_FREQ + vec3<f32>(3.7, 1.3, 9.1);
    for (var i = 0; i < 6; i += 1) {
        let weight = clamp(max_freq / freq - 1.0, 0.0, 1.0);
        if (weight <= 0.0) {
            break;
        }
        let v = vnoise(p * freq);
        let r = 1.0 - abs(v * 2.0 - 1.0);
        sum += amp * weight * mix(v, r * r, mountain);
        norm += amp * weight;
        amp *= 0.5;
        freq *= 2.03;
    }
    return sum / max(norm, 1e-6);
}

// Height of the ground at `n`, in field units (sea level 0.5): the baked
// continents plus the live relief, hills everywhere on land and ridged
// mountains where the continents are already high.
fn height_at(n: vec3<f32>, max_freq: f32, detail_k: f32) -> f32 {
    let fields = sample_equirect(surface_tex, n);
    let base = fields.r;
    if (max_freq <= 1.0 || detail_k <= 0.0) {
        return base;
    }
    let land = smoothstep(SEA_LEVEL - 0.02, SEA_LEVEL + 0.01, base);
    let mountain = smoothstep(SEA_LEVEL + 0.07, SEA_LEVEL + 0.17, base);
    let amp = mix(HILL_AMP, MOUNTAIN_AMP, mountain) * mix(0.35, 1.0, land);
    return base + (relief(n, max_freq, mountain) - 0.5) * amp * detail_k;
}

// Cloud density at direction `n`, from the baked field plus live edge
// detail where the footprint allows. Coverage is a live threshold (0 clears
// the sky, 1 overcasts it); drift rotates the lookup so the deck moves
// without re-baking.
fn cloud_density(n: vec3<f32>, max_freq: f32) -> f32 {
    let a = planet.cloud_shape.w * 0.05;
    let rotated = vec3<f32>(
        n.x * cos(a) - n.z * sin(a),
        n.y,
        n.x * sin(a) + n.z * cos(a),
    );
    var field = sample_equirect(cloud_tex, rotated).r;
    if (max_freq > 1.0) {
        let fine = fbm_lod(rotated * CLOUD_DETAIL_FREQ + vec3<f32>(7.0, 2.0, 5.0), max_freq, 4);
        field += (fine - 0.5) * 0.28 * clamp(max_freq - 1.0, 0.0, 1.0);
    }
    // One-sided threshold: density exists only ABOVE the cut. A band that
    // straddled it floated a ~30% veil over the entire sky.
    let coverage = clamp(planet.cloud_shape.x * planet.detail.y, 0.0, 1.0);
    let cut = 1.0 - coverage;
    let d = smoothstep(cut, cut + 0.30, field);
    return pow(clamp(d, 0.0, 1.0), max(planet.cloud_shape.z, 0.05));
}

// ---- scattering -------------------------------------------------------------

// Chapman function: how much longer the column of an exponential atmosphere
// is along a slant at zenith cosine `cz` than straight up, at radius `x`
// scale heights from the centre. Schüler's closed form; below the horizon
// the column is the two grazing halves less the part behind.
fn chapman(x: f32, cz: f32) -> f32 {
    let c = sqrt(x * 1.5707963);
    if (cz >= 0.0) {
        return c / ((c - 1.0) * cz + 1.0);
    }
    let sz = sqrt(max(1.0 - cz * cz, 0.0));
    let xt = x * sz;
    return 2.0 * exp(min(x - xt, 30.0)) * sqrt(xt * 1.5707963) - c / ((c - 1.0) * (-cz) + 1.0);
}

// The Sun's transmittance to a point `p` (relative to the planet's centre):
// both components' slant columns above it, zero in the planet's shadow.
fn sun_trans(p: vec3<f32>, radius: f32, beta_r: vec3<f32>, beta_m: f32) -> vec3<f32> {
    let r = length(p);
    let h = max(r - radius, 0.0);
    let cz = dot(p / r, planet.sun_dir.xyz);
    let sz = sqrt(max(1.0 - cz * cz, 0.0));
    if (cz < 0.0 && r * sz < radius) {
        return vec3<f32>(0.0);
    }
    let hr = H_RAY * radius;
    let hm = H_MIE * radius;
    let od = beta_r * (exp(-h / hr) * chapman(r / hr, cz))
        + vec3<f32>(beta_m * exp(-h / hm) * chapman(r / hm, cz));
    return exp(-min(od, vec3<f32>(30.0)));
}

struct Air {
    // Light scattered toward the camera along the segment, radiance.
    insc: vec3<f32>,
    // What the segment lets through.
    trans: vec3<f32>,
}

// Single scattering along `ray` from t0 to t1, marched with the samples
// clustered at the segment's lowest point `t_low` (the closest approach for
// a chord through the shell, the ground for a ray that ends there), where
// the air is thickest. `beta_r`/`beta_m` are vertical optical depths.
fn march(
    t0: f32, t1: f32, t_low: f32, ray: vec3<f32>, centre: vec3<f32>, radius: f32,
    beta_r: vec3<f32>, beta_m: f32,
) -> Air {
    var out = Air(vec3<f32>(0.0), vec3<f32>(1.0));
    if (t1 <= t0) {
        return out;
    }
    let sun = planet.sun_dir.xyz;
    let hr = H_RAY * radius;
    let hm = H_MIE * radius;
    let mu = dot(ray, sun);
    // Rayleigh 3/(16π)(1+μ²); Henyey-Greenstein for the haze, 1/(4π) in front.
    let ph_r = 0.0596831 * (1.0 + mu * mu);
    let g2 = MIE_G * MIE_G;
    let ph_m = 0.0795775 * (1.0 - g2) / pow(max(1.0 + g2 - 2.0 * MIE_G * mu, 1e-4), 1.5);
    let tt = clamp(t_low, t0, t1);
    var od = vec3<f32>(0.0);
    var insc = vec3<f32>(0.0);
    for (var i = 0; i < 2 * SAMPLES; i += 1) {
        var t: f32;
        var ds: f32;
        if (i < SAMPLES) {
            let u = (f32(i) + 0.5) / f32(SAMPLES);
            let len = tt - t0;
            t = tt - len * (1.0 - u) * (1.0 - u);
            ds = 2.0 * len * (1.0 - u) / f32(SAMPLES);
        } else {
            let u = (f32(i - SAMPLES) + 0.5) / f32(SAMPLES);
            let len = t1 - tt;
            t = tt + len * u * u;
            ds = 2.0 * len * u / f32(SAMPLES);
        }
        if (ds <= 0.0) {
            continue;
        }
        let p = ray * t - centre;
        let h = max(length(p) - radius, 0.0);
        let dr = exp(-h / hr) / hr;
        let dm = exp(-h / hm) / hm;
        let ext = (beta_r * dr + vec3<f32>(beta_m * dm)) * ds;
        let t_cam = exp(-(od + ext * 0.5));
        od += ext;
        let t_sun = sun_trans(p, radius, beta_r, beta_m);
        insc += t_cam * t_sun * (beta_r * dr * ph_r + vec3<f32>(beta_m * dm * ph_m)) * ds;
    }
    out.insc = insc * SUN_I * planet.sun_dir.w;
    out.trans = exp(-od);
    return out;
}

// ---- the ground's colour ----------------------------------------------------

// Surface colour before lighting: water by depth, then land by height,
// slope, dryness and latitude — sand at the shore, green to arid lowland,
// grey rock on the steep and the high, snow above a line that falls toward
// the poles.
fn surface_albedo(h: f32, fields: vec4<f32>, slope: f32, abs_lat: f32) -> vec3<f32> {
    let deep = vec3<f32>(0.010, 0.042, 0.115);
    let shallow = vec3<f32>(0.06, 0.30, 0.36);
    let ocean = mix(deep, shallow, smoothstep(SEA_LEVEL - 0.03, SEA_LEVEL, h));
    let land_amount = smoothstep(SEA_LEVEL - 0.0015, SEA_LEVEL + 0.0015, h);
    if (land_amount < 0.002) {
        return ocean;
    }
    let verdant = vec3<f32>(0.085, 0.230, 0.080);
    let arid = vec3<f32>(0.330, 0.265, 0.150);
    var land = mix(verdant, arid, smoothstep(0.35, 0.62, fields.g));
    let sand = vec3<f32>(0.42, 0.38, 0.26);
    land = mix(sand, land, smoothstep(SEA_LEVEL, SEA_LEVEL + 0.006, h));
    let rock = vec3<f32>(0.28, 0.26, 0.24);
    let high = smoothstep(SEA_LEVEL + 0.09, SEA_LEVEL + 0.17, h);
    land = mix(land, rock, max(high, smoothstep(0.35, 0.7, slope)));
    let lat = abs_lat + (fields.a - 0.5) * 0.22;
    let snow_line = SEA_LEVEL + mix(0.16, -0.02, smoothstep(0.45, 0.85, lat));
    let snow = smoothstep(snow_line - 0.02, snow_line + 0.02, h) * (1.0 - smoothstep(0.6, 0.85, slope));
    land = mix(land, vec3<f32>(0.90, 0.94, 0.98), snow);
    return mix(ocean, land, land_amount);
}

// The night side's cities: on habitable coastal lowland where the baked
// settlement field is high, a street grid with brighter avenues every
// fifth block and lit points on a finer lattice — sodium, white, and the
// odd cyan or magenta of the SPEC's neon direction. From orbit, where a
// block is under a pixel, the same thing as its mean: a warm glow. Emissive:
// radiance past 1, for the bloom to spread.
fn city_lights(n: vec3<f32>, radius: f32, urban: f32, fp: f32) -> vec3<f32> {
    // Ground coordinates in metres: longitude and latitude on the sphere
    // (longitude stretches a little toward the poles; cities live low).
    let g = vec2<f32>(atan2(n.z, n.x), asin(clamp(n.y, -1.0, 1.0))) * radius;
    let fpm = max(fp * radius, 0.05);
    // The grid.
    let q = g / BLOCK_M;
    let f = fract(q);
    let dxy = min(f, vec2<f32>(1.0) - f) * BLOCK_M;
    let cellq = floor(q);
    let avenue = step(0.5, f32(any(fract(cellq / 5.0) < vec2<f32>(0.01))));
    let w = max(mix(7.0, 16.0, avenue), fpm);
    let line = max(1.0 - smoothstep(0.0, w, dxy.x), 1.0 - smoothstep(0.0, w, dxy.y));
    let street = line * min(1.0, mix(7.0, 16.0, avenue) / fpm) * mix(0.35, 0.8, avenue);
    // The lit points.
    let qs = g / LOT_M;
    let cs = floor(qs);
    let hh = hash4(vec2<i32>(cs));
    let ps = cs + vec2<f32>(0.2) + hh.xy * 0.6;
    let d = length(qs - ps) * LOT_M;
    let pr = max(5.0, fpm);
    let point = (1.0 - smoothstep(0.0, pr, d)) * step(hh.z, 0.45) * min(1.0, 5.0 / fpm) * (0.6 + hh.w);
    let sodium = vec3<f32>(1.0, 0.62, 0.25);
    let white = vec3<f32>(0.95, 0.95, 1.0);
    let neon = select(vec3<f32>(0.25, 0.9, 1.0), vec3<f32>(1.0, 0.3, 0.9), hh.w > 0.5);
    var tint = mix(sodium, white, step(0.6, hh.x));
    tint = mix(tint, neon, step(0.88, hh.x));
    // Near to far: the grid and points resolve, then melt into the mean.
    let far_k = smoothstep(BLOCK_M * 0.15, BLOCK_M * 1.2, fpm);
    let near_l = sodium * street + tint * point;
    let far_l = sodium * 0.16;
    return mix(near_l, far_l, far_k) * urban;
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
    // The scattering tint: the preset's colour, its strongest channel at
    // the preset's density.
    let tint = planet.atmosphere.rgb / max(max(planet.atmosphere.r, planet.atmosphere.g), max(planet.atmosphere.b, 1e-3));
    let beta_r = tint * density;
    let beta_m = density * MIE_K;
    let detail_k = planet.detail.x;

    // Below the surface (no collision escape yet): dense unlit murk, opaque.
    if (d_centre <= radius) {
        return vec4<f32>(tonemap(planet.atmosphere.rgb * 0.02, exposure), 1.0);
    }

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

    // The air's shell.
    let r_top = radius * (1.0 + TOP);
    let disc_top = along * along - (d_centre * d_centre - r_top * r_top);
    if (disc_top <= 0.0 && coverage < 0.001) {
        discard;
    }
    let root_top = sqrt(max(disc_top, 0.0));
    let t_enter = max(along - root_top, 0.0);
    let t_exit = max(along + root_top, 0.0);

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

    // ---- ground --------------------------------------------------------
    var surface = vec3<f32>(0.0);
    if (coverage > 0.001) {
        let hit = ray * hit_t;
        let normal = normalize(hit - centre);
        // The pixel's footprint on the ground, in unit-sphere units: how
        // far the surface normal turns across one pixel. This is the whole
        // LOD: octaves and line widths are chosen against it.
        let fp = max(length(fwidth(normal)), 1e-7);
        let max_freq = 0.35 / (fp * DETAIL_FREQ) * exp2(detail_k - 1.0);

        // Height here and a footprint away along two tangents: the relief's
        // slope, for a shaded normal. The tangents also displace the baked
        // continents, so coasts get their relief too.
        let tx = normalize(cross(normal, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(normal.y) > 0.9)));
        let ty = cross(normal, tx);
        let fields = sample_equirect(surface_tex, normal);
        let h0 = height_at(normal, max_freq, detail_k);
        var n_shade = normal;
        var slope = 0.0;
        if (max_freq > 1.0 && detail_k > 0.0) {
            let e = max(fp * 1.5, 2e-5);
            let hx = height_at(normalize(normal + tx * e), max_freq, detail_k);
            let hy = height_at(normalize(normal + ty * e), max_freq, detail_k);
            let sx = (hx - h0) * RELIEF_M / (e * radius);
            let sy = (hy - h0) * RELIEF_M / (e * radius);
            // Water is flat.
            let on_land = smoothstep(SEA_LEVEL - 0.002, SEA_LEVEL + 0.002, h0);
            slope = length(vec2<f32>(sx, sy)) * on_land;
            n_shade = normalize(normal - (tx * sx + ty * sy) * on_land);
        }
        let albedo = surface_albedo(h0, fields, slope, abs(normal.y));
        let land_amount = smoothstep(SEA_LEVEL - 0.0015, SEA_LEVEL + 0.0015, h0);

        // Crisp terminator on the smooth globe (readability first, P1); the
        // relief shades within it. Sunlight reddens through the air above
        // the ground as the Sun sets, and the sky lights the shadows.
        let n_dot_l = dot(normal, sun);
        let day = smoothstep(-0.05, 0.09, n_dot_l);
        let sun_here = sun_trans(hit - centre, radius, beta_r, beta_m);
        let diffuse = max(dot(n_shade, sun), 0.0) * (0.6 + 0.4 * smoothstep(-0.2, 0.3, n_dot_l));
        surface = albedo * (diffuse * 1.5 * sun_here + tint * 0.10 * day + 0.006);

        // Specular on water only.
        let half_vec = normalize(sun - ray);
        let spec = pow(max(dot(normal, half_vec), 0.0), 420.0) * (1.0 - land_amount) * day;
        surface += vec3<f32>(1.0, 0.95, 0.86) * spec * 1.2 * sun_here;

        // Cloud shadows, offset toward the sun.
        if (day > 0.01 && planet.cloud_look.w > 0.001 && planet.detail.y > 0.0) {
            let shadow = cloud_density(normalize(normal + sun * 0.10), 0.35 / (fp * CLOUD_DETAIL_FREQ));
            surface *= 1.0 - shadow * planet.cloud_look.w * day;
        }

        // Night-side cities, on habitable coastal lowland.
        let night = 1.0 - day;
        if (night > 0.01 && planet.detail.z > 0.001) {
            let lowland = 1.0 - smoothstep(0.0, 0.07, h0 - SEA_LEVEL);
            let habitable = 1.0 - smoothstep(0.55, 0.78, abs(normal.y));
            let urban = smoothstep(0.52, 0.66, fields.b) * land_amount * lowland * habitable;
            if (urban > 0.001) {
                surface += city_lights(normal, radius, urban, fp) * night * planet.detail.z * 7.0;
            }
        }
    }

    // ---- cloud deck ----------------------------------------------------
    var cloud_rgb = vec3<f32>(0.0);
    var cloud_a = 0.0;
    var t_c = 0.0;
    let cloud_radius = radius + planet.cloud_shape.y;
    let inside_deck = d_centre < cloud_radius;
    if (planet.cloud_shape.x * planet.detail.y > 0.001) {
        let disc_c = along * along - (d_centre * d_centre - cloud_radius * cloud_radius);
        if (disc_c > 0.0) {
            let root = sqrt(disc_c);
            let near = along - root;
            // Inside the deck the near root is behind the camera: take the far
            // one, so the deck stays overhead on descent.
            t_c = select(near, along + root, inside_deck || near <= 0.0);
            // The deck only shows where the ray crosses it BEFORE any ground:
            // from under the deck, rays to the ground never reach it.
            let before_ground = coverage < 0.5 || t_c < hit_t;
            if (t_c > 0.0 && before_ground) {
                let shell_normal = normalize(ray * t_c - centre);
                let fp_c = max(length(fwidth(shell_normal)), 1e-7);
                let dens = cloud_density(shell_normal, 0.35 / (fp_c * CLOUD_DETAIL_FREQ) * exp2(detail_k - 1.0));
                let ndl = dot(shell_normal, sun);
                let lit = max(ndl, 0.0);
                let sun_deck = sun_trans(ray * t_c - centre, radius, beta_r, beta_m);
                let deck_day = smoothstep(-0.15, 0.2, ndl);
                cloud_rgb = planet.cloud_look.rgb * (lit * 1.3 * sun_deck + tint * 0.18 * deck_day + 0.008);

                // From outside, the deck has its own limb and needs the same
                // analytic edge as the body. From inside it has no edge at all
                // — every direction crosses it.
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
    // One march from where the ray enters the shell to where it leaves it
    // or meets the ground (blended across the limb's antialiased pixel),
    // split at the cloud deck so the air in front of the deck veils it and
    // the air behind sits under it.
    let t_end = select(t_exit, mix(t_exit, hit_t, coverage), disc_body > 0.0);
    let t_low = along;
    var front: Air;
    var behind = Air(vec3<f32>(0.0), vec3<f32>(1.0));
    if (cloud_a > 0.001 && t_c > t_enter && t_c < t_end) {
        front = march(t_enter, t_c, t_low, ray, centre, radius, beta_r, beta_m);
        behind = march(t_c, t_end, t_low, ray, centre, radius, beta_r, beta_m);
    } else {
        front = march(t_enter, t_end, t_low, ray, centre, radius, beta_r, beta_m);
    }

    // ---- compose: ground, far air, deck, near air, all in radiance -----
    var light = surface * coverage;
    light = behind.insc + behind.trans * light;
    light = cloud_rgb * cloud_a + light * (1.0 - cloud_a);
    light = front.insc + front.trans * light;

    // What of the stars gets through: the geometry's transmittance, and
    // then a bright sky hides what is behind it — occlusion follows the
    // air's own luminance, so a noon sky erases stars while a twilight of
    // the same depth leaves them.
    let lum_w = vec3<f32>(0.2126, 0.7152, 0.0722);
    let through = front.trans * (1.0 - cloud_a) * behind.trans * (1.0 - coverage);
    let alpha_geom = 1.0 - clamp(dot(through, lum_w), 0.0, 1.0);
    let air_lum = dot(front.insc + front.trans * (1.0 - cloud_a) * behind.insc, lum_w);
    let over = 1.0 - exp(-air_lum * 13.0);
    let alpha = 1.0 - (1.0 - alpha_geom) * (1.0 - over);

    if (alpha < 0.002) {
        discard;
    }
    var rgb = tonemap(light, exposure);
    rgb += vec3<f32>(dither_px(in.pos.xy)) * alpha;
    return vec4<f32>(rgb, alpha);
}
