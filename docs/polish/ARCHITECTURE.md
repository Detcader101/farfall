# FARFALL render pipeline — engineer's brief (2026-08-31, at commit ea42028)

Line numbers are for `fable/polish` at the time of writing; use them as search anchors.

## 1. Frame pipeline order

Entry: the per-frame body around `crates/app/src/lib.rs:4320` (`surface.get_current_texture()`) through `queue.present` at `crates/app/src/lib.rs:4886`. Everything below is inside a per-eye loop (`eyes`/`eye`, VR side-by-side; flat = 1 eye).

Uniform upload for every pass happens **before** the encoder (`lib.rs:~4400-4720`), then:

| # | Pass | Target | Where |
|---|---|---|---|
| 0 | `thermal.step` — 64×64 hull heat field | own texture | `lib.rs:4726-4728` |
| 0b | `cabin` — SDF-marched cockpit interior | own offscreen (see §7) | `lib.rs:4730-4737` |
| 1 | **"scene"** render pass | `SceneTarget` (scaled) | `lib.rs:4738-4813` |
| 2 | **"present"** render pass | swapchain view | `lib.rs:4814-4849` |

**Scene pass draw order** (`lib.rs:4747-4812`), each gated by `cfg.draws(name)` (`lib.rs:439`, driven by `FARFALL_SKIP`): starfield → bodies → planet → belt → mimic → scar → debris → tracer → jet → plasma (skipped in chase) → trajectory → shield → ghost → cabin (skipped in chase) → gauge block (horizon, gauge, alt_gauge, g_gauge, gvec, gyro — each `draw_within` a scissor rect from `gpu.dial_rects`, then guide, sight) → holo. **No depth buffer** (`depth_stencil_attachment: None`) — order *is* the depth. Blending is additive for instruments (`instrument.rs:37-45`).

**Present pass order** (`lib.rs:4838-4848`): blit → map (if open) → hologram (if bay open) → hud → pointer. `LoadOp::Clear` for eye 0, `Load` for eye 1 (`lib.rs:4826-4831`); viewport set per eye at `lib.rs:4846`.

**Formats.** The scene target is **not HDR float**. `SceneTarget::new(cfg.msaa, config.format, cfg.scale)` (`lib.rs:3865`) — `config.format` comes from `surface.get_default_config(...)` (`lib.rs:3722`), i.e. the swapchain's own 8-bit sRGB format (typically `Bgra8UnormSrgb`; the capture path checks for exactly that at `lib.rs:4894`). **Tonemapping is per-shader, in the scene pass**, via `tonemap(col, exposure) = 1 - exp(-col*exposure)` in `shaders/common.wgsl:125`, with `dither_px()` (`common.wgsl:130`, ±0.5/255) to kill 8-bit banding. Shaders write **linear**; the sRGB target format does the encode. A new scene pass must tonemap itself — nothing downstream will.

**MSAA** lives only in the scene target: `SceneTarget::colour_attachment()` (`render/src/lib.rs:312`) renders into a `sample_count`-sampled texture with `resolve_target: Some(colour_view)` and `StoreOp::Discard`; the resolved single-sample `colour` texture (`TEXTURE_BINDING|COPY_SRC`, `render/src/lib.rs:288-292`) is what the blit samples. `MsaaTarget` (`render/src/lib.rs:129`) is the older direct-to-swapchain variant — still present, not on the main path.

**The post/blit pass** (`crates/render/src/blit.rs`, `shaders/blit.wgsl`) is **upscale + wormhole FX only — no tonemap, no colour grade**. One fetch per output pixel with a linear sampler; `PostUniforms` (`blit.rs:6-11`) carries `fx = [fisheye, invert, flow, charge]` and `misc = [aspect, time_s, speed, _]`, all clamped 0..1. Every effect term multiplies by zero when the drive is idle (`blit.wgsl:4-6`). This is where render-scale upscaling happens and **the reason HUD/map/hologram/pointer are constructed with `sample_count = 1` against `config.format`** (`lib.rs:3884-3894`) — they draw after the upscale, at native res, so text stays sharp at any scale (`lib.rs:3882-3885`, `blit.wgsl:8-11`).

## 2. Anatomy of a pass

Most small passes are **not bespoke** — they are `InstrumentPass` (`crates/render/src/instrument.rs`) with a different shader and a different uniform block. Canonical example, `crates/render/src/shield.rs`:

- Domain constants + pure helpers first (`SHELL_CENTRE`/`SHELL_RADIUS_M`/`RIPPLE_MPS`, `shield.rs:26-30`; cf. `ghost_distance_m`/`ghost_fade`, `ghost.rs:14-26`) — testable without a GPU.
- `#[repr(C)] #[derive(bytemuck::Pod, bytemuck::Zeroable)]` uniform struct of **vec4 rows only**, std140-safe (`shield.rs:31-43`).
  Camera basis is packed into the `w` lanes: `right.w = aspect`, `up.w = tan(fov/2)`, `fwd.w = time` (`shield.rs:71-74`) — the house idiom.
- A `new(cam, head, time_s, …)` constructor that clamps/sanitises every float (`is_finite()` guards, `rem_euclid` clock wrap), plus chained `with_*` builders (`with_eye` for the chase seat `shield.rs:82-88`, `with_hyper` `shield.rs:92`). Same shape as `FrameUniforms::with_occluder`/`with_star_stretch` (`render/src/lib.rs:92,105`).
- `pub type ShieldPass = InstrumentPass;` + a free `shield_pass(device, target_format, sample_count)` calling `InstrumentPass::new_sized(...)` (`shield.rs:101-116`).
- Tests assert *packing* and *geometry mirroring the shader's maths* (`shield.rs:119-176`), never constants.

`InstrumentPass` variants: `new` (additive, 64-byte block), `new_sized` (additive, bigger block), `new_pane`/`new_pane_sized` (premultiplied over — the map), `new_layer_sized` (no blend, for render-to-texture). API: `update<T: Pod>(&queue, &T)` (`instrument.rs:253`), `draw(&mut pass)` (`:265`), `draw_within(&mut pass, rect, full_size)` which sets a scissor and restores it (`:276-290`). `UNIFORM_BYTES = 64` (`instrument.rs:27`).

**Shader registration.** WGSL has no `#include`; `shaders::compose(src)` prepends `common.wgsl` (`crates/render/src/shaders.rs:85-87`). A new shader needs: a `pub const` `include_str!` (`shaders.rs:7-36`) **and** an entry in `PASSES: &[(name, src, &[entry_points])]` (`shaders.rs:39-83`). Validation, GPU-free, `crates/render/tests/shader_validation.rs`: `every_pass_compiles_with_the_prelude` (naga parse+validate, entry points present, `:24`), `the_prelude_is_self_contained` (`:39`), `passes_do_not_shadow_prelude_helpers` (`:46` — no local `hash31/vnoise/fbm3/fbm5/tonemap`), `every_shader_file_on_disk_is_registered` (`:68` — you cannot leave a shader off `PASSES`).

Prelude helpers you should reuse rather than reinvent (`shaders/common.wgsl`): `fullscreen_ndc` (`:146`), `view_ray` (`:138`), `tonemap` (`:125`), `dither_px` (`:130`), `canopy`/`canopy_inverse`/`canopy_glass` (`:165/:179/:192`, `CANOPY_R = 1.55` at `:153`), `oct_encode/decode` (`:206`), `quat_rotate` (`:223`), SDF primitives + `sd_fighter_hull` (`:251-333`), dial helpers `dial_plane_uv`/`DIAL_DASH_C`/`DIAL_DASH_N` (`:335-367`), 7-seg `digit_dist`/`digit_mask` (`:381/:401`).

## 3. Camera / frame data

`CameraFrame` (`render/src/lib.rs:43-56`): `orient: Quat`, `fov_y`, `aspect`, `time_s` (animation only, **never gameplay**), `exposure`. `basis()` → `(X, Y, NEG_Z)` (`:60`). `projection()` is reverse-Z infinite perspective, near 0.05 (`:72`) — used by geometry passes, not the raycasting sky passes.

`FrameUniforms` (`render/src/lib.rs:78-89`), five vec4s: `right`, `up`, `forward`, `params = [tan(fov/2), aspect, time_s, exposure]`, `occluder = [centre_rel.xyz, radius]`. Built with `from_camera` (`:111`); `right.w` doubles as star-stretch (`with_star_stretch`, `:92`). The occluder is a pure optimisation (sky pixels behind the planet), image-identical without it (`:106-108`).

**Not present**: velocity, ship state, health. Passes that need those define their own uniform block and the app packs it (e.g. `game.planet_uniforms(&pose)`, `game.bodies_uniforms(...)`, `game.cabin_uniforms(&cam)`).

`ViewPose` (`app/src/lib.rs:3558-3565`): `cam: CameraFrame`, `head: Quat` (view rotation *relative to the ship* — the argument every ship-frame pass takes), `eye_ship: DVec3` (seat offset; ZERO in cockpit, `CHASE_EYE_SHIP = (0, 3.2, 24)` at `:3616`, VR eye offset otherwise). Built by `Game::pose(aspect)` (`:3482`). **Rendering is camera-relative**: f64 world positions are subtracted against the camera *by the app* in f64; only camera-relative f32 crosses into `farfall-render` (`render/src/lib.rs:4-7`).

## 4. HUD & text

`TextBitmap` (`crates/render/src/text.rs`) is a 1-bit packed bitmap, **not a texture**: `GLYPH_W=3, GLYPH_H=5, ADVANCE=4` (`:12-15`), `COLS=128, ROWS=96` (`:23-24`), stored as `rows: [[u32; 4]; 96]` (`:26`). API: `clear()` (`:103`), `draw(x, y, text) -> usize` (`:121`), `used_extent()` (`:144`). `set()` **silently drops** out-of-bounds writes (`:107-113`) — a line at row `y` needs `y + GLYPH_H` rows.

Upload path: the whole `rows` array is embedded in `HudUniforms` as `array<vec4<u32>, 96>` and pushed with `queue.write_buffer` in `HudPass::update` (`crates/render/src/hud.rs:141-143`). `hud.wgsl:56-62` reads bits directly — no sampler, no mipmaps. Blend `ALPHA_BLENDING`, no depth (`hud.rs:91-96`).

`HudUniforms` (`hud.rs:12-26`): `a = [anchor_ndc.x, anchor_ndc.y, px_canopy, aspect]`, `extent = [used_w, used_h, height_px, highlight_row_y]`, `color` (fixed cyan `hud.rs:132`), `backdrop` (flat vs glass, `hud.rs:135-139`), `sway = [sway.x, sway.y, flat?1:0, highlight_row_h]`, `rows`.

**Layout.** The panel self-sizes to `bitmap.used_extent()` (`hud.rs:126-130`). Font-pixel local coords = `(p - anchor) / px` (`hud.wgsl:84-85`); `pad = 3.0` font-pixels of safe edge before `discard` (`hud.wgsl:89-93`). Scale: `hud_scale = (screen_h/260).clamp(2,8).floor()`, `px_canopy = hud_scale * 2/screen_h * text_fov_scale()` (`lib.rs:4431`, `4676`, `4700`); `text_fov_scale` clamps 0.4..1.25 and is forced to 1.0 for menu/map (`lib.rs:1887-1893`). Anchors go through `on_glass()` (`lib.rs:200`) — the canopy sphere reprojection — unless a pane is open, in which case the panel is screen-fixed (`text_screen_anchor`, `lib.rs:1830`). Drag persistence: `Dragged` enum (`lib.rs:302`), `end_drag` writes `settings.readout_anchor` (`lib.rs:3461`).

**One bitmap, one writer per frame.** At the tail of the frame (`lib.rs:4936-4960`) exactly one of: design text / `map_panel.render` / `render_bay_card` (`lib.rs:1526`) / `menu.render`. Otherwise `frame_timing()` (`lib.rs:988-1080`) owns it, throttled to 4 Hz (`:1020`), rows at a 6-px pitch: FPS, 1% low, CPU ms, WAIT ms, MSAA+scale%, resolution, ALT, VEL, FC, BENCH, then landing/hold/mimic/strain/arms/haul.

**Menu** (`crates/app/src/menu.rs`) writes into the *same* bitmap: `VISIBLE_ITEMS=8, ROW_PX=6, COLS=32` (`:406-409`; 32 glyph cells × 4px advance = 128 = `text::COLS`). `render()` at `:1275`: header y=0, items at `(row+1)*ROW_PX`, footer at y=54, scroll marks at col 124. `line()` (`:1243`) pads every row to exactly `COLS`. The test `every_row_of_every_page_fits_the_panel` (`menu.rs:1445-1476`) walks default + `widest_settings()` fixtures × `{Menu::new, map_panel, ship_panel}` × all `Page::ALL`, asserting `header().len() <= COLS`, `footer().len() <= COLS`, `line().len() == COLS` exactly for selected and unselected rows, and that label and value never abut. **Any new menu row must fit 32 columns at its longest value.**

## 5. Adding a setting (traced: `vsync`)

`crates/app/src/settings.rs` — hand-written `key = value`, no serde. Path `~/.farfall/settings.cfg` (`:406`), wasm → `localStorage` (`:412-417`). Six steps:

1. Field on `Settings` (`:190-318`) — `pub vsync: bool` at `settings.rs:197`
2. Default in `impl Default` (`:333-401`) — `vsync: true` at `settings.rs:339`
3. Parse arm in `Settings::parse`'s `match k` (`:453-931`), clamping inline — `settings.rs:479`
4. Serialise push in `Settings::render` (`:933-1125`) — `settings.rs:937-940`
5. Menu: label (`menu.rs:172`), value display (`:243`), page membership (`:464`), adjust handler (`:735-738`)
6. Use site — `lib.rs:451-454` (env override `FARFALL_VSYNC`), live apply `lib.rs:907-909`, initial `lib.rs:3724`

**Trap:** `edits_round_trip` (`settings.rs:1157-1237`) is a *manually maintained* struct literal. There is no automatic all-keys test — forget step 7 (add your field to that literal) and a missing parse or render arm passes CI silently. Unknown keys and unparsable values are silently ignored (`:855`).

## 6. Input / binds

`crates/app/src/input.rs`. Two tables: `Action` (12 continuous axes, `:20-33`) and `Named` (23 held/toggle controls, `:117-142`), stored as `Bindings { keys: [KeyCode; Action::COUNT], named: [KeyCode; Named::COUNT] }` (`:282-285`).

Every `Named` variant: `Boost, Brake, Despin, Hyper, WarpStop, Map, Appearance, Engage, Landing, Design, LookLock, Trajectory, Assist, Chase, Holo, Capture, Bay, NextWeapon, Weapon1, Weapon2, ScaleDown, ScaleUp, Hold`.

Defaults: `Named::default_key` (`:232-259`), axis `BINDINGS` table (`:264-277`). `KEY_NAMES: &[(&str, KeyCode)]` (`:372-472`) maps **physical positions** to display labels ("A", "LSHIFT", "NUM+") because binds are layout-agnostic (`:12`); helpers `key_name` (`:474`), `key_from_name` (`:482`). Rebinding swaps rather than duplicates (`bind` `:317`, `bind_named` `:344`); `is_reserved` (`:363`) refuses Escape/Enter/Tab/Backspace. Persisted as `control.<key>` (`settings.rs:833-846`, `945-958`). The KEYS page is built at `menu.rs:489-495` (`Action::ALL` + `Named::ALL` + `LookSens`).

Guards: `no_two_default_binds_share_a_key` (`input.rs:648`), `rebinding_swaps_uniformly_...` (`:673`), `bindings_are_complete_and_unique` (`:902`, also asserts `AXES` order matches enum discriminant order — a determinism contract), `menu.rs:1536` (every action has a row). **Nothing checks `KEY_NAMES` uniqueness or round-trip.**

## 7. Quality governors

**Global auto-scale** (`app/src/lib.rs:951-991`, `govern_scale`, called once per frame at `:4927`): only the `SceneTarget` is scaled — HUD/dials/text stay native (`:946-950`). `AUTO_SCALE_MIN=0.35` (`:190`), `AUTO_SCALE_STEP_S=0.75` (`:192`), `AUTO_SCALE_RAISE_S=3.0` (`:194`). Miss (`fps < floor-3`): `scale *= (fps/floor).clamp(0.5,1).sqrt().max(0.85)`. Recover after 3 s on-floor with 1% low ≥ 80% of floor: `*= 1.08`, capped 1.0. Effective = `(settings.scale * auto_scale).clamp(0.35, 1.0)` (`scale_target`, `:937`). Target is user-set `settings.fps_floor` (default 60, `settings.rs:351`).

**Cabin** (`crates/render/src/cabin.rs`) governs itself independently. Two targets: a sharp `still` at `cockpit_res` (default 0.5) redrawn one of `STRIPS=4` scissor strips per frame while resting (`:411`), and a `moving` target at `fraction * governor.scale` re-marched whole while the head turns (`MOVING_SCALE=0.6` `:413`, floor `MOVING_SCALE_MIN=0.3` `:351`). `Governor::step` (`:363-397`): EMA `0.85/0.15`, 12-frame settle, `budget_ms = 1000/floor_fps`; down 0.05 above 1.06× budget, up 0.05 below 0.7× after 90 frames. Driven from `CabinPass::govern` (`:612`), called at `lib.rs:1017`. Composited by `shaders/cabin_blit.wgsl` (premultiplied, linear upscale, one triangle).

**Cost classes** are declared in every shader header (`// Lane: X. Cost class: …`). The dear ones: `cockpit.wgsl:3` and `map.wgsl:3` ("a short SDF march per pixel"), `hologram.wgsl:4`/`holo.wgsl:3`/`jet.wgsl:3` (bounded march), `bake.wgsl:5` and `nebula.wgsl:4` (one-off startup bakes). `lib.rs:4738` labels the scene pass "the expensive world". `render/src/lib.rs:203-205`: pixel count is the single biggest lever, bigger than any individual effect. Telemetry: `crates/app/src/telemetry.rs` `FrameStats` (120-frame rolling), `smoothed_fps` (mean of frame *times*, `:79`), `recent_low_1pct_fps` (`:132`), `skip_next_frame()` after resize/capture; readout in the window title (`lib.rs:1022-1049`) and a log line every `PERF_LOG_EVERY = 5 s` (`lib.rs:187`, `:1086`).

## 8. Things that will bite

- **No HDR.** Scene target is the swapchain's 8-bit sRGB format. Write linear, call `tonemap()` yourself, `dither_px()` if you have smooth gradients. There is no post-tonemap stage. (The `postfx` workstream is changing exactly this — after it merges, emitters write radiance and the post pass tonemaps.)
- **No depth buffer anywhere.** Sorting is draw order. Don't assume occlusion.
- **MSAA is scene-only and rebuilds targets.** `sample_count` is a *constructor* argument on every pass; changing MSAA at runtime rebuilds the whole `Passes` struct (`lib.rs:917-929`, comment at `:547`). A new scene pass must take and honour `sample_count`; a present-pass must use `1`. Unsupported sample counts are a validation *panic* at pipeline creation, hence the adapter probe at `lib.rs:3737-3746`.
- **Web = WebGPU, not WebGL2.** No `webgl` feature on wgpu; `required_limits: wgpu::Limits::default()` (`lib.rs:3713`), *not* downlevel — so no WebGL2 fallback exists and none of the downlevel restrictions apply, but device creation is async (`web.rs:22-24`, `App::pick_up_pending` `lib.rs:3816`). Screenshot/capture is native-only (`lib.rs:5077`). VR is driven by the page's JS XR loop into `xr_frame` (`web.rs:150-179`) with a `w*2 × h` swapchain. Always `cargo check --workspace --target wasm32-unknown-unknown`.
- **Determinism.** `farfall-sim` is the only authority on world state; render is a view over interpolated snapshots. `cam.time_s` is for animation only. Never let a render/HUD change touch sim code — the golden-hash CI gate will catch it, but fix the leak, never the constant.
- **`canopy()` is mandatory** for anything on the glass — a pass with its own curvature desynchronises the cockpit (`common.wgsl:165` comment). Use `canopy_inverse` + `draw_within` scissoring rather than a fullscreen triangle that discards 97% of itself (the gauges cost ~1 ms/frame before this, `common.wgsl:179-183`).
- **Body frame is +X right, +Y up, −Z forward.** "+Z forward" is a silent mirror.
- **`TextBitmap` clips silently** — off-panel rows just vanish, no panic, no test failure unless you wrote one.
- **A shader file not in `PASSES` still compiles the build but is never validated** — the `every_shader_file_on_disk_is_registered` test exists because that shipped once.

**Adding a pass, checklist:** `shaders/foo.wgsl` (with `// Lane: … Cost class: …` header) → `pub const FOO` + `PASSES` entry in `shaders.rs` → `crates/render/src/foo.rs` with `#[repr(C)] Pod` vec4-row uniforms + `foo_pass(device, format, sample_count)` → `pub mod foo` in `render/src/lib.rs` → field in `Passes` (`app/src/lib.rs:596-621`) → `update()` before the encoder, `draw()` in the right slot of the scene or present pass with a `cfg.draws("foo")` gate → unit test on the uniform packing → a setting + menu row if it's user-visible.
