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
const LADDER_HALF: f32 = 7.0 * DEG;
const LADDER_GAP: f32 = 1.6 * DEG;

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
    let h_right = cross(h_fwd, up);
    let az = atan2(dot(ray, h_right), dot(ray, h_fwd));

    let aa_e = max(fwidth(elev), 1e-5) * 1.2;
    var glow = 0.0;

    // The horizon line: everywhere, thin, with a soft halo.
    glow += 0.9 * (1.0 - smoothstep(0.0, aa_e, abs(elev) - aa_e * 0.6));
    glow += 0.12 * (1.0 - smoothstep(0.0, 8.0 * aa_e, abs(elev)));

    // The ladder: bars every 10 degrees within the azimuth window, broken
    // at the middle; the window shrinks with angle so the ladder tapers.
    let deg10 = 10.0 * DEG;
    let step_i = round(elev / deg10);
    if (hz.c.x > 0.5 && abs(step_i) >= 1.0 && abs(step_i) <= 8.0 && abs(az) < LADDER_HALF) {
        let off = abs(elev - step_i * deg10);
        let half_w = LADDER_HALF - abs(step_i) * 0.5 * DEG;
        let in_bar = abs(az) < half_w && abs(az) > LADDER_GAP;
        if (in_bar) {
            // Below the horizon the bars are dashed.
            let dashed = step_i < 0.0;
            let dash = select(1.0, step(0.5, fract(az / (2.0 * DEG))), dashed);
            let major = select(0.55, 0.9, (i32(abs(step_i)) % 3) == 0);
            glow += major * dash * (1.0 - smoothstep(0.0, aa_e, off - aa_e * 0.5));
        }
        // End ticks, pointing toward the horizon.
        let end_d = abs(abs(az) - half_w);
        let toward = select(elev - step_i * deg10, step_i * deg10 - elev, step_i < 0.0);
        if (end_d < aa_e * 1.5 && toward < 0.0 && toward > -1.2 * DEG) {
            glow += 0.8;
        }
    }

    if (glow < 0.003) {
        discard;
    }
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let glass = canopy_glass(in.ndc, aspect);
    return vec4<f32>(cyan * glow * glass * vis * 0.85, 1.0);
}
