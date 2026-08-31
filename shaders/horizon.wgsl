// horizon.wgsl — the level line and pitch ladder, head-up (pass: horizon)
//
// Lane: A (vertex+fragment only). Cost class: cheap — a fullscreen quad,
// one asin and a few bands per pixel, and most pixels discard at once.
//
// The GRAVITY horizon, drawn where it really is: the plane through the eye
// perpendicular to the local up, projected through the camera. From orbit
// the planet's limb sits well below it; this line is where level flight
// points, and the ladder above and below it is pitch in tens of degrees,
// hung on the nose's heading so it moves with the ship's bank. True
// projection, not the canopy: it is a reference in the world, like the
// path, not a dial on the glass.

struct Horizon {
    // xyz: gravity's "up" in CAMERA space (x right, y up, z forward).
    // w: visibility 0..1.
    a: vec4<f32>,
    // x: tan(fov_y/2), y: aspect, z: screen height px, w: time s
    b: vec4<f32>,
    // x: 1 to draw the pitch ladder (the level line stays either way).
    // yzw: the ship's nose in camera space — the ladder's centre.
    c: vec4<f32>,
    d: vec4<f32>,
}

@group(0) @binding(0) var<uniform> hz: Horizon;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(corners[vi], 0.0, 1.0);
    out.ndc = corners[vi];
    return out;
}

const DEG: f32 = 0.017453292;
// Half-width of the ladder bars in azimuth, and of the gap at their middle.
const LADDER_HALF: f32 = 9.0 * DEG;
const LADDER_GAP: f32 = 2.4 * DEG;
// The numerals at the bar ends: half-height, half-width, cell pitch, and
// how far outboard of the bar's end they sit.
const NUM_H: f32 = 0.70 * DEG;
const NUM_W: f32 = 0.40 * DEG;
const NUM_PITCH: f32 = 1.15 * DEG;
const NUM_OUT: f32 = 2.2 * DEG;

// A crisp line: solid within half_w of the distance, a pixel of ramp.
fn hline(d: f32, half_w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, d - half_w);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = hz.a.w;
    if (vis < 0.01) {
        discard;
    }
    let up = normalize(hz.a.xyz);
    let tan_half = hz.b.x;
    let aspect = hz.b.y;
    let ray = normalize(vec3<f32>(in.ndc.x * tan_half * aspect, in.ndc.y * tan_half, 1.0));

    // Elevation above the level plane, and azimuth from the nose's heading
    // — the NOSE, handed over in camera space: with the head turned the
    // ladder stays on the boresight, where the ship is pointed.
    let elev = asin(clamp(dot(ray, up), -1.0, 1.0));
    var fwd = hz.c.yzw;
    if (dot(fwd, fwd) < 1e-6) {
        fwd = vec3<f32>(0.0, 0.0, 1.0);
    }
    fwd = normalize(fwd);
    var h_fwd = fwd - up * dot(fwd, up);
    if (dot(h_fwd, h_fwd) < 1e-6) {
        // Looking straight up or down: no heading; hang the ladder on the
        // screen's own up instead.
        h_fwd = vec3<f32>(0.0, 1.0, 0.0) - up * up.y;
    }
    h_fwd = normalize(h_fwd);
    // Camera space is x right, y up, z forward: that is a left-handed
    // triple, so cross(forward, up) points LEFT; negated, azimuth grows
    // to the right and the numerals read the right way round.
    let h_right = -cross(h_fwd, up);
    let az = atan2(dot(ray, h_right), dot(ray, h_fwd));
    // Azimuth as an angle on the sphere at this elevation — the ladder
    // keeps its width up toward the zenith instead of splaying.
    let azs = az * cos(elev);

    // A pixel, in radians of elevation: every stroke is sized in pixels.
    let px = max(fwidth(elev), 1e-5);
    let aa = px * 1.2;
    var glow = 0.0;
    var halo = 0.0;

    // The horizon line: everywhere, crisp, with a soft halo.
    glow += hline(abs(elev), px * 1.0, aa);
    halo += 0.12 * (1.0 - smoothstep(0.0, 10.0 * px, abs(elev)));

    // The ladder: bars every 10 degrees within the azimuth window, broken
    // at the middle, the 30° and 60° bars heavier; the window shrinks with
    // angle so the ladder tapers. End ticks point at the horizon; the
    // pitch in degrees is written outboard of each end. Below the horizon
    // the bars are dashed.
    let deg10 = 10.0 * DEG;
    let step_i = round(elev / deg10);
    let abs_i = abs(step_i);
    if (hz.c.x > 0.5 && abs_i >= 1.0 && abs_i <= 8.0 && abs(azs) < LADDER_HALF + NUM_OUT + 2.0 * NUM_PITCH) {
        let off = elev - step_i * deg10;
        let half_w = LADDER_HALF - abs_i * 0.4 * DEG;
        let major = (i32(abs_i) % 3) == 0;
        let bar_hw = px * select(0.85, 1.15, major);
        let ax = abs(azs);
        let in_bar = smoothstep(LADDER_GAP - px, LADDER_GAP + px, ax)
            * (1.0 - smoothstep(half_w - px, half_w + px, ax));
        let ph = fract(azs / (2.0 * DEG));
        let pxd = px / (2.0 * DEG);
        let dash = select(1.0, smoothstep(0.5 - pxd, 0.5 + pxd, ph), step_i < 0.0);
        glow += hline(abs(off), bar_hw, aa) * in_bar * dash * select(0.85, 1.0, major);
        // End ticks toward the horizon.
        let toward = select(-off, off, step_i < 0.0);
        let tick_span = smoothstep(-px, px, toward) * (1.0 - smoothstep(1.4 * DEG - px, 1.4 * DEG + px, toward));
        glow += hline(abs(ax - half_w), bar_hw, aa) * tick_span;
        // Numerals: the pitch in tens, "10" .. "80", two cells at each
        // end, read the right way round on both sides.
        let cx = half_w + NUM_OUT;
        let qx = select(azs + cx, azs - cx, azs > 0.0);
        let q = vec2<f32>(qx, off);
        let tens = u32(abs_i);
        let d = min(
            digit_dist(q - vec2<f32>(-0.5 * NUM_PITCH, 0.0), digit_mask(tens), NUM_W, NUM_H),
            digit_dist(q - vec2<f32>(0.5 * NUM_PITCH, 0.0), digit_mask(0u), NUM_W, NUM_H),
        );
        glow += 0.9 * hline(d, px * 0.75, aa);
    }

    if (glow + halo < 0.003) {
        discard;
    }
    // Real radiance in the lines — a hair over one — for the bloom.
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let glass = canopy_glass(in.ndc, aspect);
    return vec4<f32>(cyan * (glow * 1.3 + halo) * glass * vis, 1.0);
}
