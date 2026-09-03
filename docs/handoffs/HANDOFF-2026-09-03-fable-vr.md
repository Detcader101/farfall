# HANDOFF — FARFALL native OpenXR VR, day one (2026-09-03)

## 1. Situation

Jay Jay (GitHub Detcader101; the account is shared with his dad Jay/gnutgnut) has a Valve Index
and asked on 2026-09-03 to "port farfall". By the end of the day FARFALL runs natively in the
Index through OpenXR (SteamVR runtime): stereo proven by machine checks and by his eyes ("nice
one"), cockpit-fixed HUD with real depth, Index controllers/hands/laser/grab on a second lane,
a synthetic Index + synthetic controllers so every VR invariant is benched without a headset,
and a benchmark harness that sweeps it. Two headset passes found and fixed the comfort bugs.
His closing ask: **"need 144 hz and high res so we need to get thinking creatively"** —
docs/PLAN-VR-PERF.md, lane fable/vr-perf, item 1 (measurement) landed. Nothing is merged into
fable/polish yet; merge order and GitHub decisions are his (§6, §8).

## 2. Start here

- Build ONLY via `/mnt/c/Users/jayja/farfall-captures/cargo-q.sh <cargo args>` (flock'd
  cargo.exe, 4 jobs, Windows GNU toolchain; `C:\Users\jayja\mingw64\bin` on PATH). Gates:
  `cargo-q.sh fmt --all --check && cargo-q.sh clippy --workspace --all-targets -- -D warnings &&
  cargo-q.sh test --workspace && cargo-q.sh check --workspace --target wasm32-unknown-unknown &&
  cargo-q.sh build --release -p farfall-app`.
- **Never launch the game or SteamVR from an agent.** The only launch paths are the harness
  (`farfall-captures/bench-vr.sh`, `bench-matrix-vr.sh synth|real [quick|full]`) and Jay Jay's
  own pwsh for wearing it: `set FARFALL_VR=1 & set FARFALL_VR_LABEL=1 & set RUST_LOG=info &
  C:\Users\jayja\farfall-captures\live\farfall-vr.exe` (SteamVR must already be running; HOME =
  VR RECENTRE). Staged exes + `.ref` sidecars live in `farfall-captures/live/` (farfall-vr.exe,
  farfall-vr-hands.exe, farfall-vr-perf.exe, farfall-latest.exe = flat launcher).
- Harness needs the mover daemon: `pwsh.exe -NoProfile -File farfall-captures/bench-mover.ps1`
  started via the Monitor tool (persistent), and no `farfall-captures/locks/HALT`.
- Synth row (no headset): `BENCH_VR_EXE=<exe> ./bench-vr.sh <name> --synth --out vr/<dir>
  FARFALL_VR_SCRIPT=still|look|lean|nod|spin [FARFALL_VR_HANDS=synth FARFALL_VR_SCRIPT=
  idle|reach-stick|grab-stick-roll|throttle-push|laser-menu]`. Real row (SteamVR up, headset on
  the desk): `./bench-vr.sh <name> --out vr/<dir> [FARFALL_BENCH_EXIT=drop]`. Pixel tools:
  `farfall-captures/diff.ps1` (halves diff %), `crop.ps1`, `grab-mirror.ps1`.
- Env knobs (README table): FARFALL_VR=1|synth|0, FARFALL_VR_SCALE, FARFALL_VR_LABEL=1,
  FARFALL_VR_MIRROR=pair, FARFALL_VR_SCRIPT, FARFALL_VR_HANDS=real|synth,
  FARFALL_OPENXR_LOADER, FARFALL_BENCH_EXIT=drop. Settings: graphics.vr, graphics.vr-scale,
  graphics.vr-text-scale (default 1.27), vr.hud-distance (1.0 m), vr.hands, vr.beam.

## 3. Branches & worktrees (all pushed to Detcader101/farfall)

| branch | worktree | head | holds | do |
|---|---|---|---|---|
| fable/vr | farfall-wt/vr | a7f198f (+ uncommitted teardown WIP by agent vr-port) | headset lane: xr.rs, comfort fixes, synth Index, self-checks, bench mode | integration lane for VR; merge into fable/polish after §5 lands |
| fable/vr-hands | farfall-wt/vr-hands | 4530fd0 (rebased on 972265e) | Index bindings, SDF hands, laser, grab, haptics, SynthHands | rebase onto fable/vr head, then merge into fable/vr |
| fable/vr-perf | farfall-wt/vr-perf | 33116e9 (on a7f198f) | PLAN-VR-PERF.md + per-pass GPU timings | continue items 2–5; merge into fable/vr |
| fable/editions | farfall-wt/editions | e22aa7a | docs/EDITIONS.md, docs/PLAN-MULTIPLAYER.md | draft PR #2 → fable/polish |
| fable/polish | farfall-wt/polish | 4236e69 | integration branch, 110 ahead of m1-planet | draft PR #1 → m1-planet (merging deploys the web build via Pages — Jay Jay's click) |
| m1-planet | farfall (main clone, DIRTY: another session's edits) | 84ad5cb | default branch on GitHub | don't touch the dirty tree |

GitHub: PRs #1, #2 open as drafts; topics added. Recommended (his decision): delete
origin/{web,staging,cockpit-slots} (fully merged), make `main` the default from m1-planet.
Unmerged local lanes besides these: fable/a10 (2 docs commits), fable/heli-ship (2 commits).

## 4. Done (observable), by commit

- OpenXR/Vulkan/wgpu-hal handshake, LOCAL space = ship frame, two swapchains, crop of the
  symmetric render into each eye's true frustum, windowed NoVsync mirror — be0c16b d421f7f
  9cd3855; VR HEADSET / VR RENDER SCALE menu rows, FARFALL_VR — 9cd3855; VR RECENTRE (HOME).
- Glass overlays follow the headset (were frozen to the mouse look) — e7c1410; overlays at a
  real 1 m glass plane with per-eye parallax, label = small corner glyph — 9cff919 dead452
  a9869ff; dial faces parallax from each eye's seat — 57d5ad4; head-frame eye offset — 185d0b3.
- Uniform-buffer races fixed: label (a7f08fc) and the crop (972265e) — both eyes were being
  drawn/cropped from ONE write. Rule: per-eye buffers for anything written per eye inside one
  encoder before one submit. Cabin refinement cache counts an eye switch as motion — bcd853e.
- Self-checks (exit 9 under bench): eye order by letter shape in the outer corner (+ mirror
  surface check), stereo disparity ≥2 % of pair halves, overlay depth — a9238a5 eb941e6 3b207f7.
- Index match: runtime-only FOV in cockpit/chase/EVA, ≥1:1 crop sampling, refresh logged,
  auto-recentre on first focused (or first located under bench) frame — e69d029 d03eb30 5fd667d.
- FARFALL_VR=synth synthetic Index + head scripts — 7b01603 63056ca; VR bench mode with
  render_ms/1 %/xr_wait/headroom and (fable/vr-perf) pass_ms per eye — 53144b7 70b449e 33116e9.
- Hands lane (fable/vr-hands): 7ac556e bindings, 3e95d9b SDF hands, 0556e61 laser→menu,
  0dffd16 grab stick/throttle → Controls (HOTAS wins), d177214 haptics, c3b22d0/75dcbb7
  SynthHands headless, 5ecb9ad log words, a3ce548 parallax convention + beam fix.
- Docs: SPEC §5.3 (native XR), §5.3b (hands); features.yaml openxr-vr, vr-hands; EDITIONS.md,
  PLAN-MULTIPLAYER.md, PLAN-VR-PERF.md, RESEARCH-VR-OSS.md.

## 5. In flight

- **Teardown access violation on the real path** — fix COMMITTED as d5505ab (field/drop order in RealSession, EyeSwapchain drop logging, Gpu.xr_instance_keepalive so xrDestroyInstance follows vkDestroyDevice, readback buffers always unmapped) but UNPROVEN: SteamVR was closed before the drop row could run. First job: with SteamVR up, `./bench-vr.sh vr-drop2 --out vr/check-d5505ab FARFALL_BENCH_EXIT=drop RUST_LOG=debug` must exit 0; if it still exits 5 the debug drop log shows which drop call dies. (Was: agent vr-port, uncommitted in farfall-wt/vr
  lib.rs + xr.rs): every real SteamVR bench row exits code 5 after "benchmark complete" =
  0xC0000005 truncated — a native crash in the Drop chain. `FARFALL_BENCH_EXIT=drop` row must
  exit 0; the `process::exit(0)` shortcut currently masks it for benches only. Fix = explicit
  shutdown order (wrapped eye textures → swapchains → session → wgpu device → XR instance →
  Entry) with debug log steps. Verify: `./bench-vr.sh vr-drop --out vr/check FARFALL_BENCH_EXIT=drop`.
- fable/vr-hands must be rebased onto fable/vr once the above lands.

## 6. Decisions & why

- One codebase, two editions (native single-player, web WebXR multiplayer) — Jay Jay: "the same
  game and completely crossplatform"; a fork would double every fix.
- VR comfort bugs are severity-one; **no build goes on his head until a labelled pair capture
  has been eyeballed and the self-checks pass** (memory feedback_vr_comfort). He quit the first
  build in a minute: "my eyes went wonky bro please treat the VR more seriously".
- The headset is never the test loop: synthetic Index + synthetic controllers + harness.
- Readout/sight collimated at infinity is acceptable (real HUDs); dials/panes/hands must have
  real depth. Menu and bay hologram stay head-locked (modal UI) — flagged as a look issue.
- Multiplayer: deterministic lockstep on the untouched sim, browser WebRTC host-star, ShedNet
  signalling/relay, native later (PLAN-MULTIPLAYER.md). Decisions left to him: relay hosting,
  player cap (8, default 4), whether native joins, shared belt fight timing.
- Not done on purpose: XR_EXT_hand_tracking pinch; motion-smoothing tricks (not 144 Hz).

## 7. Known problems (numbers)

- 144 Hz not held: real row at SteamVR 150 % (2468×2740/eye) = 30 ms/frame, 31 fps; at
  synth 2016×2240 scale 1.0 = 11.7 ms (auto-scale governs the world to ~65 %). Budget 6.9 ms.
- pass_ms (synth still, 68 % scale): thermal 0.004, cabin 0.39, world 2.10, post 0.71, present
  0.27 per eye (~3.5 ms/eye) vs 11.7 ms frame → ~5 ms/frame OUTSIDE the passes: suspect the
  per-frame blocking `device.poll(wait)` on the VR path (kills CPU/GPU overlap) and the
  uninstrumented composite (crops+labels+mirror+self-check readbacks).
- 1 % lows 20–50 ms on every row (periodic stall; flat baseline noted it too) = judder.
- Launch hang after "Found 6 cooperative matrix configurations", no window, contagious while a
  hung instance lives (6 in a row once); harness kills by pid and retries. Game-side unknown.
- Auto-scale cannot be pinned in a bench (`vr_auto_scale_on = settings.auto_scale || cfg.vr`):
  scale rows measured the governor. Need FARFALL_AUTO_SCALE=0.
- Critic: hands are grey placeholder blobs; throttle-push/idle rows show no hand in the
  800×600 mirror crop (use FARFALL_BENCH_SIZE=1600,1200); settings menu is a flat screen panel
  in VR; looking back shows the hull from inside (no seat back); nod cuts the readout at the
  top; mirror capture is letterboxed to the window, not the swapchain size.
- SteamVR on his box: Index at 144 Hz, ~150 % SS. Advise 90 Hz / 100 % until perf lands.

## 8. Next steps, ranked

1. Land the teardown fix (§5); prove with the drop row exiting 0; restage; rebase vr-hands.
2. fable/vr-perf: add "composite" + "frame" spans, log unaccounted_ms, REMOVE the blocking
   per-frame poll (take render_ms from timestamps), FARFALL_AUTO_SCALE=0; re-measure.
3. PLAN-VR-PERF items 2–5 by the numbers (far world once per frame, foveated density,
   direct asymmetric frustum, per-eye cabin cache); then `bench-matrix-vr.sh real quick` at
   144 Hz with SteamVR 100 % and 150 %.
4. Chase the 1 % low spikes (periodic stall) and the launch hang (what a hung instance holds).
5. Merge: vr-hands → vr → polish (gate-commit.sh / merge-branch.sh in farfall-captures), refresh
   `live/farfall-latest.exe` and the desktop launcher; then Jay Jay clicks PR #1.
6. Designer pass with Jay Jay: hand look, menu in cockpit space, seat back, hud-distance and
   text-scale by eye, larger mirror capture size.
7. VR devlog edition (gen-devlog-vr.sh page exists: devlog/vr-synth-2026-09-03) + real rows.
8. Multiplayer M-net-0 (loopback lockstep) when he says go; GitHub deletions/`main` default.

## 9. Verification evidence

- farfall-captures/vr/check-a9869ff (first real capture: text 10× too big, no labels),
  vr/synth-check (synth-lean/still/grab rows: the mono bug, 0.07 % → 18.65 % pair-halves),
  vr/check-972265e (real stereo 5.79 %), vr/check-a7f198f (drop row, all checks OK, rc=5),
  vr/perf-check (pass_ms), vr/synth-2026-09-03 + devlog/vr-synth-2026-09-03/index.html
  (20/20 synth quick rows, numbers table), devlog/2026-09-03-1653 (flat devlog: 51/51, solo
  330–355 fps = no flat regression vs 293–312 on 2026-09-02).
- Self-check baselines: synth still pair-halves 18.65 %, real desk 5.8–10 %; eye order OK;
  overlay-depth 0.0465 NDC (synth) / 0.0396 (real, IPD 0.0609).

## 10. User context

- Jay Jay is a designer; surface visual choices to him (hand look, HUD sizes, menu placement).
- "treat the VR more seriously" → comfort proof before wearing; "make sure you get a better
  benchmarking setup that mimics a vr setup with controllers and test from there" → synth loop;
  "research open source games … MIT … take code … 6 dof … hand code … interactions … make it a
  full game" → RESEARCH-VR-OSS.md + hands lane; "cleanup the github and make two versions,
  native singleplayer and WebXR multiplayer … server and client state features and host peer to
  peer options" → editions + PLAN-MULTIPLAYER; "do you commit each feature?" → yes, per lane.
- Machine etiquette (memory feedback_machine_etiquette): CPU ≤50 %, deaf benches on the BENCH
  desktop, never steal focus, never launch SteamVR. Session ended 2026-09-03 ~21:10 BST.
