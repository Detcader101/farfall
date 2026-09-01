# Polish pass — design (`fable/design`, 2026-08-31)

Scope: DESIGN mode grown to the whole glass — full dial orientation, the holo3PP /
mini map / readout as design elements, the cockpit as a shareable file — plus the
hyper camera-shake investigation and the bench matrix's coverage rows.
Files: `crates/app/src/{cockpit,settings,shake,menu,lib,hud_file(new)}.rs`,
`crates/render/src/{gauge,attitude,cabin}.rs`,
`shaders/{common,gauge,gvec,gyro,cockpit,guide}.wgsl`,
`farfall-captures/bench-matrix.sh`.

## What a player will notice

1. **A dial can face any way.** Beside the tilt (`,`/`.`), DESIGN mode gets a
   sideways LEAN on `;`/`'` (`ui.<dial>.lean`, ±60°) and an in-plane ROTATE on
   `9`/`0` (`ui.<dial>.rotate`, ±180°); LEAN and ROTATE rows join the DIALS page,
   Backspace still resets everything. On the dash the face plane itself turns
   (`dial_plane_uv` takes the oriented normal; the DIAL/WARTHOG housing follows —
   the socket code now packs tilt and lean in 5° steps) and on the glass the
   hologram foreshortens both ways and its face turns (`dial_glass_uv`, one
   prelude helper shared by gauge/gvec/gyro). The plate and its markings turn
   together, so the needle still reads true. The gyro's geometric ball ignores
   both — a sphere has no face to turn.
2. **Everything on the glass is a design element.** The holo3PP, the mini map
   and the readout are found by DESIGN's pointer like dials — each with its own
   card, `-`/`=` sizing (`holo.size`, `ui.map.size` scales the mini pane,
   `ui.readout.size` scales the readout's text), Backspace to stock, all eight
   anchors ringed on the guide. The mini map stays up in DESIGN mode and drags
   anywhere (`ui.map = on at x,y`). HOLO VIEW/SIZE/RANGE rows moved GFX → DIALS.
3. **SAVE HUD / LOAD HUD.** The whole cockpit as a small self-describing
   `key = value` file, `~/.farfall/huds/hud-<n>.fhud` (header + `hud.version`):
   every `ui.*` key, `holo.*`, the mini map's look — nothing of graphics,
   controls or the world. Sharing = sending the file; LOAD HUD cycles the folder
   plus DEFAULT; SAVE overwrites the one worn last; `FARFALL_HUD=path` wears one
   for a run. First player-to-player artefact — SPEC §11 records the direction.
4. **Hyper shake fixed at the root.** Two real causes found: (a) settings never
   persisted on a plain Windows launch at all — `Settings::path()` read only
   `HOME`, which Explorer never sets, so every run was stock; Jay Jay's live
   build (`farfall` repo, `m1-planet`) still has the old 40% stock, which is the
   "crazy shake" — it ends when polish ships. Path now falls back to
   `USERPROFILE`, and `settings.version` gates a migration: a versionless file
   carrying exactly the old 40% adopts 12%, any other value is kept. (b) While
   the hyper field is up the helmet camera is reined in (`Shake::hyper_damp`):
   deflection scaled 0.45× and capped at a quarter of the free limit — the chaos
   drive still bucks the *ship*, which is the warning.
5. **Matrix coverage.** New rows: `mode-hyper-shake`, `mode-disembark`,
   `gfx-dust`/`gfx-dust-belt` (new frozen-only `FARFALL_BENCH_DUST=k`),
   `design-lean` (a generated `.fhud` staging a leaned+rotated dial and a moved,
   grown mini map, worn in DESIGN mode), `hud-fhud`, `hud-map-drag`.

## Verification (all in `farfall-captures/design/`, looked at)

- `design-lean-1.png` / `hud-fhud-1.png` — the speed dial leaned 30° and rotated
  45° on the dash, markings turned with the plate; G meter tilted 20°; mini map
  at (0.10, 0.55) at 1.5×. `hud-map-drag-1.png` — same dial upright (map-only
  file), mini map at (−0.30, 0.20).
- `design-guide-1.png` — the guide ringing dials + mini map + holo + readout;
  the card reads POINT AT ANY GLASS PIECE: DIALS MAP HOLO READOUT.
- `mode-shake-1.png` vs `mode-hyper-shake-1.png` — identical parked (3°,2°,1°)
  helmet camera; under the field the dash sits near level and reads through the
  streaks. `hyper-shake-spin-1..4.png` — the same all round the cabin.
- `mode-disembark-1.png` — LANDED ON PLANET … DISEMBARK NOT YET on the readout.
- `gfx-dust-1.png` — the dust row at 2×.
- Gate green at every commit: fmt, clippy `-D warnings`, `cargo test
  --workspace`, wasm check; release build captured above (~200 fps avg 800×600).

## Decisions

- **Lean lands wholly on +X.** The tilted dash normal has no x component, so
  `oriented_normal = tilted*cos(lean) + X*sin(lean)` stays unit with no
  quaternion; the WGSL mirror is three lines. Rotation is applied to the dial's
  own (u,v) axes after the plane hit, which is what makes "markings turn with
  the plate" true by construction.
- **Socket code repacked at 5°.** Both angles must ride one exact-integer f32
  lane; 5° housing steps are invisible and the face itself is exact.
- **HUD import starts hud keys from stock.** An omitted key means the stock
  look, not "whatever you had" — a shared file reproduces the *sender's* glass.
- **The menu never touches the disk.** SAVE/LOAD emit `MenuEvent::SaveHud` /
  `LoadHud(pick)`; the app owns the filesystem, so menu tests stay pure.
- **`settings.version` lives in `FILE_ONLY_KEYS`** — a ledgered key with no
  menu row, keeping both coverage tests honest.

## Left

- LOAD HUD shows picks as "HUD n" (list order), not the file's own name — the
  Copy-only `Menu` can't hold strings; fine until someone wants named saves.
- A stranger-named `.fhud` (not `hud-<n>`) loads fine but SAVE never overwrites
  it (slot 0 → next free number). Deliberate: never clobber a shared file.
- The readout's drag hit-box in `begin_drag` doesn't grow with
  `ui.readout.size` (fixed generous box); worth unifying if it ever feels off.
