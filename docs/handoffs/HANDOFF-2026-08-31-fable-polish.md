# HANDOFF — fable/polish — 2026-08-31

Paste into a fresh session: `Read HANDOFF.md at C:\Users\jayja\farfall-wt\polish and continue from "Next steps".`

## Situation

FARFALL's polish pass (Jay Jay's brief of 2026-08-31 morning: "make every piece more efficient and cleaner /
higher definition … HDR and surreal … make sure the HUD isn't cutting off and the menus include every feature
and keybind … nice to look at, to market it"). Eight parallel agents, one branch each, all merged into
**`fable/polish`** (integration branch, worktree `C:\Users\jayja\farfall-wt\polish`) at `51eafe2` — gate green
(fmt, clippy -D warnings, tests, wasm check). 35 commits since the m1-planet base `84ad5cb`. The merged
build runs; a 44-capture sweep is in `C:\Users\jayja\farfall-captures\devlog\2026-08-31-1331\index.html`.

## Start here

```sh
export PATH="$PATH:/mnt/c/Users/jayja/.cargo/bin"       # WSL; Windows cargo.exe (1.97.1 gnu; mingw64 on PATH)
Q=/mnt/c/Users/jayja/farfall-captures/cargo-q.sh          # build queue: 1 slot x 4 jobs (Jay Jay caps CPU at 50%)
$Q test --workspace; $Q clippy --workspace --all-targets -- -D warnings; $Q check --workspace --target wasm32-unknown-unknown
$Q build --release -p farfall-app                         # -> target/release/farfall.exe
B=/mnt/c/Users/jayja/farfall-captures/bench.sh            # captures: born unfocused off-screen, BENCH virtual desktop, 1 core, stamped
$B "$PWD/target/release/farfall.exe" /mnt/c/Users/jayja/farfall-captures/<dir> <name> [FARFALL_X=Y ...]
/mnt/c/Users/jayja/farfall-captures/bench-matrix.sh <exe> <outdir> quick   # the 44-capture feature sweep, 4 at a time
```
Never launch farfall.exe any other way while Jay Jay is at the PC (it steals focus). `locks/HALT` stops all benches.
`FARFALL_SPAWN=belt|uranus|moon|sun` starts the live game where the drive would land. Knob list: top of
`crates/app/src/lib.rs`. Machine rules: `~/.claude/projects/-mnt-c-Users-jayja/memory/feedback_machine_etiquette.md`.

## Branches & worktrees (`git worktree list`)

| branch | state | do |
|---|---|---|
| fable/polish | integration, HEAD 51eafe2, green | keep; merge tuning/hotas into it; eventually PR to m1-planet |
| fable/tuning | agent running (critic list) | merge when it reports |
| fable/hotas | agent running (T.Flight HOTAS 4 + STICK wizard); nothing committed yet | merge when it reports |
| fable/gauges, postfx, ships, bench-window, particles, hud-menu, landing, world | merged | delete after PR |
| fable/resume | ANOTHER session's branch (world save/resume); merged polish@985cac6 into itself | leave to that session |
| m1-planet (main repo `C:\Users\jayja\farfall`) | Jay Jay's working copy; its WIP is in afb83c1 | untouched |

## Done (observable, with commits)

- HDR picture: float world target, bloom, auto-exposure, AgX, rim fringe, dither; GFX rows BLOOM/EXPOSURE/TONEMAP/FRINGE; stars as points; drive effects outside the glass; shake 40→12 % — `e447fa2`
- WARTHOG default, no wells/cavities, AA strokes, ladder numerals, HDR hoops — `ba8f71d`
- Mimics at 90/320 m + MIMIC SIZE; miner ships (seek/mine/grow four tiers, real haul) — `3f4deb8` `6241b8f`
- Space dust pass, plumes with diamonds/RCS, liquid ghost, honeycomb shield; three baseline bench bugs — `d8246d7`; dust two-pipeline fix — `bec49c0`
- 5×7 font, eight-tab menu card (scroll, ROW n/m, descriptions, HELP), CONTROLS card (F1), wrapping readout, mini map + holo3PP stock, hologram range + contact marks — `ac55148`…`7cc21d6`
- LANDED sim state (golden hash unchanged), DISEMBARK (I), landing pad, LANDING ASSIST/PAD — `aef03b8` `19884c2`
- Ring-plane seam fixed (it was Uranus' ring, not the nebula); scattering atmosphere, terrain, clouds, city lights, 4096² nebula — `5e4a65d` `3d33f5b` `280b2b9`
- Bench mode deaf + unfocused window + FARFALL_WINDOW_POS/BENCH_SIZE — `16e8639` `b957c1f`; FARFALL_SPAWN — `6794851`
- Docs: `docs/polish/` (PLAN, AGENT-BRIEF, ARCHITECTURE, one .md per agent)

## In flight

- `tuning`: MERGED (`74347db`, gate green; scene suite 18/18 now runs on Windows). Its open items: in-air dust streaks, the plasma veil at orbital speed in thick air, hoops mid-view low down, thick limb glow.
- `hotas` agent: reconciling — it is merging fable/polish INTO fable/hotas itself (its STICK page predates the 8-tab card menu), then gate + captures + report; merge its branch when it lands. Stick facts: winmm via windows-sys, T.Flight HOTAS 4 detected (044F:B67C), wizard + STICK page built (`2526e46`).
- Devlog iteration 2: `farfall-captures/devlog/2026-08-31-1452` (44 frames on `74347db`); `devlog/latest.html` self-refreshes and follows iterations. The bench harness now needs `bench-mover.ps1` running (an independent daemon moves BENCHMARK-titled windows to the BENCH desktop — the desktop manager refuses the launcher's own tree; -TitleMatch guards the live game). Public skill/agent repo: github.com/Detcader101/claude-devlog.

## Decisions & why

- One integration branch, one worktree per agent, merge in order gauges→postfx→ships→bench-window→particles→hud-menu→landing→world; every merge through the full gate. Conflicts were all mechanical (shared lists in lib.rs/menu.rs/settings.rs/features.yaml) — resolved by union.
- Horizon line draws first in the ship pass (dash hides it, never blooms). Dust: one pipeline per target format.
- Devlog pages are NEVER committed to this repo (Jay Jay); they live in `farfall-captures/devlog/`. The devlog skill/agent is public: github.com/Detcader101/claude-devlog.
- `FARFALL_FOV` bench knob does not exist yet (the matrix's two FOV captures are at the default).

## Known problems

- See the devlog's critic list (tuning agent has it). Perf lines need ≥6 s benches (default now 6).
- `crates/render/src/sight.rs` was unformatted on the base; fmt now fixes it in the gate.
- Ship's chase-view perf at 2880×1800 not re-measured since world merged (1080p desk; use `FARFALL_BENCH_SIZE=2880,1800`).

## Next steps (ranked)

1. Merge `fable/tuning` and `fable/hotas` through `farfall-captures/merge-branch.sh <name>`; fix conflicts by union as before.
2. Run `bench-matrix.sh <exe> <out> quick` on the result; `/devlog` (pages outside the repo; open once).
3. Re-measure 2880×1800 (`FARFALL_BENCH_FULL=1 FARFALL_VSYNC=off FARFALL_BENCH_SIZE=2880,1800`) at alt 500, belt, hyper — hold the 60 fps floor.
4. Add `FARFALL_FOV`; regenerate scene goldens; update README's controls paragraph for DISEMBARK/HOLO RANGE/F1.
5. PR `fable/polish` → `m1-planet` (CI: macOS + Linux gates), then delete merged branches/worktrees.
6. Open feature asks from Jay Jay still untouched: "make features simulated" applied to mimic/miner only so far; disembark walk-out is a stub.

## Evidence

`C:\Users\jayja\farfall-captures\`: `BASELINE.md` (baseline numbers + critique), `baseline/`, `fullspin/`, one dir per agent,
`devlog/2026-08-31-1331/` (44 stamped frames + index.html). Baseline: 1.5 ms/frame at 1080p; merged: ~1.8–2.4 ms.

## User context

Jay Jay (Detcader101). Rules given today: CPU never above 50 %; benches never take focus (BENCH desktop, one tab for the devlog); stop agents calmly near the 5-hour limit; devlog explains asks→changes→proof and the dev process; no devlog commits in the repo. Advisor-partnership working mode; agents-first.
