// cabin_blit.wgsl — the cabin's own render, laid over the scene (pass: cabin_blit)
//
// Lane: A. Cost class: trivial — one textured triangle.
//
// The cabin (cockpit.wgsl) is drawn at a fraction of the scene's size into
// a texture of its own — the march is the dearest thing per pixel in the
// frame, and a cabin of soft-shaded metal loses nothing at half size —
// then this lays it over the scene, premultiplied, scaled up with a linear
// filter. The holograms are drawn after, full size, and stay sharp.

@group(0) @binding(0) var cabin_tex: texture_2d<f32>;
@group(0) @binding(1) var cabin_sampler: sampler;

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
    let c = textureSample(cabin_tex, cabin_sampler, in.uv);
    if (c.a < 0.002 && dot(c.rgb, c.rgb) < 1e-6) {
        discard;
    }
    return c;
}
