# The multiplayer edition: the same world, flown together

*Plan, 2026-09-03. Two editions of one codebase: native singleplayer (today's
binary, unchanged) and the WebXR multiplayer page (the same page, plus a NET
page in the menu). The ledger (features.yaml) tracks each chunk; this is the shape.*

Jay Jay's ask: "make two versions, native singleplayer and WebXR multiplayer.
WebXR multiplayer is the same game and completely crossplatform with the
addition of server and client state features and host peer to peer options."

## 0. What the code actually is today (the facts the design rests on)

- `crates/sim` is one ship: `WorldState { time_s, ship }`, `step(params, state,
  controls)`, `state_hash` (FNV-1a over the bits). Pure, libm-only, no PRNG, no
  ship–ship anything. Golden hash gated on macOS + Linux (+ Windows by hand).
- The fixed-step body of `Game::tick` (`crates/app/src/lib.rs` ~4407–4573) is the
  real per-tick authority: it composes `step_controls` from the stick, HOLD, the
  landing computer and the chaos jitter (`strike_rng`), calls `sim::step`, then
  the belt, the arms, mimics, miners and EVA add impulses straight onto
  `state.ship.vel_mps`. That glue reads settings mid-loop (`arms_power`,
  `mimics_chance`, `miners_count`, `hold_gain` …), uses std transcendentals
  (belt.rs:121/129, mimic, miner, hold, warp, landing, heli) and `HashSet`/`HashMap`
  state (`belt.wounds`, `belt.dead`, `mimics.revealed`). It is NOT bit-stable
  across machines and was never meant to be (SPEC §6.5a: "the golden hash does
  not know they exist").
- The web build is `cargo check`ed for wasm32 but the golden hash has never been
  run under wasm. The page owns the WebXR session in JS (`web/xr.js`) and calls
  into the module through three `#[wasm_bindgen]` functions.
- The world file (`save.rs`, SPEC §7.6) already serialises the whole world plus the
  app-side ledgers as sealed `key = value` text with `parse(render(w)) == w`.

Consequence: "the sim is the sole authority" is true of `crates/sim`. For a shared
session the authority has to be a *deterministic session layer* around it, and the
app-side glue has to be either made deterministic or kept out of the shared state.
The MVP keeps it out; phase 2 brings it in.

## 1. Netcode model — deterministic lockstep with input delay, no rollback

**Decision: pure lockstep.** Every peer runs the identical sim; the only thing on
the wire per tick is each pilot's *final* controls (plus any impulse its own glue
put on its ship). The golden hash is the desync detector, as SPEC §11 promised.

- **Why not rollback (GGPO-style):** rollback needs a cheap snapshot/restore of
  everything the tick touches. Here that is `Game`'s 8.5k-line ledger (belt live
  set, arms, mimics, miners, EVA), not `WorldState`. A flight sim with tonnes of
  inertia hides 50–70 ms of stick latency completely; a fighting game does not.
  Rollback buys nothing we can feel and costs the biggest refactor available.
- **Input delay:** `delay_ticks = ceil((rtt_max/2 + 15 ms) / DT)`, clamped 3..24,
  negotiated by the host as the max over peers at join and at every resync.
  Europe RTTs of 20–60 ms give 4–8 ticks (33–67 ms). Renegotiated only at pause
  points (join/resync), never mid-flight.
- **Why host-authoritative is wrong here:** it would mean serialising the world at
  a rate over WebRTC and the sim would stop being the authority on every machine.
  It is kept for one thing only: *late join* (§1.3), where the host's world file
  is the truth.

### 1.1 The frame (what a peer authors for tick T)

```
Frame { tick: u32, controls: PackedControls, impulse: Option<DVec3> }
PackedControls { thrust: [i16;3], torque: [i16;3], bits: u8 (assist boost brake despin hyper), hyper_level: u16 }
```
- **The frame carries the computers' output, not the stick.** HOLD, LANDING
  ASSIST and the chaos jitter run on the owner's machine before packing (they
  are local state and a local PRNG); every peer then integrates identical numbers.
  Test: `crates/app/src/lib.rs::a_session_frame_carries_the_computers_final_controls_not_the_sticks`.
- **Quantised axes.** i16 in [-1, 1]. The owner unpacks its own frame before its
  own `sim::step`, so local and remote integrate the same bits. ±1 and 0 are
  exact, so the golden test's controls are untouched.
- **Impulses ride the frame.** The belt shove, the gun kick and a mimic's hit on
  *your* ship are app-side and not shared in the MVP; instead the owner sums the
  tick's impulses and ships them in its *next* frame. All peers (the owner
  included) apply the impulse at that frame's tick. The owner feels its own rock
  strike `delay` ticks (≈ 60 ms) late — imperceptible, and bit-identical everywhere.
- **Packets:** frames go out every 4 ticks (30 Hz) on the unreliable channel,
  each packet carrying the last 8 frames (redundancy covers loss and reorder).
  A gap older than the tail triggers a resend request on the reliable channel.

### 1.2 Stall, clock, drift

- A tick runs only when every peer's frame for it is present; otherwise the
  accumulator waits (render keeps drawing the last snapshot). After 2 s the
  readout says `WAITING FOR <name>`; after 10 s the host drops the peer (§1.4).
- There is no wall-clock sync; tick numbers are the clock. Ping/pong on the
  reliable channel every 2 s measures RTT for `delay_ticks` only.
- Drift: a peer whose accumulator is more than `delay/2` ticks ahead of the
  slowest peer's last frame holds; one behind runs up to 4 ticks per render
  frame to catch up. `Session::ticks_to_run(now_ms, accumulator)` owns this.
  Test: `crates/net/src/session.rs::a_fast_clock_is_held_and_a_slow_one_catches_up`.

### 1.3 Join (the airlock), leave, desync, pause, save

- **Join = a one-second freeze for everyone.** Host sends `JoinAt { tick: J }`
  (J = now + 2·delay); every peer stops at J; the host renders its world file
  (`Save::render()`, already sealed) plus the fleet (each ship's `ShipState`,
  craft, name) and sends it reliably; the joiner acks; host sends `Resume`. The
  new ship spawns at a pose derived from J and its player index (200 m off the
  host, matched velocity). No catch-up code in the MVP; for 2–8 friends a second's
  pause on join is fine. Test: `crates/net/src/session.rs::a_joiner_at_tick_j_runs_on_identical_to_the_room`.
- **Leave:** `Leave { player, at: L }` from the host; at L everyone removes the
  ship, frames from it after L are ignored. **Host leaves = session ends**; every
  peer keeps flying its world solo (it already has the whole state). No host
  migration in the MVP.
- **Desync:** every 120 ticks each peer sends `Beacon { tick, fleet_hash }` where
  `fleet_hash` folds `sim::state_hash` over every ship in player order. The host
  compares; a mismatch names the peer in the log and on its readout (`DESYNC`)
  and runs the join procedure for that peer (host's world wins). Three desyncs in
  a minute drop the peer. Test: `::a_desynced_peer_is_named_and_resynced_from_the_host`.
- **Pause:** in a session Esc opens the menu but the sim does not freeze; the
  pilot's frame keeps sending (controls zeroed while the menu is open). No
  session-wide pause in the MVP.
- **Save:** every peer keeps writing its own world file as today, so when the
  session ends you resume where you were. Nothing is saved server-side, ever.

## 2. Transport

### 2.1 Browser (the multiplayer edition)

- **WebRTC DataChannels, two per link:** `frames` (unordered, `maxRetransmits: 0`)
  and `ctl` (ordered, reliable: hello/welcome, join snapshot, beacons, ping,
  leave). Signalling over a WebSocket to ShedNet (§4).
- **Topology, MVP: host star.** One browser is the host; every peer holds one
  link to it; the host forwards frames. One NAT traversal per peer, the host is
  the join authority, and the wire cost is N−1 links on the host only. Full mesh
  (saves one hop, ~10–20 ms) is a later `net.topology = MESH` option, not MVP.
- **Relay when NAT fails:** standard ICE with a TURN server on ShedNet (coturn,
  BSD-3, apt package — zero code). `net.relay = AUTO | NEVER | ALWAYS`. Public STUN
  (`stun.l.google.com:19302`) is the fallback when ShedNet is down: works across
  most home NATs, not symmetric ones.
- **No-server path ("paste a code"):** the host's offer (SDP + gathered ICE, deflated,
  base64, ~600 chars) is the session code; the joiner pastes it and returns an
  answer code the same way. Copy/paste buttons on the page; a text row on the NET
  page. This is M-net-1's whole signalling and the permanent offline fallback.
- **JS owns the sockets, Rust owns the protocol.** `web/net.js` (~150 lines) holds
  `RTCPeerConnection`, the channels, the WebSocket and the codes, mirroring how
  `xr.js` owns the XR session. The bridge is three `#[wasm_bindgen]` functions in
  `crates/app/src/web.rs`: `net_push(peer, channel, bytes)`, `net_drain() -> Vec<u8>`
  (a length-prefixed batch of outgoing (peer, channel, bytes) taken once per frame)
  and `net_event(peer, kind)` for connect/disconnect. Reason: web-sys WebRTC needs
  a dozen feature flags and leaked Closures for what is 150 lines of JS.

### 2.2 Native (later, M-net-4) — the same protocol

- **Crate: `str0m`** (MIT OR Apache-2.0, sans-IO WebRTC: ICE, DTLS, SCTP data
  channels, no async runtime) on a `std::net::UdpSocket`; the same signalling
  messages over `tungstenite` (MIT/Apache-2.0). Fallback if str0m's data-channel
  path proves rough at the spike: `webrtc-rs` (MIT/Apache-2.0, heavier, tokio).
  `matchbox` (MIT/Apache-2.0) is rejected: it marries its own signalling server
  and socket shape. Every option passes `deny.toml`; the choice is re-verified
  at M-net-4 because it is the furthest out.
- Native never becomes a second protocol: a native client joins a browser host.

## 3. Code architecture (`app → {render, sim, net}`; `net → sim` only)

**New crate `crates/net` (farfall-net)**: pure Rust, deps `farfall-sim` + `glam`,
no GPU, no `std::time` (the caller passes `now_ms`), compiles to wasm trivially,
in the cargo-deny gate and the wasm check like the others.

```
crates/net/src/lib.rs        pub use; MAX_PLAYERS = 8; PeerId; Channel { Frames, Ctl }
crates/net/src/frame.rs      Frame, PackedControls, pack/unpack, fleet_hash
crates/net/src/wire.rs       Message enum + hand-rolled encode/decode (no format crate,
                             matches the repo idiom; the snapshot body IS the world file text)
crates/net/src/session.rs    Session state machine: Offline | Hosting | Joining | InSession;
                             the frame store (ring of [Option<Frame>; MAX_PLAYERS] by tick),
                             push_local / ready(tick) / ticks_to_run, beacons, join/leave/resync
crates/net/src/transport.rs  trait Transport { send(to, ch, &[u8]); recv() -> Option<(from, ch, Vec<u8>)>; peers() }
crates/net/src/loopback.rs   LoopbackNet::mesh(n, Faults { delay_ticks, loss, reorder, seed }) — seeded, deterministic
crates/app/src/net_bridge.rs BridgeTransport (wasm: the JS queues); Str0mTransport later (native)
web/net.js                   sockets, signalling, session codes
```

**`Game::tick` in a session** (the fixed-step loop, unchanged for Offline):
1. Compose `step_controls` exactly as today (HOLD, landing assist, chaos jitter),
   add the previous tick's summed impulses, `session.push_local(tick + delay, frame)`.
2. `let frames = session.ready(tick) else break` (stall).
3. Own ship: `sim::step` with the *unpacked own frame*, then `vel += impulse`.
4. Each remote pilot (`Vec<Pilot { id, name, craft, params: ShipParams, state: ShipState }>`):
   `sim::step(&pilot.params, &WorldState { time_s, ship: pilot.state }, frame.controls)`,
   then its impulse. The sim needs **no change**: `step` is already a pure per-ship
   function and `state_hash` hashes one ship; `fleet_hash` folds it. The golden
   hash never moves.
5. The belt/arms/mimics/miners/EVA glue runs on the own ship as today, but its
   shoves and kicks go into `pending_impulse` instead of onto `state.ship.vel_mps`.
6. `session.beacon_if_due(tick, fleet_hash)`.

**Shared in a session:** world preset + `wind_strength` (in `Welcome`), each
pilot's craft and its `ShipParams` (FIGHTER or HELICOPTER, fixed for the session;
changing craft = leave and rejoin), names, the delay.
**Not shared:** settings (graphics, keys, layout, panels), saves, the HUD, the
belt fight (rocks are hash-placed so everyone sees the same rocks in the same
places, but breaking them, mimics and miners stay per-pilot in the MVP).

**Drawing other pilots:** the mimic pass (`render/src/mimic.rs`, `MimicView`) with
a new `kind = 5` (pilot) and `PILOT_LANES = MAX_PLAYERS − 1` lanes beside
`MAX_MIMICS`; hull effort from the frame's thrust; a name on the glass through the
edge-arrow/label machinery mimics already use (`mimic.line`, the sight). The pass
draws the fighter SDF; a heli pilot wears the fighter hull until the mimic pass
gains a craft flag (small shader change, same chunk as M-net-1's render work).
Test: `crates/app/src/lib.rs::a_remote_pilot_wears_a_hull_lane_and_a_name_on_the_glass`.

**Menu, NET page** (settings keys, all with menu rows): `net.name`, `net.signal-url`
(default the ShedNet URL, blank = paste-only), `net.relay` (AUTO/NEVER/ALWAYS),
`net.max-players` (host; default 4, cap 8), rows `HOST SESSION`, `SESSION CODE`
(copy/paste), `LOBBY`, `LEAVE`. Native shows the page only from M-net-4.

## 4. The ShedNet service (`server/`, in this repo)

- **What:** `farfall-signal` — signalling + lobby, nothing else, holds no game
  state. Python 3, asyncio + `websockets` (BSD-3), ~200 lines, JSON over WSS:
  `host {name, max, craft} → code`, `list → [sessions]`, `join {code}`, and
  `offer / answer / ice` forwarded to the named peer. Sessions live in a dict
  and die with the host's socket. Plus **coturn** (BSD-3, apt) for STUN/TURN,
  static-auth secret, time-limited credentials minted by the signal server.
- **Why Python, not Rust:** it is the tekken-bot pattern Jay already operates
  (Debian CT, systemd, git-pull timer, no build step); the protocol is tiny.
- **Where:** Proxmox CT `shed-farfall` on `shed-pve`, unit
  `farfall-signal.service` (user `farfall`, `/opt/farfall/server`), timer
  `farfall-signal-update.timer` polling `origin/main` like `tekken-bot-update`.
  Ports: WS 8765 internal behind the estate's TLS reverse proxy (the page is
  served over https from GitHub Pages, so `ws://` is blocked as mixed content —
  **WSS is a hard requirement**); coturn 3478 tcp+udp and a small UDP relay range
  (49152–49200). Documented in `gnutgnut/shednet` like every other host.
- **When it is down:** the paste-a-code path with public STUN still works; the
  lobby row says `SHEDNET OFFLINE`. The game never needs it to run.
- Tests: `server/test_signal.py::a_host_code_is_listed_until_the_host_leaves`,
  `::offers_and_answers_reach_only_their_peer`, `::a_dead_host_takes_its_session_with_it`.

## 5. Milestones (each ends green and eyeballed; effort in agent-sessions)

| Milestone | Done means | Effort |
|---|---|---|
| **M-net-0 loopback lockstep** | `crates/net` exists, in CI. `crates/app/src/lib.rs::two_games_over_loopback_hash_identical_after_n_ticks_with_staggered_inputs` (two `Game`s, one host, `LoopbackNet` with delay 6, loss 5 %, reorder; 72 000 ticks = 10 sim-minutes; every beacon agrees). `crates/net/src/session.rs::two_peers_in_lockstep_agree_on_every_hash_after_ten_minutes`, `::a_late_frame_stalls_the_tick_and_never_skips_it`, `::lost_and_reordered_packets_are_covered_by_the_redundant_tail`, `frame.rs::packed_controls_round_trip_bit_for_bit_for_every_bool_and_the_axes_within_a_quantum`, `wire.rs::every_message_round_trips_through_its_bytes`. **And the golden hash under wasm**: a CI job running `cargo test -p farfall-sim` on `wasm32-wasip1` under wasmtime (Apache-2.0 w/ LLVM exception) — `golden_hash_holds_on_wasm32`. | 2 |
| **M-net-1 two browsers on a LAN** | `web/net.js`, the bridge, the NET page, pilot lanes. Two laptops on the home LAN join by pasted codes, fly formation for 10 minutes, no `DESYNC` line, a capture of the other hull on the glass looked at. Join, leave, host-leaves all exercised by hand. | 3 |
| **M-net-2 ShedNet lobby** | `server/` deployed on `shed-farfall` behind WSS; coturn up; `net.relay = ALWAYS` forces the TURN path and still lock-steps 10 minutes clean from two different networks (home + phone hotspot). The SPEC §11 experiment, settled. | 2 (+ Jay's CT/TLS time) |
| **M-net-3 WebXR in the session** | Quest Browser joins via the LOBBY row (no keyboard in VR: codes are entered flat before ENTER VR, the lobby needs none); the other pilot renders in stereo; 10 minutes clean at 72 Hz with the XR frame loop driving the tick. | 1 |
| **M-net-4 native client** | `Str0mTransport` + signalling client; the native binary joins a browser host through ShedNet; the cross-platform hash holds Windows-native vs Chrome vs Quest for 10 minutes. | 3–4 |

Total ≈ 11–12 sessions. Order is fixed; M-net-4 is the only one Jay Jay may cut.

**Phase 2 (after M-net-4, its own plan):** the shared belt fight — libm and
ordered maps in belt/arms/mimic/miner, session-shared gameplay settings, one
session PRNG, ship–ship contact; then full mesh and host migration.

## 6. Risks

- **wasm f64 equivalence.** IEEE ops and pure-Rust libm should be bit-identical,
  but no one has run the hash under wasm. M-net-0 does before anything else.
  Watch for `mul_add`/FMA in glam's f64 paths (wasm has no FMA; native may fuse).
- **App-side glue leaking into the shared state.** Anything that writes
  `state.ship` outside `sim::step` must go through `pending_impulse`. A test greps
  the session path for direct writes; the beacon catches what the test misses.
- **WebRTC inside a WebXR session.** DataChannel events arrive on the main thread
  independent of the XR frame loop, so nothing changes; the risk is Quest Browser
  throttling the page when the headset is doffed (the peer stalls everyone —
  handled by the 10 s drop). Measured at M-net-3.
- **Clock drift and jitter** are handled by the hold/catch-up rule; the number
  to watch is stall seconds per minute in the log.
- **Mixed content.** Without TLS on ShedNet the lobby cannot exist for the
  Pages-hosted page. Decide the TLS route before M-net-2 starts.

## 7. What Jay Jay decides

1. **Relay hosting:** ask Jay for a CT on `shed-pve`, a WSS route and the coturn
   ports (§4). Without it the edition still ships on paste-a-code.
2. **Max players:** proposed cap 8, default 4 (the mimic pass gains 7 lanes).
3. **Whether native ever joins:** my call is yes, last (M-net-4); until then the
   native binary is the singleplayer edition and the page is the multiplayer one.
4. **The belt fight staying per-pilot in the MVP** — accepted as stated, or phase 2
   pulled forward at ~3 more sessions.
