// xrblit.wgsl — native VR's cut-out blit (SPEC §5.3, `app/src/xr.rs::cutout_uv`).
//
// Lane: A. Cost class: trivial — one fetch per output pixel, same as blit.wgsl.
//
// Each eye is rendered as a symmetric frustum wide enough to hold its true
// asymmetric one (`VrEye::symmetric`); on the web that render is cropped back
// to the real field by the browser's WebGL compositor. Native has no
// browser, so this pass is that crop: it samples an arbitrary UV rectangle
// of the source (the rendered pair, one eye's half, cut down to its true
// field) and stretches it to fill the whole destination — a plain rescale
// once the crop is right, since a rectilinear projection's crop-then-stretch
// exactly reconstructs the narrower frustum it was cropped from. The same
// pass, with `rect = [0, 0, 0.5, 1]` (no crop, just the left half), is also
// the mirror window's draw.

struct XrBlit {
    // u0, v0, u1, v1 — the source rectangle, in the bound texture's own
    // UV space (v = 0 at the top, matching blit.wgsl's convention).
    rect: vec4<f32>,
}

@group(0) @binding(0) var scene_tex: texture_2d<f32>;
@group(0) @binding(1) var scene_sampler: sampler;
@group(0) @binding(2) var<uniform> u: XrBlit;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = fullscreen_ndc(vi);
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    // Clip space has +Y up, texture space has +V down (blit.wgsl).
    let base = vec2<f32>(xy.x * 0.5 + 0.5, 0.5 - xy.y * 0.5);
    out.uv = mix(u.rect.xy, u.rect.zw, base);
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return textureSample(scene_tex, scene_sampler, in.uv);
}
