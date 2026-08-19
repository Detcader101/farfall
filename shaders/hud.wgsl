// hud.wgsl — bitmap text overlay (SPEC §6.5, pass: hud)
//
// Lane: A (vertex+fragment only). Cost class: trivial (one small screen region,
// one bit test per pixel).
//
// The CPU rasterises text into a bit mask (see render/src/text.rs); this shader
// only asks "is this bit set". Pixel-snapped and unfiltered on purpose: a HUD
// must stay razor sharp at any resolution (SPEC P1), so no smoothing, no
// mipmaps, no subpixel drift.

const COLS: u32 = 128u;
const ROWS: u32 = 64u;

struct Hud {
    // xy: top-left origin in physical pixels, z: pixels per font pixel, w: unused
    origin_scale: vec4<f32>,
    // xy: occupied extent in font pixels; the backdrop hugs this, not the buffer
    extent: vec4<f32>,
    color: vec4<f32>,
    // Background panel colour; alpha 0 disables the panel.
    backdrop: vec4<f32>,
    // COLS bits per row, four u32 per row.
    rows: array<vec4<u32>, 64>,
}

@group(0) @binding(0) var<uniform> hud: Hud;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    var out: VsOut;
    out.pos = vec4<f32>(xy, 0.0, 1.0);
    return out;
}

fn bit_at(cell: vec2<u32>) -> bool {
    if (cell.x >= COLS || cell.y >= ROWS) {
        return false;
    }
    let word = hud.rows[cell.y][cell.x >> 5u];
    return (word & (1u << (cell.x & 31u))) != 0u;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let scale = max(hud.origin_scale.z, 1.0);
    let local = (in.pos.xy - hud.origin_scale.xy) / scale;

    // One font pixel of padding around the panel, which hugs the text.
    let pad = 1.0;
    if (local.x < -pad || local.y < -pad
        || local.x >= hud.extent.x + pad || local.y >= hud.extent.y + pad) {
        discard;
    }

    if (local.x >= 0.0 && local.y >= 0.0) {
        let cell = vec2<u32>(u32(local.x), u32(local.y));
        if (bit_at(cell)) {
            return hud.color;
        }
    }

    if (hud.backdrop.a <= 0.0) {
        discard;
    }
    return hud.backdrop;
}
