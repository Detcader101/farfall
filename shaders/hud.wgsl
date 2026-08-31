// hud.wgsl — bitmap text on the canopy (SPEC §6.5, pass: hud)
//
// Lane: A (vertex+fragment only). Cost class: trivial (one small screen region,
// one bit test per pixel).
//
// The CPU rasterises text into a bit mask (see render/src/text.rs); this shader
// only asks "is this bit set" — but it asks in CANOPY space, not screen space:
// the readout lives on the same spherical shell as the instrument cluster,
// through the same canopy() warp from the common prelude, with the same
// hologram tint, scanlines and glass falloff. There is no flat debug overlay
// left in the game — everything the pilot reads is projected on one piece of
// glass.
//
// The text is still unfiltered on purpose: a bit is lit or it is not, with a
// half-pixel analytic edge, no mipmaps, no subpixel drift (SPEC P1). The warp
// gently shears the grid near the rim, which is the point — that shear is the
// shape of the glass.

const COLS: u32 = 128u;
const ROWS: u32 = 96u;

struct Hud {
    // xy: anchor on the canopy in NDC (top-left of the text block),
    // z: font-pixel size in canopy units, w: aspect (w/h)
    a: vec4<f32>,
    // xy: occupied extent in font pixels; the panel hugs this, not the buffer.
    // z: surface height in px (scanline frequency), w: the highlighted
    // row's top in font px (negative: none; its height is sway.w)
    extent: vec4<f32>,
    color: vec4<f32>,
    // Background glass tint; alpha 0 disables the panel.
    backdrop: vec4<f32>,
    // xy: hologram sway in canopy units (see HoloSway); z: flat (screen)
    // block; w: the highlighted row's height in font px.
    sway: vec4<f32>,
    // COLS bits per row, four u32 per row.
    rows: array<vec4<u32>, 96>,
}

@group(0) @binding(0) var<uniform> hud: Hud;

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

fn bit_at(cell: vec2<i32>) -> bool {
    if (cell.x < 0 || cell.y < 0 || cell.x >= i32(COLS) || cell.y >= i32(ROWS)) {
        return false;
    }
    let word = hud.rows[u32(cell.y)][u32(cell.x) >> 5u];
    return (word & (1u << (u32(cell.x) & 31u))) != 0u;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    let aspect = hud.a.w;
    let px = max(hud.a.z, 1e-6);

    // Same warp as every instrument: pixel and anchor both live on the shell.
    // A flat block sits on the screen (the pause panels, which follow the
    // head); a glass block takes the shell's warp like every instrument.
    let flat = hud.sway.z > 0.5;
    let p = select(
        canopy(in.ndc, aspect) - canopy(hud.a.xy, aspect),
        (in.ndc - hud.a.xy) * vec2<f32>(aspect, 1.0),
        flat,
    );
    // Two depth layers: the glyphs float in front of the smoked panel, so
    // under rotation they parallax apart — same inertia vector as the
    // instrument cluster, same one piece of glass.
    let p_text = p - hud.sway.xy;
    let p_panel = p - hud.sway.xy * 0.55;
    // Font-pixel coordinates: x right, y down from the anchor.
    let local = vec2<f32>(p_text.x, -p_text.y) / px;
    let panel = vec2<f32>(p_panel.x, -p_panel.y) / px;

    // Padding around the panel, wide enough that the swaying text never
    // walks off its own glass.
    let pad = 3.0;
    if (panel.x < -pad || panel.y < -pad
        || panel.x >= hud.extent.x + pad || panel.y >= hud.extent.y + pad) {
        discard;
    }

    let glass = canopy_glass(in.ndc, aspect);
    // Static scanlines, matched to the gauge pass: same glass, same texture.
    let scan = 0.90 + 0.10 * sin(in.ndc.y * hud.extent.z * 1.7);

    if (local.x >= 0.0 && local.y >= 0.0 && bit_at(vec2<i32>(floor(local)))) {
        let lit = hud.color.rgb * scan * glass;
        return vec4<f32>(lit, hud.color.a);
    }

    if (hud.backdrop.a <= 0.0) {
        discard;
    }
    // The panel is smoked glass, not a debug box: it dims with the canopy too.
    var back = vec4<f32>(hud.backdrop.rgb * glass, hud.backdrop.a);
    if (flat) {
        // A card: a hairline of the hologram's light at its edge, a
        // dimmer rule under the header, and a soft band on the chosen row.
        let edge = min(min(panel.x + pad, hud.extent.x + pad - panel.x),
                       min(panel.y + pad, hud.extent.y + pad - panel.y));
        let frame = 1.0 - smoothstep(0.35, 0.85, edge);
        let rule = 1.0 - smoothstep(0.1, 0.5, abs(panel.y - 5.6)) * 1.0;
        var band = 0.0;
        if (hud.extent.w >= 0.0) {
            band = step(hud.extent.w - 0.5, panel.y) * step(panel.y, hud.extent.w + hud.sway.w - 0.5);
        }
        back = vec4<f32>(
            back.rgb + hud.color.rgb * (frame * 0.55 + rule * 0.18 + band * 0.07) * scan,
            min(back.a + frame * 0.15 + band * 0.08, 1.0),
        );
    }
    return back;
}
