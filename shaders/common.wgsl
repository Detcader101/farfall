// common.wgsl — shader prelude, prepended to every pass (see render/src/shaders.rs).
//
// WGSL has no #include, so composition happens in Rust. Everything here is
// shared by two or more passes; duplicating a noise function across passes is
// how two parts of the same world quietly stop agreeing with each other.

// ---------------------------------------------------------------- hashing

// pcg2d-style integer hash -> 4 independent floats in [0,1).
fn hash4(cell: vec2<i32>) -> vec4<f32> {
    var v = vec2<u32>(bitcast<u32>(cell.x), bitcast<u32>(cell.y));
    v = v * 1664525u + 1013904223u;
    v.x += v.y * 1664525u;
    v.y += v.x * 1013904223u;
    v = v ^ (v >> vec2<u32>(16u));
    v.x += v.y * 1664525u;
    v.y += v.x * 1013904223u;
    v = v ^ (v >> vec2<u32>(16u));
    return vec4<f32>(
        f32(v.x & 0xffffu) / 65536.0,
        f32(v.x >> 16u) / 65536.0,
        f32(v.y & 0xffffu) / 65536.0,
        f32(v.y >> 16u) / 65536.0,
    );
}

// Three decorrelated odd multipliers and a two-round finaliser.
//
// The obvious choice of 2147483647 (2^31 - 1) for one of the multipliers is a
// trap: z * (2^31 - 1) == (z << 31) - z, so the z term degenerates to -z plus a
// parity bit, and one round of mixing cannot hide it. Measured lattice
// correlation was -0.27 at a z step of 2 — an anisotropic stripe running
// through every noise field built on this hash.
fn hash31(p: vec3<i32>) -> f32 {
    var h = u32(p.x) * 0x8da6b343u + u32(p.y) * 0xd8163841u + u32(p.z) * 0xcb1ab31fu;
    h = (h ^ (h >> 15u)) * 0x2c1b3c6du;
    h = (h ^ (h >> 12u)) * 0x297a2d39u;
    return f32(h ^ (h >> 15u)) / 4294967296.0;
}

// ------------------------------------------------------------------ noise

fn vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(
        mix(
            mix(hash31(i + vec3<i32>(0, 0, 0)), hash31(i + vec3<i32>(1, 0, 0)), u.x),
            mix(hash31(i + vec3<i32>(0, 1, 0)), hash31(i + vec3<i32>(1, 1, 0)), u.x),
            u.y,
        ),
        mix(
            mix(hash31(i + vec3<i32>(0, 0, 1)), hash31(i + vec3<i32>(1, 0, 1)), u.x),
            mix(hash31(i + vec3<i32>(0, 1, 1)), hash31(i + vec3<i32>(1, 1, 1)), u.x),
            u.y,
        ),
        u.z,
    );
}

// Normalised to [0,1], like fbm5. The amplitudes sum to 0.875, so leaving the
// division out shifts the mean to 0.4375 — and every threshold written against
// it as though 0.5 were the midpoint then sits in the wrong place, silently.
fn fbm3(p: vec3<f32>) -> f32 {
    let sum = 0.5 * vnoise(p) + 0.25 * vnoise(p * 2.03) + 0.125 * vnoise(p * 4.07);
    return sum / 0.875;
}

// Detail-limited fbm: octaves are added until they reach the resolution limit
// `max_freq`, then stop.
//
// This is the whole "spend samples where they read" idea in one function. A
// fixed octave count is wrong in both directions at once: from orbit the fine
// octaves land far below a pixel and cost real time to contribute nothing but
// shimmer, while up close the field runs out of octaves entirely and the
// surface goes smooth. Driving the count from the pixel's footprint gives
// cheap distant worlds and detailed near ones from the same call.
//
// The last octave fades in rather than appearing, so crossing a threshold is
// invisible; and the sum is normalised by the weight actually accumulated, so
// the field keeps a mean of 0.5 at any octave count — otherwise every coastline
// would creep as the ship approached it.
fn fbm_lod(p: vec3<f32>, max_freq: f32, max_octaves: i32) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0; i < max_octaves; i += 1) {
        let weight = clamp(max_freq / freq - 1.0, 0.0, 1.0);
        if (weight <= 0.0) {
            break;
        }
        sum += amp * weight * vnoise(p * freq);
        norm += amp * weight;
        amp *= 0.5;
        freq *= 2.03;
    }
    return sum / max(norm, 1e-6);
}

// Normalised to [0,1]: five octaves, for terrain-scale structure.
fn fbm5(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amp = 0.5;
    var freq = 1.0;
    var norm = 0.0;
    for (var i = 0; i < 5; i += 1) {
        sum += amp * vnoise(p * freq);
        norm += amp;
        amp *= 0.5;
        freq *= 2.03;
    }
    return sum / norm;
}

// ------------------------------------------------------------------ output

// Exposure + soft shoulder. No history buffers, no temporal accumulation (P1).
fn tonemap(col: vec3<f32>, exposure: f32) -> vec3<f32> {
    return vec3<f32>(1.0) - exp(-max(col, vec3<f32>(0.0)) * exposure);
}

// +-0.5/255 of hash noise: kills 8-bit banding without temporal shimmer.
fn dither_px(px: vec2<f32>) -> f32 {
    return (hash31(vec3<i32>(vec3<f32>(px, 17.0))) - 0.5) / 255.0;
}

// ------------------------------------------------------------------ camera

// View ray for a pixel, from the camera basis. Camera-relative rendering means
// the camera is always at the origin, so this is the whole camera model.
fn view_ray(ndc: vec2<f32>, right: vec3<f32>, up: vec3<f32>, forward: vec3<f32>,
            tan_half_fov: f32, aspect: f32) -> vec3<f32> {
    return normalize(
        forward + right * (ndc.x * tan_half_fov * aspect) + up * (ndc.y * tan_half_fov)
    );
}

// Fullscreen triangle vertex: (-1,-1), (3,-1), (-1,3).
fn fullscreen_ndc(vertex_index: u32) -> vec2<f32> {
    return vec2<f32>(f32((vertex_index << 1u) & 2u), f32(vertex_index & 2u)) * 2.0 - 1.0;
}

// ------------------------------------------------------------------ canopy

// Canopy radius, in aspect-corrected screen units. Smaller bends harder.
const CANOPY_R: f32 = 1.55;

// The canopy projection: the HUD is not painted on the screen, it is painted
// on the inside of a spherical shell centred on the pilot's eye, and the
// screen shows that shell in perspective. Content at shell angle t lands on
// screen at tan(t), so this inverse — screen pixel back to shell angle — is
// atan: near the centre it is the identity, and toward the rim each shell
// unit covers ever more screen, so instruments there stretch and bow the way
// the inside of a dome does. Every HUD element — gauges, text, whatever comes
// next — passes through this one function, so the whole cockpit shares a
// single piece of glass. It lives in the prelude precisely so no pass can
// grow its own subtly different curvature.
fn canopy(ndc: vec2<f32>, aspect: f32) -> vec2<f32> {
    let v = vec2<f32>(ndc.x * aspect, ndc.y);
    let r = length(v);
    if (r < 1e-4) {
        return v;
    }
    let x = r / CANOPY_R;
    return v * (atan(x) / x);
}

// The inverse: a point on the shell back to the screen pixel that shows it.
// Lets a pass that knows where on the glass it lives draw only that patch of
// screen instead of a fullscreen triangle that discards 97% of itself —
// the gauges were costing a millisecond a frame to decide not to draw.
fn canopy_inverse(c: vec2<f32>, aspect: f32) -> vec2<f32> {
    let r = length(c);
    if (r < 1e-4) {
        return vec2<f32>(c.x / aspect, c.y);
    }
    let x = tan(min(r / CANOPY_R, 1.5));
    let v = c * (x * CANOPY_R / r);
    return vec2<f32>(v.x / aspect, v.y);
}

// The projection dims toward the rim of the glass: light hitting the canopy
// obliquely reads fainter, which sells the shell more than the distortion
// does. Shared for the same reason canopy() is.
fn canopy_glass(ndc: vec2<f32>, aspect: f32) -> f32 {
    let rim = length(vec2<f32>(ndc.x * aspect, ndc.y));
    return 1.0 - 0.38 * smoothstep(0.75, 1.45, rim);
}

// ------------------------------------------------------------- octahedral

// Octahedral map of the unit sphere onto [-1,1]². The hemisphere with z > 0
// is the inner diamond, where a clamped texture keeps all its resolution.
// Used by the starfield for its sky cells, and by the thermal sim to store a
// field over the hull (ship space: x right, y up, z forward — so the forward
// hemisphere, where the pilot looks, gets the detail) with the plasma pass
// reading it back. One copy: a private variant in either pass would let the
// heat drift away from where it is drawn.
fn oct_encode(d: vec3<f32>) -> vec2<f32> {
    let n = d / (abs(d.x) + abs(d.y) + abs(d.z));
    let sign_n = select(vec2<f32>(-1.0), vec2<f32>(1.0), n.xy >= vec2<f32>(0.0));
    return select(n.xy, (1.0 - abs(n.yx)) * sign_n, n.z < 0.0);
}

fn oct_decode(f: vec2<f32>) -> vec3<f32> {
    var n = vec3<f32>(f.x, f.y, 1.0 - abs(f.x) - abs(f.y));
    let t = clamp(-n.z, 0.0, 1.0);
    n.x += select(t, -t, n.x >= 0.0);
    n.y += select(t, -t, n.y >= 0.0);
    return normalize(n);
}

// ------------------------------------------------------------ quaternion

// Rotate a vector by a unit quaternion (x, y, z, w).
fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

// ------------------------------------------------------------- blackbody

// Colour of a hot body at temperature `kk` kilokelvin, normalised so the
// brightest channel is ~1. A compact fit of the Planckian locus: 1 kK is a
// deep red barely there, 2 kK orange, 3.5 kK yellow-white, 6 kK white, and
// beyond that it cools to blue-white. Brightness is the caller's business
// (Stefan-Boltzmann: T^4), this is chromaticity only.
fn blackbody(kk: f32) -> vec3<f32> {
    let t = clamp(kk, 0.5, 12.0);
    let r = clamp(1.0 - 0.10 * max(t - 6.5, 0.0), 0.55, 1.0);
    let g = clamp(0.46 * log(t) + 0.12, 0.0, 1.0);
    let b = clamp(0.55 * log(max(t - 1.6, 0.01)) + 0.02, 0.0, 1.0);
    return vec3<f32>(r, g, b);
}

// ----------------------------------------------------------------- the ship
//
// The ship's own shape, as signed distance in its frame: x right, y up, -z
// the nose, metres, the pilot's head at the origin. One definition, used by
// the cockpit (the nose and wings seen through the glass, the hull the
// pilot sits inside) and by the map (the same ship, small). A fighter: a
// long fuselage, swept delta wings, twin engines, a single canted-free fin.

fn sd_round_box(p: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0) - r;
}

fn sd_ellipsoid_c(p: vec3<f32>, c: vec3<f32>, r: vec3<f32>) -> f32 {
    let q = p - c;
    let k0 = length(q / r);
    let k1 = length(q / (r * r));
    return k0 * (k0 - 1.0) / max(k1, 1e-6);
}

fn sd_capsule_ab(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

// The exterior only: fuselage, wings, engines, fin. Solid.
fn sd_fighter_exterior(q: vec3<f32>) -> f32 {
    // Fuselage: a long ellipsoid, fattest under the pilot, drawn out to a
    // needle at the nose and a flat tail.
    // Fuselage: the body from the cockpit back, and a slimmer nose cone
    // ahead of it that drops away below the sill — the pilot sits high,
    // as in a fighter, and looks over the nose.
    let fus = sd_ellipsoid_c(q, vec3<f32>(0.0, -1.05, 2.2), vec3<f32>(1.0, 0.85, 5.6));
    let nose = sd_ellipsoid_c(q, vec3<f32>(0.0, -1.3, -3.4), vec3<f32>(0.72, 0.5, 3.3));
    var d = min(fus, nose);
    // The wings, engines and fin all live aft and outboard: one box bounds
    // them, and a point well clear of it needs only the box's distance (a
    // safe lower bound for the march) — which is most of the cabin.
    let aft = sd_round_box(q - vec3<f32>(0.0, -0.5, 4.4), vec3<f32>(6.0, 1.4, 3.6), 0.0);
    if (aft > 0.3) {
        return min(d, aft);
    }
    // Wings: swept delta, thin, the chord shrinking toward the tips.
    let wx = abs(q.x);
    let wq = vec3<f32>(wx - 3.2, q.y + 0.92, q.z - 2.3 - 0.62 * wx);
    let wing = sd_round_box(wq, vec3<f32>(2.6, 0.035, 1.15 - 0.12 * wx), 0.03);
    d = min(d, wing);
    // Engines: two nacelles under the tail, and the fin between them.
    let eq = vec3<f32>(abs(q.x) - 0.62, q.y + 0.85, q.z);
    let eng = sd_capsule_ab(eq, vec3<f32>(0.0, 0.0, 4.2), vec3<f32>(0.0, 0.0, 7.3), 0.44);
    d = min(d, eng);
    // The nozzle: a ring at each nacelle's tail, a flared lip the plume
    // comes out of.
    let nz = vec2<f32>(length(eq.xy) - 0.36, eq.z - 7.55);
    let nozzle = length(nz) - 0.07;
    d = min(d, nozzle);
    let fq = vec3<f32>(q.x, q.y - 0.35, q.z - 6.0 - 0.55 * max(q.y - 0.35, 0.0));
    let fin = sd_round_box(fq, vec3<f32>(0.035, 1.0, 0.75), 0.02);
    return min(d, fin);
}

// The ship with the cockpit carved out: the cavity the pilot sits in, and
// the glass cut above it. What is left has thickness — the wall between
// the cavity and space.
fn sd_fighter_hull(q: vec3<f32>) -> f32 {
    let outer = sd_fighter_exterior(q);
    let cavity = sd_ellipsoid_c(q, vec3<f32>(0.0, -0.25, -0.1), vec3<f32>(0.92, 1.05, 1.75));
    // The glass cut opens the top; it stays clear of the dash below the
    // sill, so nothing of it haunts the march down there.
    let glass = sd_round_box(q - vec3<f32>(0.0, 0.7, -0.45), vec3<f32>(0.80, 0.9, 1.25), 0.15);
    // max(a, -b) is the usual carve. It is not a true distance for a point
    // outside the hull but inside the cut (where the pilot sits): it hands
    // back the distance to the cut's wall, which can overshoot whatever
    // stands inside the cut. The cockpit's march bisects on overshoot for
    // exactly that reason.
    return max(outer, -min(cavity, glass));
}

// --------------------------------------------------------------- dial plane
//
// A DIAL-style instrument lies flat in the dash, a real face under glass:
// the pixel's ray (in the ship's frame, turned by the pilot's head) is
// intersected with the dial's plane and the hit is mapped into the dial's
// own 2D coordinates, scaled so the dial's drawing radius is the same
// number it is on the glass. Returns (u, v, hit): hit < 0.5 means the ray
// never meets the plane from the front.
//
// `place0..3`: right, up, fwd of the head in ship frame (w of fwd: tan of
// half the fov), and the dial's centre (w: metres per drawing unit).
// The dial's basis: U along the ship's x, V = N x U on the dash plane.
fn dial_plane_uv(ndc: vec2<f32>, aspect: f32, right: vec4<f32>, up: vec4<f32>,
                 fwd: vec4<f32>, centre: vec4<f32>, normal: vec3<f32>) -> vec3<f32> {
    let tan_half = fwd.w;
    let ray = normalize(fwd.xyz + right.xyz * (ndc.x * tan_half * aspect) + up.xyz * (ndc.y * tan_half));
    let denom = dot(ray, normal);
    if (denom > -1e-4) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    let t = dot(centre.xyz, normal) / denom;
    if (t <= 0.0) {
        return vec3<f32>(0.0, 0.0, 0.0);
    }
    let hit = ray * t - centre.xyz;
    let u_axis = vec3<f32>(1.0, 0.0, 0.0);
    let v_axis = normalize(cross(normal, u_axis));
    let scale = max(centre.w, 1e-4);
    return vec3<f32>(dot(hit, u_axis) / scale, dot(hit, v_axis) / scale, 1.0);
}

// The dash's plane, shared with the cockpit: a point on it and its normal.
const DIAL_DASH_C: vec3<f32> = vec3<f32>(0.0, -0.50, -1.05);
const DIAL_DASH_N: vec3<f32> = vec3<f32>(0.0, 0.9563, 0.2924);
