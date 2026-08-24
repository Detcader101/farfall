// holo.wgsl — the holo3PP panel (pass: holo)
//
// Lane: A (vertex+fragment only). Cost class: trivial (one small screen
// region, one texture read per pixel).
//
// A live third-person projection of the ship, on the inside of the ship:
// the chase view is rendered to a small texture every frame (the same
// passes as the real sky — starfield, bodies, planet, belt, and the jet
// itself), and this pass puts that picture on the canopy glass as a
// hologram panel. Third person without ever leaving first person: the
// pilot watches the outside of their own ship on a screen inside it.
//
// The picture itself is faithful — no tint over the image, because the
// point is a perfect representation — but the panel is glassware: a
// hairline hologram frame with corner ticks, the cluster's scanlines,
// and the canopy's own falloff, so it sits in the cockpit rather than
// floating over the game.

struct Holo {
    // xy: panel centre on the canopy (NDC), z: half height in canopy
    // units, w: aspect (w/h of the screen)
    a: vec4<f32>,
    // x: the texture's aspect (w/h), y: surface height in px (scanline
    // frequency), z: shown (0 skips), w: time (s)
    b: vec4<f32>,
    // xy: hologram sway (canopy units), zw: unused
    sway: vec4<f32>,
}

@group(0) @binding(0) var<uniform> holo: Holo;
@group(0) @binding(1) var holo_tex: texture_2d<f32>;
@group(0) @binding(2) var holo_samp: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    if (holo.b.z < 0.5) {
        discard;
    }
    let aspect = holo.a.w;
    // On the glass, like every instrument: the same canopy warp, the same
    // sway as the cluster, so the panel is part of the cockpit.
    let p = canopy(in.ndc, aspect) - canopy(holo.a.xy, aspect) - holo.sway.xy;
    let hh = max(holo.a.z, 1e-4);
    let hw = hh * holo.b.x;
    let edge = 0.012;
    if (abs(p.x) > hw + edge || abs(p.y) > hh + edge) {
        discard;
    }
    let glass = canopy_glass(in.ndc, aspect);
    let cyan = vec3<f32>(0.45, 0.92, 1.0);

    // Inside: the picture, upright (texture v runs down).
    if (abs(p.x) <= hw && abs(p.y) <= hh) {
        let uv = vec2<f32>(p.x / hw, -p.y / hh) * 0.5 + vec2<f32>(0.5);
        var rgb = textureSample(holo_tex, holo_samp, uv).rgb;
        // The cluster's static scanlines, faint: a hologram, not a hole
        // in the canopy.
        let scan = 0.96 + 0.04 * sin(in.ndc.y * holo.b.y * 1.7);
        rgb *= scan * mix(0.75, 1.0, glass);
        // The frame's light bleeds a hair over the picture's rim.
        let rim = max(abs(p.x) / hw, abs(p.y) / hh);
        rgb += cyan * smoothstep(0.965, 1.0, rim) * 0.25;
        return vec4<f32>(rgb, 0.96);
    }

    // The frame: a hairline of the hologram's light, brighter at the
    // corner ticks, breathing very slightly.
    let bx = abs(p.x) - hw;
    let by = abs(p.y) - hh;
    let d = max(bx, by);
    let line = 1.0 - smoothstep(0.0, edge, d);
    let tick = step(hw - 0.09, abs(p.x)) + step(hh - 0.09, abs(p.y));
    let breathe = 0.9 + 0.1 * sin(holo.b.w * 2.1);
    let lit = cyan * line * (0.5 + 0.35 * min(tick, 1.0)) * breathe * glass;
    return vec4<f32>(lit, line * 0.85);
}
