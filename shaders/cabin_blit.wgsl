// cabin_blit.wgsl — the cabin's own render, laid over the scene (pass: cabin_blit)
//
// Lane: A. Cost class: trivial — one textured triangle.
//
// The cabin (cockpit.wgsl) is drawn at a fraction of the scene's size into
// a texture of its own — the march is the dearest thing per pixel in the
// frame, and a cabin of soft-shaded metal loses nothing at half size —
// then this lays it over the scene, premultiplied, scaled up with a linear
// filter. The holograms are drawn after, full size, and stay sharp.

struct CabinBlit {
    // The head's basis in ship frame; w of fwd: tan(fov/2)
    right: vec4<f32>,
    up: vec4<f32>,
    fwd: vec4<f32>,
    // x: aspect, y: on 0..1, z: time (s)
    misc: vec4<f32>,
    // x: main thrust 0..1 (the plumes), y: pitch demand -1..1, z: yaw
    // demand, w: roll demand — the RCS puffs.
    thrust: vec4<f32>,
}

@group(0) @binding(0) var cabin_tex: texture_2d<f32>;
@group(0) @binding(1) var cabin_sampler: sampler;
@group(0) @binding(2) var<uniform> cb: CabinBlit;

fn sd_capsule_line(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 1e-8), 0.0, 1.0);
    return length(pa - ba * h);
}

// The engines' light: two plumes out of the nozzles, their length with the
// thrust, gathered along the ray by closest approach — the same trick as
// the socket beams, a glowing line with a soft skirt. And the RCS: small
// puffs at the nose and the wingtips, lit by the demand on each axis.
fn thruster_light(ray: vec3<f32>, reach: f32, now: f32) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    let main = clamp(cb.thrust.x, 0.0, 1.0);
    if (main > 0.01) {
        let len = 2.0 + 9.0 * main;
        for (var i = 0; i < 2; i += 1) {
            let x = select(-0.62, 0.62, i == 1);
            let a = vec3<f32>(x, -0.85, 7.6);
            let e = vec3<f32>(0.0, 0.0, len);
            let ac = a - ray * dot(a, ray);
            let bc = e - ray * dot(e, ray);
            let u = clamp(-dot(ac, bc) / max(dot(bc, bc), 1e-8), 0.0, 1.0);
            let q = a + e * u;
            let t = clamp(dot(q, ray), 0.0, reach);
            let d = length(q - ray * t);
            // A needle of white-hot core with shock diamonds down its
            // first half, in a translucent blue-violet skin that widens
            // aft and ripples as it streams.
            let r_skin = 0.30 + 0.55 * u;
            let r_core = 0.09 + 0.07 * u;
            let tail = pow(1.0 - u * 0.85, 1.2);
            let along = u * len;
            let rip = vnoise(vec3<f32>(x * 3.0, along * 2.6 - now * 34.0, d * 4.0));
            let dd = d / r_skin;
            let shell = exp(-dd * dd * 2.2) * (0.45 + 0.7 * smoothstep(0.35, 0.95, dd) * (1.0 - smoothstep(0.95, 1.4, dd)));
            let skin = shell * (0.6 + 0.7 * rip) * tail;
            let dc = d / r_core;
            let core = exp(-dc * dc * 1.6) * tail;
            let diamonds = pow(0.5 + 0.5 * cos(along * 4.2 - 0.6), 6.0) * (1.0 - smoothstep(0.1, 0.65, u)) * exp(-dc * dc * 0.9);
            let skin_col = mix(vec3<f32>(0.30, 0.50, 1.00), vec3<f32>(0.55, 0.35, 1.00), u);
            light += (skin_col * skin * 1.4 + vec3<f32>(0.85, 0.90, 1.0) * core * 2.2 + vec3<f32>(1.0) * diamonds * 1.6) * main;
        }
    }
    // RCS puffs: nose up/down for pitch, nose left/right for yaw, the
    // wingtips for roll. A puff is a small bright ball of gas.
    let pitch = cb.thrust.y;
    let yaw = cb.thrust.z;
    let roll = cb.thrust.w;
    let puffs = array<vec4<f32>, 6>(
        vec4<f32>(0.0, -0.55, -5.6, max(pitch, 0.0)),
        vec4<f32>(0.0, -1.35, -5.6, max(-pitch, 0.0)),
        vec4<f32>(-0.55, -0.95, -5.4, max(yaw, 0.0)),
        vec4<f32>(0.55, -0.95, -5.4, max(-yaw, 0.0)),
        vec4<f32>(-5.6, -0.75, 4.5, max(-roll, 0.0)),
        vec4<f32>(5.6, -0.75, 4.5, max(roll, 0.0)),
    );
    for (var i = 0; i < 6; i += 1) {
        let pf = puffs[i];
        if (pf.w < 0.02) { continue; }
        let t = clamp(dot(pf.xyz, ray), 0.0, reach);
        let d = length(pf.xyz - ray * t);
        light += vec3<f32>(0.85, 0.9, 1.0) * exp(-d / 0.18) * pf.w * 0.8;
    }
    return light;
}



struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let ndc = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(ndc, 0.0, 1.0);
    out.uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var c = textureSample(cabin_tex, cabin_sampler, in.uv);
    // The engines and the RCS, drawn here at full size every frame (they
    // change with the throttle; the cabin behind them does not): light
    // gathered along this pixel's ray, hidden where the hull is in front.
    if (cb.misc.y > 0.5 && (cb.thrust.x > 0.01 || dot(cb.thrust.yzw, cb.thrust.yzw) > 1e-4)) {
        let ndc = vec2<f32>(in.uv.x * 2.0 - 1.0, 1.0 - in.uv.y * 2.0);
        let ray = normalize(
            cb.fwd.xyz + cb.right.xyz * (ndc.x * cb.fwd.w * cb.misc.x) + cb.up.xyz * (ndc.y * cb.fwd.w)
        );
        let tl = thruster_light(ray, 40.0, cb.misc.z);
        let lit = (vec3<f32>(1.0) - exp(-tl * 1.2)) * (1.0 - c.a);
        // The plumes' glow on the hull: what faces aft and down — the
        // wings' upper skins, the dash's far edge — catches a blue cast
        // under thrust, the RCS a white flick where it fires.
        let aft = smoothstep(-0.2, 0.9, ray.z);
        let down = smoothstep(0.1, -0.6, ray.y);
        let wash = vec3<f32>(0.30, 0.50, 1.00) * cb.thrust.x * (0.18 * aft + 0.06 * down * (1.0 - aft)) * (0.85 + 0.15 * sin(cb.misc.z * 53.0));
        c = vec4<f32>(c.rgb + lit + wash * c.a, c.a);
    }
    if (c.a < 0.002 && dot(c.rgb, c.rgb) < 1e-6) {
        discard;
    }
    return c;
}
