// gyro.wgsl — the attitude ball (pass: gyro, an instrument)
//
// Lane: A (vertex+fragment only). Cost class: trivial — one small quad on
// the canopy, a handful of SDFs.
//
// Attitude relative to GRAVITY, not to the orbit or the camera: "up" is the
// line from the planet's centre through the ship, and this instrument says
// how the hull sits against it. The ball carries the horizon and a pitch
// ladder, rolled by the ship's bank and slid by its pitch; the fixed wings
// in the middle are the ship; the pointer on the rim is the bank against
// the roll scale. A small amber tick on the rim is the prograde azimuth —
// which way the ship is actually going across the level plane, relative to
// the nose: drift.
//
// Same glass, same warp, same hologram as the rest of the cluster: the
// canopy() projection, the sway parallax, the scanlines, the rim falloff.

struct Gyro {
    // x: pitch, rad (nose above the level plane). y: roll, rad (right wing
    // down positive). z: visibility 0..1. w: aspect.
    a: vec4<f32>,
    // x: drift, rad (prograde to the right of the nose positive). y: target
    // height px (scanline frequency). zw: canopy anchor, NDC.
    b: vec4<f32>,
    // xy: hologram sway. z: time s. w: unused.
    c: vec4<f32>,
    d: vec4<f32>,
    // DIAL placement, as the gauges: right, up, fwd (w tan half fov), centre
    // (w metres per unit; 0 = on the glass).
    p0: vec4<f32>,
    p1: vec4<f32>,
    p2: vec4<f32>,
    p3: vec4<f32>,
}

@group(0) @binding(0) var<uniform> gyro: Gyro;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
}

const RADIUS: f32 = 0.155;
const QUAD_HALF: f32 = 0.30;
// Pitch scale: this many radians of pitch slide the horizon one radius.
const PITCH_PER_RADIUS: f32 = 0.7854; // 45 degrees

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let aspect = gyro.a.w;
    let centre = canopy(gyro.b.zw, aspect);
    let half = select(QUAD_HALF, QUAD_HALF * 1.8, gyro.p3.w > 0.0);
    let xy = canopy_inverse(centre + corners[vi] * half, aspect);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

fn seg(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h);
}

fn line(d: f32, half_w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(0.0, aa, d - half_w);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = gyro.a.z;
    if (vis < 0.01) {
        discard;
    }
    let aspect = gyro.a.w;
    let in_dash = gyro.p3.w > 0.0;
    var p = canopy(in.ndc, aspect) - canopy(gyro.b.zw, aspect);
    if (in_dash) {
        let duv = dial_plane_uv(in.ndc, aspect, gyro.p0, gyro.p1, gyro.p2, gyro.p3, DIAL_DASH_N);
        if (duv.z < 0.5) {
            discard;
        }
        p = duv.xy;
    }
    if (length(p) > RADIUS * 1.35) {
        discard;
    }
    let sway = select(gyro.c.xy, vec2<f32>(0.0), in_dash);
    let p_face = p - sway * 0.18;
    let p_mid = p - sway * 0.55;
    let p_near = p - sway * 1.0;
    let aa = max(fwidth(p.x), 1e-5) * 0.9;

    let pitch = gyro.a.x;
    let roll = gyro.a.y;
    let drift = gyro.b.x;

    var glow = 0.0;
    var hot = 0.0;
    var warn = 0.0;
    var sky = 0.0;
    var ground = 0.0;

    // ---- the ball: rolled by -roll, slid by pitch ---------------------
    // Right wing down (roll > 0): the real horizon appears to rotate
    // counter-clockwise — the left side rises. The horizon is the line
    // q.y = y0 in the ball's own frame, so to turn that line CCW on the
    // glass the frame itself rotates CW: q = R(+roll)·p. (The first cut
    // used R(−roll), and the ball rolled against the world.)
    let cr = cos(roll);
    let sr = sin(roll);
    let q = vec2<f32>(cr * p_mid.x + sr * p_mid.y, -sr * p_mid.x + cr * p_mid.y);
    let r_ball = length(p_mid);
    let in_ball = r_ball < RADIUS - 0.006;
    let ball_edge = 1.0 - smoothstep(RADIUS - 0.010, RADIUS - 0.004, r_ball);
    if (in_ball) {
        // Vertical position on the ball of a given pitch line.
        let y0 = -pitch / PITCH_PER_RADIUS * RADIUS;
        let above = q.y - y0;
        sky = smoothstep(-0.004, 0.004, above) * ball_edge;
        ground = (1.0 - smoothstep(-0.004, 0.004, above)) * ball_edge;
        // Horizon line.
        glow += 0.9 * line(abs(above), 0.0012, aa * 1.6) * ball_edge;
        // Pitch ladder: every 10 degrees, a bar whose half-width shrinks
        // with angle; negative pitch bars are broken in the middle.
        for (var i = 1; i <= 8; i += 1) {
            let ang = f32(i) * 0.17453;
            let w = 0.050 - 0.004 * f32(i);
            let ya = y0 + ang / PITCH_PER_RADIUS * RADIUS;
            let yb = y0 - ang / PITCH_PER_RADIUS * RADIUS;
            let da = seg(q, vec2<f32>(-w, ya), vec2<f32>(w, ya));
            let db = min(
                seg(q, vec2<f32>(-w, yb), vec2<f32>(-0.012, yb)),
                seg(q, vec2<f32>(0.012, yb), vec2<f32>(w, yb)),
            );
            let major = select(0.45, 0.8, (i % 3) == 0);
            glow += major * line(da, 0.0008, aa * 1.6) * ball_edge;
            glow += major * line(db, 0.0008, aa * 1.6) * ball_edge;
        }
    }

    // ---- the rim: roll scale and bank pointer ---------------------------
    let r = length(p_face);
    let theta = atan2(p_face.x, p_face.y); // 0 at top, + to the right
    let ring = abs(r - RADIUS);
    glow += 0.9 * line(ring, 0.0016, aa * 1.6);
    glow += 0.15 * (1.0 - smoothstep(0.0, 0.012, ring));
    // Ticks at 0, ±10, ±20, ±30, ±45, ±60, ±90 degrees from the top.
    let ticks = array<f32, 7>(0.0, 0.17453, 0.34907, 0.5236, 0.7854, 1.0472, 1.5708);
    for (var i = 0; i < 7; i += 1) {
        let t = ticks[i];
        let len = select(0.012, 0.022, i == 0 || i == 3 || i == 5 || i == 6);
        for (var sgn = -1.0; sgn <= 1.0; sgn += 2.0) {
            let ang = t * sgn;
            let dir = vec2<f32>(sin(ang), cos(ang));
            let d = seg(p_face, dir * (RADIUS + 0.004), dir * (RADIUS + 0.004 + len));
            glow += 0.7 * line(d, 0.0009, aa * 1.6);
            if (i == 0) {
                break;
            }
        }
    }
    // Bank pointer: a small triangle inside the rim at the roll angle.
    {
        let dir = vec2<f32>(sin(roll), cos(roll));
        let tip = dir * (RADIUS - 0.006);
        let base = dir * (RADIUS - 0.026);
        let side = vec2<f32>(dir.y, -dir.x) * 0.010;
        let d = min(seg(p_mid, tip, base + side), min(seg(p_mid, tip, base - side), seg(p_mid, base - side, base + side)));
        hot += line(d, 0.0006, aa * 1.6);
        glow += 0.4 * (1.0 - smoothstep(0.0, 0.010, d));
    }
    // Drift: prograde azimuth relative to the nose, an amber tick on the
    // rim — level with the horizon, the ship goes where this points.
    {
        let ang = clamp(drift, -1.4, 1.4);
        let dir = vec2<f32>(sin(ang), cos(ang));
        let d = seg(p_mid, dir * (RADIUS - 0.030), dir * (RADIUS - 0.010));
        warn += 0.9 * line(d, 0.0014, aa * 1.6);
    }

    // ---- the ship: fixed wings and a dot, nearest layer ------------------
    {
        let wl = seg(p_near, vec2<f32>(-0.075, 0.0), vec2<f32>(-0.018, 0.0));
        let wr = seg(p_near, vec2<f32>(0.018, 0.0), vec2<f32>(0.075, 0.0));
        let tl = seg(p_near, vec2<f32>(-0.018, 0.0), vec2<f32>(-0.018, -0.012));
        let tr = seg(p_near, vec2<f32>(0.018, 0.0), vec2<f32>(0.018, -0.012));
        let w = min(min(wl, wr), min(tl, tr));
        hot += line(w, 0.0012, aa * 1.6);
        glow += 0.35 * (1.0 - smoothstep(0.0, 0.010, w));
        let dot_d = length(p_near);
        hot += 1.0 - smoothstep(0.0030, 0.0030 + aa * 1.6, dot_d);
    }

    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    let scan = select(0.90 + 0.10 * sin(in.ndc.y * gyro.b.y * 1.7), 1.0, in_dash);
    let glass = select(canopy_glass(in.ndc, aspect), 1.0, in_dash);
    var colour = cyan * glow
        + vec3<f32>(1.0) * hot * 0.9
        + amber * warn
        + cyan * sky * 0.10
        + amber * ground * 0.06;
    colour *= scan * glass * vis;
    return vec4<f32>(colour, 1.0);
}
