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
    // xy: hologram sway. zw: the world's east in the ship's frame,
    // octahedral (the geometric ball).
    c: vec4<f32>,
    // x: 0 hologram, 1 JET (a shaded disc), 2 the geometric ball — a
    // sphere in the dash (p3: centre, w radius in metres) painted with
    // the world's frame; yzw: the world's up in the ship's frame.
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
    let size = select(max(gyro.p0.w, 0.25), 1.8, gyro.p3.w > 0.0);
    let half = QUAD_HALF * size;
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

// The geometric ball: the pixel's ray from the head meets the sphere in
// the dash; the hit's normal, in the ship's frame, is read against the
// world's up and east — the ball is a globe that stays with the world as
// the ship turns about it. Sky above the horizon, earth below, the
// horizon line, pitch lines every 10°, meridians every 30°, and the
// fixed wings at the point nearest the pilot. Lit by a lamp up-left of
// the head, with a rim.
fn ball_3d(ndc: vec2<f32>, aspect: f32, vis: f32) -> vec4<f32> {
    let tan_half = gyro.p2.w;
    let ray = normalize(gyro.p2.xyz + gyro.p0.xyz * (ndc.x * tan_half * aspect) + gyro.p1.xyz * (ndc.y * tan_half));
    let c = gyro.p3.xyz;
    let rad = max(gyro.p3.w, 1e-3);
    let b = dot(ray, c);
    let disc = b * b - (dot(c, c) - rad * rad);
    if (disc < 0.0 || b <= 0.0) {
        discard;
    }
    let t = b - sqrt(disc);
    let hit = ray * t;
    let n = (hit - c) / rad;
    // Anti-aliasing width on the unit sphere: a pixel's footprint.
    let aa = max(fwidth(n.x) + fwidth(n.y) + fwidth(n.z), 1e-4) * 0.7;

    let up = normalize(gyro.d.yzw);
    let east = normalize(oct_decode(gyro.c.zw));
    let north = normalize(cross(up, east));
    // Painted as a real attitude ball is: the world seen THROUGH the
    // ball, so the point facing the pilot wears the antipode — nose down,
    // and the earth rolls up into view.
    let m = -n;
    let lat = asin(clamp(dot(m, up), -1.0, 1.0));
    let lon = atan2(dot(m, east), dot(m, north));

    var glow = 0.0;
    var hot = 0.0;
    // Horizon and pitch lines: every 10°, the horizon itself heavier.
    let horizon = 1.0 - smoothstep(0.0, aa * 1.8, abs(lat) - 0.004);
    let pitch_line = abs(fract(lat / 0.17453 + 0.5) - 0.5) * 0.17453;
    let pl = 1.0 - smoothstep(0.0, aa * 1.6, pitch_line - 0.0025);
    // Meridians every 30°, fading toward the poles where they crowd.
    let mer = abs(fract(lon / 0.5236 + 0.5) - 0.5) * 0.5236 * cos(lat);
    let ml = (1.0 - smoothstep(0.0, aa * 1.6, mer - 0.002)) * (1.0 - smoothstep(1.2, 1.5, abs(lat)));
    glow += 0.55 * pl + 0.3 * ml;
    hot += 0.9 * horizon;

    // Sky and earth, shaded as a sphere by a lamp up-left of the head.
    let lamp = normalize(-0.5 * gyro.p0.xyz + 0.6 * gyro.p1.xyz - 0.7 * gyro.p2.xyz);
    let shade = 0.25 + 0.75 * max(dot(n, lamp), 0.0);
    let spec = pow(max(dot(n, normalize(lamp - ray)), 0.0), 40.0);
    let rim = pow(1.0 - max(dot(n, -ray), 0.0), 3.0);
    let sky = smoothstep(-0.004, 0.004, lat);
    // The WARTHOG ball is a real ADI: brighter blue over brown.
    let warthog = gyro.d.y > 0.5;
    let sky_rgb = select(vec3<f32>(0.16, 0.32, 0.58), vec3<f32>(0.24, 0.46, 0.78), warthog);
    let earth_rgb = select(vec3<f32>(0.36, 0.22, 0.10), vec3<f32>(0.46, 0.26, 0.10), warthog);
    var colour = mix(earth_rgb, sky_rgb, sky) * shade * 1.6
        + vec3<f32>(0.6, 0.6, 0.55) * rim * 0.25
        + vec3<f32>(1.0, 0.97, 0.9) * spec * 0.35;

    // The ship: fixed wings and a dot at the point of the ball nearest
    // the pilot, in that point's tangent frame, the same drawing as the
    // disc's at the ball's scale.
    let f = -normalize(c);
    let e1 = normalize(cross(gyro.p1.xyz, f));
    let e2 = cross(f, e1);
    let uv = vec2<f32>(dot(n, e1), dot(n, e2)) * RADIUS;
    let aa2 = aa * RADIUS;
    {
        let wl = seg(uv, vec2<f32>(-0.075, 0.0), vec2<f32>(-0.018, 0.0));
        let wr = seg(uv, vec2<f32>(0.018, 0.0), vec2<f32>(0.075, 0.0));
        let tl = seg(uv, vec2<f32>(-0.018, 0.0), vec2<f32>(-0.018, -0.012));
        let tr = seg(uv, vec2<f32>(0.018, 0.0), vec2<f32>(0.018, -0.012));
        let w = min(min(wl, wr), min(tl, tr));
        let front = step(0.0, dot(n, f));
        hot += line(w, 0.0014, aa2 * 1.6) * front;
        hot += (1.0 - smoothstep(0.0030, 0.0030 + aa2 * 1.6, length(uv))) * front;
    }
    let ivory = vec3<f32>(0.82, 0.78, 0.62);
    let cream = vec3<f32>(0.96, 0.92, 0.80);
    colour += ivory * glow * 0.55 + cream * hot * 0.9;
    return vec4<f32>(colour * vis, 1.0);
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = gyro.a.z;
    if (vis < 0.01) {
        discard;
    }
    let aspect = gyro.a.w;
    if (gyro.d.x > 1.5) {
        return ball_3d(in.ndc, aspect, vis);
    }
    let in_dash = gyro.p3.w > 0.0;
    var p = (canopy(in.ndc, aspect) - canopy(gyro.b.zw, aspect)) / max(gyro.p0.w, 0.25);
    if (in_dash) {
        let duv = dial_plane_uv(in.ndc, aspect, gyro.p0, gyro.p1, gyro.p2, gyro.p3, DIAL_DASH_N);
        if (duv.z < 0.5) {
            discard;
        }
        p = duv.xy;
    }
    // Tilted (p1.w, radians): in the dash the face plane itself leans,
    // handled above; on the glass a hologram leaned off the pilot's line
    // of sight foreshortens, top edge nearer.
    if (!in_dash) {
        let tilt = gyro.p1.w;
        let lean = max(cos(tilt), 0.35);
        let persp = 1.0 - 0.35 * sin(tilt) * p.y / 0.2;
        p = vec2<f32>(p.x * persp, p.y / lean * persp);
    }
    if (length(p) > RADIUS * 1.35) {
        discard;
    }
    // JET: the ball is a solid sphere — shaded sky and earth halves with
    // a lamp from the upper left and a rim — sitting in its bowl.
    let jet = gyro.d.x > 0.5;
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

    let period = in_dash || jet;
    let cyan = select(vec3<f32>(0.22, 0.85, 1.0), vec3<f32>(0.82, 0.78, 0.62), period);
    let amber = select(vec3<f32>(1.0, 0.62, 0.18), vec3<f32>(0.85, 0.22, 0.10), period);
    let scan = select(0.90 + 0.10 * sin(in.ndc.y * gyro.b.y * 1.7), 1.0, period);
    let glass = select(canopy_glass(in.ndc, aspect), 1.0, period);
    var colour = cyan * glow
        + select(vec3<f32>(1.0), vec3<f32>(0.96, 0.92, 0.80), period) * hot * 0.9
        + amber * warn;
    if (jet) {
        // The solid ball: sky blue above, earth brown below, shaded as a
        // sphere (z from the radius) with a lamp up-left.
        let rb = r_ball / RADIUS;
        let z = sqrt(max(1.0 - rb * rb, 0.0));
        let n3 = vec3<f32>(p_mid / RADIUS, z);
        let lamp = normalize(vec3<f32>(-0.5, 0.6, 0.7));
        let shade = 0.25 + 0.75 * max(dot(n3, lamp), 0.0);
        let rim = pow(1.0 - z, 3.0);
        let sky_rgb = vec3<f32>(0.16, 0.32, 0.58);
        let earth_rgb = vec3<f32>(0.36, 0.22, 0.10);
        colour += (sky_rgb * sky + earth_rgb * ground) * shade * 1.6 + vec3<f32>(0.6, 0.6, 0.55) * rim * 0.25 * ball_edge;
    } else {
        colour += cyan * sky * 0.10 + amber * ground * 0.06;
    }
    colour *= scan * glass * vis;
    return vec4<f32>(colour, 1.0);
}
