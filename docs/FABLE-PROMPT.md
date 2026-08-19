# Prompt for Fable — paste as a single message after `/model claude-fable-5`

> Two notes before you paste (from Opus, not part of the prompt):
>
> 1. **The "use a workflow" line is load-bearing.** Claude Code will not spawn
>    multi-agent orchestration unless you explicitly opt in — the phrasing below
>    does that. Without it Fable works solo, which is not what you're paying for.
> 2. **Everything below the line is the prompt.** It's deliberately spec-heavy and
>    light on step-by-step instructions: Fable degrades when over-directed, and does
>    its best work when handed the full problem and the authority to decide.

---

I'm building an open-source, shader-driven 3D space game — roleplay-oriented, where
choices materially change the world, in the lineage of Elite Dangerous' Background
Simulation and EVE Online's player-consequence loop. Targets: Mac and Windows
(flat), and VR on Meta Quest and Valve Index. Rust compiled to WebAssembly, running
in the browser via WebXR, is the chosen direction. The whole project must be
releasable under a permissive licence (MIT, or MIT/Apache-2.0 dual to match the Rust
ecosystem) with no GPL or copyleft code in the tree.

Read `~/space-game/PLAN.md` first. It is a research pass I had Opus do today
(2026-08-19), with every licence and API claim verified against crates.io, docs.rs,
and upstream specs and issues. Take it as a well-researched starting position, not
as settled truth — I want you to attack it.

**Use a workflow with multiple agents for this.** Fan out to verify independently
rather than reasoning it through alone; the claims here are exactly the kind that
look right and are subtly stale.

What I want back is a single implementation-ready technical specification that a
fresh Opus session can start executing immediately, without needing me to fill in
gaps. Specifically, I care about these things and I'd rather you tell me I'm wrong
than agree with me:

**Attack the load-bearing claims.** The plan says Rust → wasm → WebXR is only viable
today through WebGL2, because the WebXR/WebGPU binding is an unratified Editor's
Draft, `web-sys` has no `XrGpuBinding`/`XrProjectionLayer` types, and wgpu's
`ExternalTexture` is video-oriented rather than a way to wrap an XR compositor
texture. If any of that has changed, or if there's a route the research missed —
a maintained crate, a viable fork, a JS-shim pattern that doesn't cost a texture
copy — that single finding reshapes the project and I need to know before I write
code. Verify it adversarially: default to "the optimistic claim is wrong" and make
it prove itself. Confirm the licence of every dependency you propose from the actual
repository, not from memory.

**Resolve the topology question.** The plan proposes one Rust core with three
front-ends (native flat, native OpenXR, web) behind an `XrBackend` trait, and it
hedges on whether this is fundamentally a web game that also runs in VR, or a native
VR game that also has a web build. Decide it. Give me the reasoning, the cost of
being wrong, and the concrete migration path if the WebXR/WebGPU binding ships in
six months. I'd rather have a firm recommendation I can disagree with than a menu.

**Make the frame budget real.** 90 Hz stereo is roughly 9–11 ms for both eyes on
hardware that, for Quest, is a phone. Give me an actual per-pass millisecond budget
for a target frame — starfield, planet, atmosphere, volumetrics, ships, cockpit, UI,
post — for both a Quest-class and a desktop-class GPU, and say which techniques in
the plan don't fit and must be cut or downgraded. Precomputed atmosphere LUTs,
quarter-res volumetrics with temporal reprojection, distance-banded representation,
and floating-origin f64→f32 are the plan's proposals; validate or replace them.

**Design the simulation that makes choices matter.** The plan argues for an
Elite-style Background Simulation — deterministic faction influence values driving
system states — running single-player and offline first, with authored Yarn Spinner
narrative reading and writing those values, and a later path to shared server state.
I want the actual design: the state a faction holds, the tick function, how player
actions propagate, how you keep it from collapsing into a single dominant faction or
into noise, and how you test it (I want to fast-forward a thousand ticks in CI and
assert the economy is still coherent). This is the part most likely to be
hand-waved, and it's the part that determines whether this is a game or a tech demo.

**Define the vertical slice.** One 20-minute experience — a specific star system,
a specific set of factions, a specific story arc with real divergent endings —
playable flat and in VR. Be concrete enough that "done" is unambiguous.

**Sequence the work.** The plan opens with six de-risking spikes (R1–R6) before any
game code. Tell me if that's right, if the ordering is right, and what each spike's
kill criterion should be. Then give me a milestone plan where each milestone has a
demonstrable artifact.

Output as markdown to `~/space-game/SPEC.md`, plus `~/space-game/TASKS.md` holding
the first milestone broken into tasks specific enough for an Opus session to pick up
cold and execute. Where you overrule the plan, say so explicitly and say why — I
want the disagreements visible, not smoothed over. Where you're genuinely uncertain,
mark it as an open question with the experiment that would settle it; don't paper
over a gap with plausible prose.

I'll be handing your output back to Opus to implement, so write for that reader:
precise, decided, and free of choices I'd have to make again later.
