// hud.wgsl — bitmap text on the canopy (SPEC §6.5, pass: hud)
//
// Lane: A (vertex+fragment only). Cost class: trivial (one small screen region,
// a handful of bit tests per pixel).
//
// The CPU rasterises text into a bit mask (see render/src/text.rs); this shader
// only asks "how much of this pixel does the mask cover" — but it asks in
// CANOPY space, not screen space: the readout lives on the same spherical shell
// as the instrument cluster, through the same canopy() warp from the common
// prelude, with the same hologram tint, scanlines and glass falloff. There is
// no flat debug overlay left in the game — everything the pilot reads is
// projected on one piece of glass.
//
// The text is box-filtered: each screen pixel's footprint in font pixels is
// intersected with the lit cells under it, so a glyph edge that falls between
// two pixels is a proportional grey rather than a jagged step, at 800x600 and
// at 2880x1800 alike. No mipmaps, no texture, no subpixel drift (SPEC P1): the
// bits are the truth and the filter is exact. The warp gently shears the grid
// near the rim, which is the point — that shear is the shape of the glass.

const COLS: u32 = 384u;
const ROWS: u32 = 180u;
// vec4<u32> per bitmap row.
const ROW_VECS: u32 = 3u;

struct Hud {
    // xy: anchor on the canopy in NDC (top-left of the text block),
    // z: font-pixel size in canopy units, w: aspect (w/h)
    a: vec4<f32>,
    // xy: the panel's extent in font pixels; z: surface height in px
    // (scanline frequency), w: the highlighted row's top in font px
    // (negative: none; its height is sway.w)
    extent: vec4<f32>,
    color: vec4<f32>,
    // Background glass tint; alpha 0 disables the panel.
    backdrop: vec4<f32>,
    // xy: hologram sway in canopy units (see HoloSway); z: flat (screen)
    // block; w: the highlighted row's height in font px.
    sway: vec4<f32>,
    // A scrollbar in font px: x: track top, y: track bottom, z: thumb top,
    // w: thumb bottom. x negative: none.
    bar: vec4<f32>,
    // x: a rule under the header at this font-px row (negative: none),
    // y: a rule over the footer (negative: none); text below it is drawn
    // in ivory, dimmer — the footnote, not the content. zw: unused.
    rules: vec4<f32>,
    // COLS bits per row, three vec4<u32> per row.
    rows: array<vec4<u32>, 540>,
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
    let word = u32(cell.x) >> 5u;
    let v = hud.rows[u32(cell.y) * ROW_VECS + (word >> 2u)];
    return (v[word & 3u] & (1u << (u32(cell.x) & 31u))) != 0u;
}

// Any lit cell within one of this one: the halo's mask.
fn near_bit(cell: vec2<i32>) -> bool {
    for (var dy = -1; dy <= 1; dy += 1) {
        for (var dx = -1; dx <= 1; dx += 1) {
            if (bit_at(cell + vec2<i32>(dx, dy))) {
                return true;
            }
        }
    }
    return false;
}

// The share of a pixel's footprint (`fw` font px wide, centred on `local`)
// that lit cells cover: an exact box filter over the up-to-four cells the
// footprint touches. `halo` filters the dilated mask instead.
fn coverage(local: vec2<f32>, fw: vec2<f32>, halo: bool) -> f32 {
    let half = fw * 0.5;
    let lo = local - half;
    let hi = local + half;
    let c0 = floor(lo);
    var cov = 0.0;
    for (var dy = 0; dy < 2; dy += 1) {
        for (var dx = 0; dx < 2; dx += 1) {
            let cell = c0 + vec2<f32>(f32(dx), f32(dy));
            let o = max(min(hi, cell + 1.0) - max(lo, cell), vec2<f32>(0.0));
            let lit = select(bit_at(vec2<i32>(cell)), near_bit(vec2<i32>(cell)), halo);
            if (lit) {
                cov += o.x * o.y;
            }
        }
    }
    return cov / (fw.x * fw.y);
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
    // The pixel's footprint in font pixels, taken before any discard so
    // the derivative is in uniform control flow. Never wider than a cell:
    // the filter reads at most four.
    let fw = clamp(fwidth(local), vec2<f32>(1e-3), vec2<f32>(1.0));
    let aa = max(fw.y, 1e-3);

    // Padding around the panel, wide enough that the swaying text never
    // walks off its own glass, and wide enough for a scrollbar.
    let pad = 4.0;
    if (panel.x < -pad || panel.y < -pad
        || panel.x >= hud.extent.x + pad || panel.y >= hud.extent.y + pad) {
        discard;
    }

    let glass = canopy_glass(in.ndc, aspect);
    // Static scanlines, matched to the gauge pass: same glass, same texture.
    let scan = 0.90 + 0.10 * sin(in.ndc.y * hud.extent.z * 1.7);
    let ivory = vec3<f32>(0.92, 0.90, 0.80);

    // The text: its coverage of this pixel, and a dark halo round it on
    // the glass so it reads over a white sky as well as over the stars.
    var ink = 0.0;
    var halo = 0.0;
    if (local.x >= -1.0 && local.y >= -1.0) {
        ink = coverage(local, fw, false);
        if (!flat) {
            halo = coverage(local, fw, true);
        }
    }
    // Footnote text (below the footer rule) in ivory, a shade dimmer.
    var text_rgb = hud.color.rgb;
    if (flat && hud.rules.y >= 0.0 && panel.y > hud.rules.y) {
        text_rgb = ivory * 0.78;
    }

    // The panel is smoked glass, not a debug box: it dims with the canopy
    // too. A halo darkens it further under the strokes.
    var back = vec4<f32>(hud.backdrop.rgb * glass, hud.backdrop.a);
    if (!flat) {
        back.a = min(back.a + halo * 0.55, 1.0);
    }
    if (flat) {
        // A card: a hairline of the hologram's light at its edge, a
        // brighter band behind the header, a rule under it, a rule over
        // the footer, a soft band on the chosen row, and the scrollbar.
        let edge = min(min(panel.x + pad, hud.extent.x + pad - panel.x),
                       min(panel.y + pad, hud.extent.y + pad - panel.y));
        let frame = 1.0 - smoothstep(0.3, 0.3 + aa * 1.5, edge);
        var rule = 0.0;
        var header = 0.0;
        if (hud.rules.x >= 0.0) {
            rule += 1.0 - smoothstep(0.25, 0.25 + aa, abs(panel.y - hud.rules.x));
            header = step(panel.y, hud.rules.x);
        }
        if (hud.rules.y >= 0.0) {
            rule += 0.6 * (1.0 - smoothstep(0.25, 0.25 + aa, abs(panel.y - hud.rules.y)));
        }
        var band = 0.0;
        if (hud.extent.w >= 0.0) {
            band = smoothstep(hud.extent.w - 1.0 - aa, hud.extent.w - 1.0, panel.y)
                 * (1.0 - smoothstep(hud.extent.w + hud.sway.w - 1.0, hud.extent.w + hud.sway.w - 1.0 + aa, panel.y));
        }
        var bar = 0.0;
        if (hud.bar.x >= 0.0) {
            let bx = hud.extent.x + pad * 0.5;
            let on_x = 1.0 - smoothstep(0.5, 0.5 + aa, abs(panel.x - bx));
            let on_track = step(hud.bar.x, panel.y) * step(panel.y, hud.bar.y);
            let on_thumb = smoothstep(hud.bar.z - aa, hud.bar.z, panel.y)
                         * (1.0 - smoothstep(hud.bar.w, hud.bar.w + aa, panel.y));
            bar = on_x * (0.22 * on_track + 0.75 * on_thumb);
        }
        let light = frame * 0.55 + rule * 0.22 + band * 0.10 + bar + header * 0.04;
        back = vec4<f32>(
            back.rgb + hud.color.rgb * light * scan,
            min(back.a + frame * 0.15 + band * 0.10 + header * 0.06, 1.0),
        );
    }
    if (back.a <= 0.0 && ink <= 0.0) {
        discard;
    }
    // The ink over the panel: alpha-composited here so the filter's
    // proportional greys blend with the glass rather than punching it.
    let lit = text_rgb * scan * glass;
    let a_ink = ink * hud.color.a;
    let a = a_ink + back.a * (1.0 - a_ink);
    let rgb = (lit * a_ink + back.rgb * back.a * (1.0 - a_ink)) / max(a, 1e-4);
    return vec4<f32>(rgb, a);
}
