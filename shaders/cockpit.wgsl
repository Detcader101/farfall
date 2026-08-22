// cockpit.wgsl — the cabin around the pilot's head (pass: cockpit)
//
// Lane: A. Cost class: cheap — one analytic shell per pixel, no march.
//
// A wireframe cabin in the TRON manner: a canopy dome over the pilot with
// glowing ribs and hoops, a sill where the glass meets the hull, a dash
// below it gridded in light, a rear bulkhead behind. It is drawn in the
// SHIP'S frame — the ray for each pixel is turned by the pilot's head, so
// looking round shows the cabin going round the pilot while the nose stays
// where it is. The dials are drawn after this and sit on the same glass.
//
// Everything is a function of the ray's direction: azimuth from the nose
// and elevation from the eye-line. The hull is where the dome ends; the
// lines are where the ribs are. Nothing here is geometry the CPU knows.

struct Cockpit {
    // xyz: the head's right axis in ship frame. w: line glow 0..2
    right: vec4<f32>,
    // xyz: the head's up axis in ship frame. w: hull opacity 0..1
    up: vec4<f32>,
    // xyz: the head's forward axis in ship frame (-Z is the nose). w: tan(fov/2)
    fwd: vec4<f32>,
    // x: aspect, y: time, z: on 0..1, w: unused
    misc: vec4<f32>,
}

@group(0) @binding(0) var<uniform> ck: Cockpit;

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
// The sill: where the glass meets the hull, elevation below the eye-line,
// ahead and to the sides. The dash sits below it.
const SILL_FRONT: f32 = -22.0 * DEG;
const SILL_SIDE: f32 = -8.0 * DEG;
// Behind the shoulders the cabin closes in: the bulkhead.
const BULKHEAD_AZ: f32 = 118.0 * DEG;
// The dome's top: a spine runs fore and aft up there.
const CROWN: f32 = 62.0 * DEG;

// A line of angular half-width `w` (radians) at distance `d`, anti-aliased
// by the pixel's own angular size `aa`.
fn line(d: f32, w: f32, aa: f32) -> f32 {
    return 1.0 - smoothstep(w, w + aa, abs(d));
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let on = ck.misc.z;
    if (on < 0.01) {
        discard;
    }
    let aspect = ck.misc.x;
    let tan_half = ck.fwd.w;
    let glow_k = ck.right.w;
    let hull_k = ck.up.w;
    let time = ck.misc.y;

    // The ray in the ship's frame: x right, y up, -z the nose.
    let ray = normalize(
        ck.fwd.xyz + ck.right.xyz * (in.ndc.x * tan_half * aspect) + ck.up.xyz * (in.ndc.y * tan_half)
    );
    // Azimuth from the nose (signed, right positive), elevation from the
    // eye-line.
    let az = atan2(ray.x, -ray.z);
    let el = asin(clamp(ray.y, -1.0, 1.0));
    let aaz = abs(az);
    // Angular size of this pixel, for the anti-aliasing.
    let aa = max(fwidth(el) + fwidth(az) * 0.5, 1e-4) * 1.2;

    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    var glow = 0.0;
    var warm = 0.0;
    var hull = 0.0;

    // ---- the sill and the hull below it -----------------------------------
    // The sill dips at the nose so the view down the boresight is clear,
    // and rises toward the shoulders where the side panels are.
    let sill = mix(SILL_FRONT, SILL_SIDE, smoothstep(25.0 * DEG, 95.0 * DEG, aaz));
    let below = el < sill;
    // Behind the bulkhead line everything is hull; the line itself leans
    // in at the top so the dome closes over the pilot's shoulders.
    let bulk = BULKHEAD_AZ - max(el, 0.0) * 0.45;
    let behind = aaz > bulk;
    if (below || behind) {
        hull = 1.0;
        // The dash: a fine grid in (az, el), lit, fading with depth below
        // the sill so the floor goes dark.
        let depth = select(aaz - bulk, sill - el, below);
        let fade = exp(-depth / (18.0 * DEG));
        let g_az = abs(fract(az / (6.0 * DEG) + 0.5) - 0.5) * 6.0 * DEG;
        let g_el = abs(fract(el / (6.0 * DEG) + 0.5) - 0.5) * 6.0 * DEG;
        let grid = max(line(g_az, 0.0, aa), line(g_el, 0.0, aa));
        glow += 0.18 * grid * fade;
        // Panel frames on the dash ahead: two instrument bays either side
        // of the nose, outlined.
        if (below) {
            let bay_az = abs(aaz - 34.0 * DEG);
            let bay_el = abs(el - (sill - 14.0 * DEG));
            let bay = max(line(bay_az - 22.0 * DEG, 0.0, aa) * step(bay_el, 10.0 * DEG),
                          line(bay_el - 10.0 * DEG, 0.0, aa) * step(bay_az, 22.0 * DEG));
            glow += 0.7 * bay;
            // A heartbeat along the bay's top edge.
            let pulse = 0.5 + 0.5 * sin(time * 1.5 + az * 3.0);
            glow += 0.25 * line(bay_el - 10.0 * DEG, 0.0, aa) * step(bay_az, 22.0 * DEG) * pulse;
        }
    }

    // ---- the sill line itself ---------------------------------------------
    // The glowing edge of the glass all the way round, and its twin on the
    // bulkhead.
    if (!behind) {
        glow += 1.0 * line(el - sill, aa * 0.4, aa);
        glow += 0.20 * (1.0 - smoothstep(0.0, 2.5 * DEG, abs(el - sill)));
    }
    if (!below) {
        glow += 0.9 * line(aaz - bulk, aa * 0.4, aa);
    }

    // ---- the dome: ribs and hoops ---------------------------------------
    if (!below && !behind) {
        // Ribs: great circles through the nose-to-tail axis... which in
        // angular space are lines of constant azimuth. Five of them, the
        // outer ones heavier.
        let ribs = array<f32, 3>(38.0 * DEG, 72.0 * DEG, 100.0 * DEG);
        for (var i = 0; i < 3; i += 1) {
            let w = select(aa * 0.35, aa * 0.6, i == 2);
            glow += 0.75 * line(aaz - ribs[i], w, aa);
        }
        // The spine over the crown, and a hoop where the dome begins to
        // curve over: lines of constant elevation.
        glow += 0.55 * line(el - CROWN, aa * 0.35, aa);
        glow += 0.35 * line(el - 28.0 * DEG, aa * 0.3, aa) * step(aaz, 72.0 * DEG);
        // A faint tint of the glass near its edges: the canopy is a thing,
        // not an absence.
        let to_edge = min(min(el - sill, bulk - aaz), 1.0);
        glow += 0.05 * (1.0 - smoothstep(0.0, 12.0 * DEG, to_edge));
        // Warning strip along the inner ribs' feet, amber, subtle.
        warm += 0.25 * line(aaz - 38.0 * DEG, aa * 0.4, aa) * step(el - sill, 4.0 * DEG);
    }

    let alpha = hull * hull_k;
    let lit = (cyan * glow + amber * warm) * glow_k * on;
    // Premultiplied: the hull darkens what it covers, the lines add.
    let ground = vec3<f32>(0.01, 0.02, 0.035) * alpha;
    if (alpha < 0.002 && dot(lit, lit) < 1e-6) {
        discard;
    }
    return vec4<f32>(ground + lit * on, alpha * on);
}
