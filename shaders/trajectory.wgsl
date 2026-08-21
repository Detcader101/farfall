// trajectory.wgsl — where the ship is going (pass: trajectory)
//
// Lane: A (vertex+fragment only). Cost class: cheap — a few hundred thin
// quads, and the integration lives in the vertex stage where there are
// hundreds of invocations, not millions.
//
// The predicted path is COMPUTED on the GPU: every vertex integrates the
// ship's ballistic future from the same initial state — gravity toward the
// planet, nose-on drag through the exponential atmosphere — for its own
// prefix of the timeline, and lands where the ship will be. No buffer of
// points crosses from the CPU; the CPU hands over the state and the laws.
//
// The ribbon is drawn in TRUE projection, not on the canopy: a path through
// the world has to lie on the world, passing over the ground it will pass
// over and ending on the ground it will hit. A reticle marks where it
// starts — the prograde point, the direction the ship is actually moving —
// and a boresight marks the nose, so the two can be read against each
// other: that gap is angle of attack.
//
// Segments are spaced quadratically in time, dense near the ship where
// perspective magnifies every metre, sparse far out where an orbit is a
// gentle curve.

struct Traj {
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // x: tan(fov_y/2), y: aspect, z: time_s, w: exposure
    params: vec4<f32>,
    // xyz: planet centre relative to the ship, metres. w: radius, m.
    centre_radius: vec4<f32>,
    // x: mu (m³/s²), y: sea-level density (kg/m³), z: scale height (m),
    // w: atmosphere top (m)
    phys: vec4<f32>,
    // xyz: ship velocity, world frame, m/s. w: CdA / m (m²/kg).
    vel: vec4<f32>,
    // x: prediction horizon, s. y: segment count. z: visibility 0..1,
    // w: screen height, px.
    look: vec4<f32>,
    // x: odometer — metres of path the ship has already flown, so the marks
    // can stay fixed to the world and stream past. y: mark spacing, m.
    mark: vec4<f32>,
}

@group(0) @binding(0) var<uniform> tj: Traj;

// Substeps per segment.
const SUBSTEPS: u32 = 6u;
// Ribbon width and reticle radius, pixels.
const RIBBON_PX: f32 = 3.0;
const RETICLE_PX: f32 = 22.0;
const BORESIGHT_PX: f32 = 14.0;
// The marks along the path are FIXED TO THE WORLD: an even grid of
// mark.y metres laid along the trajectory, phased by the odometer so that
// as the ship flies the marks stream toward it and past — the hoops show
// the velocity, not just the route. What each mark looks like depends on
// its distance from the ship:
//
//   hoops   — the nearest RING_COUNT marks, within HOOPS_TO_M: true rings
//             in the world. Radius grows as sqrt(d) so the far ones shrink
//             gently and still read as depth; thickness grows with d so
//             line weight holds on screen.
//   dashes  — beyond that, to DOTS_FROM_M: the ribbon breaks into dashes
//             on the same grid.
//   dots    — beyond, to the horizon or the edge of the view: dots, dimmer.
const RING_COUNT: u32 = 8u;
const HOOPS_TO_M: f32 = 9000.0;
const RING_RADIUS_M: f32 = 90.0;
const DOTS_FROM_M: f32 = 40000.0;

// Distance from the ship of world-fixed mark i: the first mark is the
// next grid line ahead, the rest follow at the spacing.
fn ring_distance(i: u32) -> f32 {
    let spacing = max(tj.mark.y, 1.0);
    let ahead = spacing - fract(tj.mark.x / spacing) * spacing;
    return ahead + f32(i) * spacing;
}

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    // x: across the ribbon -1..1 (or local uv for the reticles),
    // y: along — metres of path.
    @location(0) uv: vec2<f32>,
    // x: 0 ribbon, 1 reticle, 2 boresight, 3 distance ring. y: 1 if the
    // path hits the ground before this point. z: fraction of the horizon
    // (ribbon) or of the ring count (rings).
    @location(1) kind: vec3<f32>,
}

// Time at the start of segment k: quadratic spacing.
fn seg_time(k: f32) -> f32 {
    let n = max(tj.look.y, 1.0);
    let f = k / n;
    return tj.look.x * f * f;
}

struct Point {
    pos: vec3<f32>,
    hit: f32,
    // Metres of path travelled to get here.
    dist: f32,
}

// Integrate the ballistic path to the start of segment `k`. Semi-implicit
// Euler, like the sim itself (SPEC §7.2).
fn integrate(k: u32) -> Point {
    var p = vec3<f32>(0.0);
    var v = tj.vel.xyz;
    let c = tj.centre_radius.xyz;
    let radius = tj.centre_radius.w;
    let mu = tj.phys.x;
    var hit = 0.0;
    var dist = 0.0;
    for (var seg = 0u; seg < k; seg += 1u) {
        let dt = (seg_time(f32(seg + 1u)) - seg_time(f32(seg))) / f32(SUBSTEPS);
        for (var s = 0u; s < SUBSTEPS; s += 1u) {
            if (hit > 0.5) {
                break;
            }
            let rel = p - c;
            let r = length(rel);
            var a = rel * (-mu / (r * r * r));
            let h = r - radius;
            if (h < tj.phys.w) {
                let rho = tj.phys.y * exp(-h / tj.phys.z);
                let speed = length(v);
                a -= v * (0.5 * rho * speed * tj.vel.w);
            }
            v += a * dt;
            p += v * dt;
            dist += length(v) * dt;
            if (length(p - c) < radius) {
                // Land on the surface, and stay there.
                p = c + normalize(p - c) * radius;
                hit = 1.0;
            }
        }
    }
    return Point(p, hit, dist);
}

struct RingPoint {
    pos: vec3<f32>,
    dir: vec3<f32>,
    ok: bool,
}

// Integrate along the path until `dist` metres of it have gone by. Same
// laws and steps as integrate(); `ok` is false if the ground or the horizon
// comes first.
fn integrate_to_distance(dist: f32) -> RingPoint {
    var p = vec3<f32>(0.0);
    var v = tj.vel.xyz;
    let c = tj.centre_radius.xyz;
    let radius = tj.centre_radius.w;
    let mu = tj.phys.x;
    let n = u32(max(tj.look.y, 1.0));
    var travelled = 0.0;
    for (var seg = 0u; seg < n; seg += 1u) {
        let dt = (seg_time(f32(seg + 1u)) - seg_time(f32(seg))) / f32(SUBSTEPS);
        for (var s = 0u; s < SUBSTEPS; s += 1u) {
            let rel = p - c;
            let r = length(rel);
            var a = rel * (-mu / (r * r * r));
            let h = r - radius;
            if (h < tj.phys.w) {
                let rho = tj.phys.y * exp(-h / tj.phys.z);
                a -= v * (0.5 * rho * length(v) * tj.vel.w);
            }
            v += a * dt;
            let step = v * dt;
            let len = length(step);
            if (travelled + len >= dist) {
                // Land exactly on the distance, inside this step.
                let f = (dist - travelled) / max(len, 1e-6);
                let q = p + step * f;
                if (length(q - c) < radius) {
                    return RingPoint(q, v, false);
                }
                return RingPoint(q, normalize(v), true);
            }
            p += step;
            travelled += len;
            if (length(p - c) < radius) {
                return RingPoint(p, v, false);
            }
        }
    }
    return RingPoint(p, v, false);
}

// World (camera-relative) to NDC; z is the view depth for clipping.
fn project(p: vec3<f32>) -> vec3<f32> {
    let x = dot(p, tj.right.xyz);
    let y = dot(p, tj.up.xyz);
    let z = dot(p, tj.forward.xyz);
    let tan_half = tj.params.x;
    let aspect = tj.params.y;
    return vec3<f32>(x / (z * tan_half * aspect), y / (z * tan_half), z);
}

// A small screen-space quad (6 vertices) centred on `ndc`, `px` pixels in
// radius, collapsed to nothing when `show` is false.
fn quad_vertex(ndc: vec2<f32>, px: f32, corner: u32, show: bool, kind: f32) -> VsOut {
    let corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let h = max(tj.look.w, 1.0);
    let aspect = tj.params.y;
    let local = corners[corner];
    let size = select(0.0, px * 2.0 / h, show);
    var out: VsOut;
    out.pos = vec4<f32>(ndc + local * size * vec2<f32>(1.0 / aspect, 1.0), 0.0, 1.0);
    out.uv = local;
    out.kind = vec3<f32>(kind, 0.0, 0.0);
    return out;
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let n = u32(max(tj.look.y, 1.0));
    let ribbon_verts = n * 6u;

    if (vi >= ribbon_verts + 12u) {
        // Distance rings: a hoop in the world, perpendicular to the path,
        // every RING_SPACING_M along it.
        let i = (vi - ribbon_verts - 12u) / 6u;
        let corner = (vi - ribbon_verts - 12u) % 6u;
        let corners = array<vec2<f32>, 6>(
            vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
            vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        );
        let local = corners[corner];
        let ring_d = ring_distance(i);
        let ring = integrate_to_distance(ring_d);
        let in_range = ring_d < HOOPS_TO_M;
        var out: VsOut;
        out.uv = local;
        out.kind = vec3<f32>(3.0, 0.0, ring_d / HOOPS_TO_M);
        // A basis across the path: up-ish first, then the cross product.
        let cref = tj.centre_radius.xyz;
        var side = cross(ring.dir, normalize(-cref + ring.pos * 0.0));
        if (dot(side, side) < 1e-6) {
            side = cross(ring.dir, vec3<f32>(1.0, 0.0, 0.0));
        }
        side = normalize(side);
        let upish = cross(side, ring.dir);
        let ring_r = RING_RADIUS_M * sqrt(ring_d / 1000.0);
        let world = ring.pos + (side * local.x + upish * local.y) * ring_r;
        let pr = project(world);
        if (!ring.ok || !in_range || pr.z <= 1.0 || dot(ring.pos, tj.forward.xyz) <= 1.0) {
            out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
            return out;
        }
        out.pos = vec4<f32>(pr.xy, 0.0, 1.0);
        return out;
    }

    if (vi >= ribbon_verts) {
        // The reticles: prograde at the first path point, boresight at the
        // nose. Prograde hides when the ship is moving away from the view.
        let which = (vi - ribbon_verts) / 6u;
        let corner = (vi - ribbon_verts) % 6u;
        if (which == 0u) {
            let first = project(integrate(1u).pos);
            return quad_vertex(first.xy, RETICLE_PX, corner, first.z > 0.0, 1.0);
        }
        return quad_vertex(vec2<f32>(0.0), BORESIGHT_PX, corner, true, 2.0);
    }

    // Ribbon: segment k, quad corner. The path starts one segment out —
    // segment 0 would begin at the camera itself, which has no projection.
    let k = vi / 6u + 1u;
    let corner = vi % 6u;
    let end = select(0u, 1u, corner == 1u || corner == 4u || corner == 5u);
    let side = select(-1.0, 1.0, corner == 2u || corner == 3u || corner == 5u);

    let a = integrate(k);
    let b = integrate(k + 1u);
    let pa = project(a.pos);
    let pb = project(b.pos);
    let here = select(pa, pb, end == 1u);
    let hit = select(a.hit, b.hit, end == 1u);

    var out: VsOut;
    // Either end behind the camera: collapse the quad.
    if (pa.z <= 1.0 || pb.z <= 1.0) {
        out.pos = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        out.uv = vec2<f32>(0.0);
        out.kind = vec3<f32>(0.0, hit, 0.0);
        return out;
    }
    // Perpendicular in aspect-corrected screen space, scaled to pixels.
    let aspect = tj.params.y;
    let h = max(tj.look.w, 1.0);
    let d = (pb.xy - pa.xy) * vec2<f32>(aspect, 1.0);
    let len = length(d);
    let perp = select(vec2<f32>(0.0, 1.0), vec2<f32>(-d.y, d.x) / len, len > 1e-6);
    let half_px = RIBBON_PX * 0.5 * 2.0 / h;
    let offset = perp * half_px * side * vec2<f32>(1.0 / aspect, 1.0);
    out.pos = vec4<f32>(here.xy + offset, 0.0, 1.0);
    out.uv = vec2<f32>(side, select(a.dist, b.dist, end == 1u));
    out.kind = vec3<f32>(0.0, hit, f32(k + end) / f32(n));
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let vis = tj.look.z;
    if (vis < 0.01) {
        discard;
    }
    let time = tj.params.z;
    let cyan = vec3<f32>(0.22, 0.85, 1.0);
    let amber = vec3<f32>(1.0, 0.62, 0.18);
    var colour = vec3<f32>(0.0);

    if (in.kind.x < 0.5) {
        // Ribbon: soft edges; solid through the hoops, kilometre dashes
        // beyond them, kilometre dots beyond that. Amber once the ground is
        // in it.
        let edge = 1.0 - smoothstep(0.35, 1.0, abs(in.uv.x));
        let dist = max(in.uv.y, 1.0);
        // The same world-fixed grid the hoops sit on.
        let spacing = max(tj.mark.y, 1.0);
        let grid = fract((dist + tj.mark.x) / spacing);
        let hoops_end = HOOPS_TO_M;
        // Duty cycle of the lit part of each mark: all of it under the
        // hoops, half of it dashed, a quarter of it dotted.
        let duty = select(select(0.25, 0.5, dist < DOTS_FROM_M), 1.0, dist < hoops_end);
        let soft = fwidth(grid) * 1.5;
        let lit = 1.0 - smoothstep(duty - soft, duty + soft, grid);
        let pattern = select(lit, 1.0, duty >= 1.0);
        let level = select(select(0.55, 0.8, dist < DOTS_FROM_M), 1.0, dist < hoops_end);
        let fade = 1.0 - in.kind.z * 0.5;
        let tint = mix(cyan, amber, in.kind.y);
        colour = tint * edge * pattern * level * fade * 0.9;
    } else if (in.kind.x > 2.5) {
        // Distance ring: a hoop that gets thicker with every kilometre, so
        // the far ones hold their weight on screen as perspective shrinks
        // them — thickness in metres grows, thickness in pixels stays
        // readable.
        let r = length(in.uv);
        let aa = max(fwidth(r) * 1.2, 0.01);
        let thickness = 0.02 + 0.09 * clamp(in.kind.z, 0.0, 1.0);
        let hoop = 1.0 - smoothstep(0.0, aa, abs(r - (0.92 - thickness)) - thickness);
        colour = cyan * hoop * 0.8;
    } else {
        // Reticles: SDF rings and ticks in the quad's local space.
        let r = length(in.uv);
        let aa = fwidth(r) * 1.2;
        if (in.kind.x < 1.5) {
            // Prograde: a ring with four gaps and a centre dot.
            let ring = 1.0 - smoothstep(0.0, aa, abs(r - 0.72) - 0.05);
            let gaps = step(0.30, abs(in.uv.x)) * step(0.30, abs(in.uv.y))
                + step(0.0, -abs(in.uv.x) + 0.30) * step(0.0, -abs(in.uv.y) + 0.30);
            let dot = 1.0 - smoothstep(0.0, aa, r - 0.12);
            colour = cyan * (ring * min(gaps, 1.0) + dot);
        } else {
            // Boresight: a small cross with an open centre.
            let arm = min(abs(in.uv.x), abs(in.uv.y));
            let reach = max(abs(in.uv.x), abs(in.uv.y));
            let cross = (1.0 - smoothstep(0.0, aa, arm - 0.06))
                * step(0.35, reach) * step(reach, 0.95);
            colour = cyan * cross * 0.8;
        }
    }
    if (dot(colour, colour) < 1e-6) {
        discard;
    }
    // Additive, like the gauges: black costs nothing.
    return vec4<f32>(colour * vis, 1.0);
}
