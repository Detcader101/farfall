// blit.wgsl — upscale the scene target to the swapchain (SPEC §6.3).
//
// Lane: A. Cost class: trivial (one texture fetch per output pixel).
//
// The scene renders at a fraction of native and is stretched here, while the
// HUD is drawn afterwards straight onto the swapchain at full resolution. That
// split is the point: shading cost scales with the scene's pixel count, but
// text and instruments stay pin-sharp at any render scale (P1).

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // Clip space has +Y up, texture space has +V down.
    out.uv = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(scene_tex, scene_sampler, in.uv);
}
