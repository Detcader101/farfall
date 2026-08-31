# Brief for every polish-pass agent (read fully before touching code)

You are one of several parallel agents, each in its own git worktree and branch under
`C:\Users\jayja\farfall-wt\<name>` (WSL path `/mnt/c/Users/jayja/farfall-wt/<name>`),
all branched from `fable/polish`. The orchestrator merges branches; **never merge, rebase
onto, or push anything yourself**. Commit often on your own branch.

Read first: `CLAUDE.md`, `docs/polish/PLAN.md` (art direction — the taste rules), and
`docs/polish/ARCHITECTURE.md` if it exists (render pipeline map). Then `SPEC.md` §5–§8
and `features.yaml`'s header rules. Baseline captures and the visual audit of what is wrong
today: `/mnt/c/Users/jayja/farfall-captures/BASELINE.md` (+ `baseline/*.png`, view them
with the Read tool).

## Toolchain (Windows exe driven from WSL)

```sh
export PATH="$PATH:/mnt/c/Users/jayja/.cargo/bin"      # cargo.exe / rustc.exe (1.97.1, gnu)
cargo.exe build --release -p farfall-app                # → target/release/farfall.exe
cargo.exe test --workspace                              # the gate (~2.5 min)
cargo.exe fmt --all && cargo.exe clippy --workspace --all-targets -- -D warnings
cargo.exe check --workspace --target wasm32-unknown-unknown   # the web lane must not rot
```
Your worktree's `target/release` is pre-seeded, so the first build is ~1–2 min. Use
`cargo.exe` (never plain `cargo`; there is no Linux toolchain). Do not run the exe from
the shared `farfall` repo — only your own worktree's.

## Seeing your work (mandatory — "verified" means you looked at a capture)

```sh
B=/mnt/c/Users/jayja/farfall-captures/bench.sh
$B /mnt/c/Users/jayja/farfall-wt/<name>/target/release/farfall.exe \
   /mnt/c/Users/jayja/farfall-captures/<name> <capture-name> [FARFALL_X=Y ...]
```
It runs a 4 s frozen bench (windowed 800×600), passes any `FARFALL_*` env vars through to
Windows, moves the PNG(s) to `farfall-captures/<name>/<capture-name>-N.png`, and prints the
perf line. Then **Read the PNG** and judge it against PLAN.md. Every `FARFALL_BENCH_*` knob
is documented at the top of `crates/app/src/lib.rs` (~line 360). For a full-res perf number:
`FARFALL_BENCH_FULL=1 FARFALL_VSYNC=off FARFALL_BENCH_SECONDS=8`. The 60 fps floor at
2880×1800 4×MSAA is the budget; report worst-case scenes (alt 500 m, belt, nebula, hyper).
Jay Jay may be playing the game fullscreen on this PC while you work: bench windows pop over
it for 4 s — batch your captures rather than firing dozens one at a time, and never leave a
window open. If you need a knob that doesn't exist, add it (documented in that list).

## Rules that are not negotiable

- Render-only unless your scope says otherwise; the sim's golden hash must not change.
- No image or font assets. Everything drawn is a shader; layout maths is Rust under test.
- Every user-facing feature: a settings key (`crates/app/src/settings.rs`), a menu row
  (`crates/app/src/menu.rs`), a `features.yaml` entry, and a behaviour-named test.
- Keep `crates/app/src/lib.rs` edits small and localised (prefer a new module); other
  agents are editing it too. Never reformat or reorder code you aren't changing.
- Commit style: the repo's — a long descriptive subject line in plain English saying what
  the change *does* for the player. Add the trailers:
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`
- Before you finish: fmt + clippy + tests + wasm check all green; `features.yaml` updated;
  write `docs/polish/<name>.md` (what you did, capture names that prove it, what's left,
  decisions and why), commit it. Your final message to the orchestrator: ≤400 words —
  commits, what a player will notice, verification evidence, open problems.
- Work autonomously. Do not ask questions; decide, document the decision, move on.
