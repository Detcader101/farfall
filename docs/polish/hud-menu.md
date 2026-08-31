# fable/hud-menu — everything a new player reads or navigates

Branch `fable/hud-menu`, worktree `farfall-wt/hud-menu`. Scope: the font, the
settings menu, a first-run CONTROLS card, the readout, the stock gauges (mini
map + holo3PP), and the hologram's range and enemy marks.

## What changed, by piece

### Font (`crates/render/src/text.rs`, `shaders/hud.wgsl`, `crates/render/src/hud.rs`)

- 3×5 → **5×7 glyphs**, advance 6, line pitch 9 (`text::LINE`). The whole
  printable ASCII set with the look-alikes resolved (slashed `0`, serifed `1`,
  square `5`, flat-sided `B`), plus three marks (◆ cursor, ↑↓ dropdown). The
  bitmap grows to **384×180** (12 words a row = three `vec4<u32>`; 20 lines).
- The shader **box-filters** the bits over each pixel's footprint (`fwidth`,
  taken before any `discard`), so a glyph edge between two pixels is a
  proportional grey — clean at 1.5 px/dot (800×600) and at 4.5 px/dot
  (2880×1800). On the glass a **dark halo** (a dilated mask, same filter)
  sits under the strokes so the readout reads over a white sky.
- `panel::px_canopy(height)`: the scale is **fractional** (`h/400`, 1.5..8 px a
  dot) instead of `floor(h/260)`, which makes one font pixel a constant 1/200
  of the screen height: every block is the same size in canopy units on every
  supported screen, so a layout proven to fit at 800×600 fits at 2880×1800.
- `HudPass::update` takes a `HudBlock` (anchor, scale, flat, highlight,
  extent override, scrollbar, rules) instead of nine positional arguments.
  `text::wrap` word-wraps; `draw_line(col, line, ..)` lays text on the pitch.

### Menu (`crates/app/src/menu.rs`)

- A **48-column card**, header of **eight tabs** (GFX KEYS CABIN DIALS ARMS
  MAP SHIP HELP — MAP and SHIP are real settings pages now; the DRIVE panel and
  the bay card are the same `Menu` standalone at 32 columns), **12 rows**, a
  **shader-drawn scrollbar** (geometry from `Menu::scrollbar()`, tested), a
  footer with `ROW n/m` and the keys, and a **two-line description** of the
  chosen row under a footer rule, drawn in ivory. PageUp/PageDown. A click on
  a tab pages to it.
- Every row has `describe()`; every setting-editing row has `keys()`; the
  coverage test `every_settings_key_has_a_menu_row` walks
  `settings::KEYS` (the new ledger of every file key, itself tested against
  `render()`) and fails on any key without a row. New rows: SAFE EDGE (was
  file-only), HOLO RANGE, CARD AT START. The bay hologram look moved from GFX
  to SHIP.
- The card is kept by its **centre** (`ui.panel-menu` default `0,0.04`) and
  laid out per aspect, so it is centred on any screen and still draggable.
- **HELP page**: `HELP` table of five groups (FLIGHT, DRIVES, VIEW, ARMS,
  PANELS); each row is the live key, the control name and a ≤19-char gloss;
  the footer carries the sentence. Fixed keys (Esc, Tab, F1, LMB, RMB, + −)
  are listed with the binds. Tested: every axis and named bind appears exactly
  once; every gloss fits its column; every glyph exists.

### CONTROLS card (`crates/app/src/card.rs`)

Two columns of essentials read from the live bindings; shows when there is no
settings file (first run), at every start if `ui.controls-card = on`, and on
**F1** (now reserved). Any key or a click puts it away and saves the settings
so it does not return unasked. `FARFALL_BENCH_CARD=1` for captures.

### Readout (`crates/app/src/readout.rs`)

Compact two-column numbers, then ALT / VEL / FC / BENCH, then the status line
(landing, hail, hold, strain, arms, haul) **wrapped** to 32 columns over up to
three lines. Stock anchor top-left `(-0.96, 0.94)`, clear of the arch.

### Stock gauges

- `Instrument::Map` (**MINI MAP**, `ui.map`, ON by default): the map pass
  drawn as a small undimmed pane (`map::MINI_HALF_H`) at `map::MINI_ANCHOR`
  re-projected with the head like a dial (`MapLook { half_h, dim }`,
  `map.right.w` gates the dim in the shader). M still opens the full map.
- `holo.view` defaults ON, size 0.24, anchor `(0.52, -0.46)` right of the
  cluster.

### Hologram range and marks (`crates/render/src/holo.rs`, `shaders/holo.wgsl`)

- `holo.range` 1..4 (HOLO RANGE row; `,` / `.` = HOLO WIDER / HOLO CLOSER,
  rebindable; the wheel in flight). The ship SDF is evaluated at `q * range`
  so the ship shrinks and the scene round it stands for `1500 m × range`.
- Up to 8 marks (`HoloScene::marks`): revealed mimics relative to the ship,
  placed at `distance / reach` toward the rim (pinned on the rim beyond it),
  drawn as small octahedra — red and beating for hostile, amber for a hail,
  grey for a wreck — each on a hair-thin tether to the ship.

## Verification

_Pending: the bench harness was HALTED (locks/HALT) when this branch reached
its first build. Captures to take in `farfall-captures/hud-menu/`:_
`cockpit`, `card` (`FARFALL_BENCH_CARD=1`), `menu-0..7`
(`FARFALL_BENCH_MENU=n`), `land` (`FARFALL_BENCH_LAND=1`), `mimic`
(`FARFALL_BENCH_MIMIC=hostile`), `full` (`FARFALL_BENCH_FULL=1`).

Gate: `cargo test --workspace`, `fmt --check`, `clippy -D warnings`, wasm
check — all green at every commit on the branch.

## Decisions

- **Fractional HUD scale** over integer: the box filter makes non-integer
  scales clean, and a constant NDC size is what lets one test prove the fit on
  every screen. SPEC P1's "no subpixel drift" is kept — the bits are still the
  truth; only the resampling is exact instead of nearest.
- **MAP and SHIP as tabs**: the brief asked for every tab visible; the old
  menu only had five (the audit's "cut off after ARMS" was the header being
  exactly full). Giving the plan and the bay look pages of their own lets GFX
  breathe and puts every settings key somewhere findable. ENGAGE on the MAP
  page fires the drive like the DRIVE panel does.
- **Descriptions in the footer, not per row**: at 48 columns a per-row
  sentence does not fit; the HELP row carries a gloss and the footer the
  sentence, which also keeps the KEYS page one line per bind.
- **The card dismissal saves the settings**: "no settings file" is the
  first-run test, so dismissing must create the file or the card would show
  every launch. `ui.controls-card` (default off) is the way to have it back at
  every start; F1 any time.
- `settings::KEYS`/`DRAGGED_KEYS`/`key_matches` are `#[cfg(test)]`: they are
  the ledger the coverage test needs and nothing at runtime reads them.

## Left

- The scene tests (`crates/app/tests/scenes.rs`, opt-in) measure regions for
  the menu card and the hologram that may shift with the new layout; re-run
  with `FARFALL_SCENE_TESTS=1` once the bench is free and retune the regions.
- Dial digits (gauge.rs) still draw their own segment numerals — the other
  agent's scope.
