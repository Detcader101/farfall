# HOTAS / joystick support

FARFALL flies on a stick. A Thrustmaster T.Flight HOTAS 4 works out of the box;
any other HID joystick works after a run through the wizard (Esc → STICK →
SETUP WIZARD). Everything lives in `crates/app/src/stick.rs`; the sim is
untouched — the stick is just another way of filling in the same `Controls`
the keyboard does.

## How it works

Two layers, kept apart (the design is lifted from the Arma Reforger HOTAS tool,
`hotas-reforger`):

| Layer | Maps | Where it lives |
|---|---|---|
| **Device map** | raw axis index / button number → flight control, trigger, named control | `~/.farfall/settings.cfg`, the `stick.*` keys; learned by the wizard |
| **Actions** | named control → what the game does | the same `Named` table the KEYS page uses |

A stick button bound to BOOST *is* the BOOST key: the app turns the button's
edge into a press/release of whatever key BOOST is bound to and sends it down
the one key path (`App::key_input`). There is no second list of what buttons
do, so a button can never do something the KEYS page cannot show.

Per frame (`App::poll_stick`, called from `RedrawRequested` before the frame
samples the controls):

1. The reader polls the platform for one sample: up to 8 axes in [-1, 1] and a
   32-bit button mask. The hat's four ways are bits 28–31, so a hat direction
   binds like any button. **Calibration gate**: an axis that has only ever
   read full deflection contributes nothing until it is first seen inside the
   rails (±0.95) — winmm reports an axis nothing has touched since plug-in at
   full rail (a live flight had the rocker's STRAFE at +1.00 throughout), so a
   railed reading from an untouched axis is "no data yet", not a demand. The
   moment it moves it is real for good, rails included; the log says
   `stick: axis 4 rests at full deflection - ignored until it moves`.
2. If the wizard is up, the sample goes to it and nothing flies.
3. Otherwise `StickMap::body_axes` turns the sample into the six body axes —
   thrust xyz, torque xyz — each shaped (deadzone, curve), signed by the body
   frame (+X right, +Y up, −Z the nose) and clamped. `InputState::set_stick`
   adds them to the keys' smoothed axes; the stick bypasses the key ramp (it is
   its own ramp) and the sum is clamped to [-1, 1].
4. The trigger button sets `stick_fire`, OR-ed with the mouse's trigger.
5. Every button edge becomes its key. If a KEYS row is waiting for a bind
   ("PRESS KEY"), a stick button binds itself to that row instead.

### Readers

| Platform | API | Crate | Licence |
|---|---|---|---|
| Windows | winmm `joyGetDevCapsW` / `joyGetPosEx` | `windows-sys` 0.61 (already in the tree through winit; feature `Win32_Media_Multimedia`) | MIT OR Apache-2.0 |
| Browser | Gamepad API `navigator.getGamepads()` | `web-sys` (features `Navigator`, `Gamepad`, `GamepadButton`) + `js-sys` | MIT OR Apache-2.0 |
| Linux / macOS | — none yet (see *Other platforms*) | | |

**Why winmm, not gilrs.** `gilrs` 0.11 (MIT/Apache-2.0) uses Windows Gaming
Input by default on Windows (XInput behind a feature). WGI does enumerate
generic HID sticks on Windows 10+, but it only delivers input to a focused
window, its axis order for a non-gamepad is whatever the HID descriptor says,
and it pulls the full `windows` crate tree in. winmm is the API the
`hotas-reforger` tool already measured this exact unit through — its axis
order (X Y Z U V R = indices 0–5) is what the shipped default map is written
in — and the binding is one function in a crate the tree already has. Zero new
dependencies, no licence question, a verified device layout. The trade: six
axes, 32 buttons, one hat, Windows only. That is the whole of a HOTAS 4.

The device is identified by its USB ids from `JOYCAPSW.wMid/wPid` (winmm's own
name is "Microsoft PC-joystick driver" for everything) and by parsing the
`Gamepad.id` string in the browser (Chrome's `(Vendor: 044f Product: b67c)`,
Firefox's `044f-b67c-`). A known id shows its name on the STICK page and in the
wizard: **T.FLIGHT HOTAS 4**. PID `B67B` is the same stick with its
base switch on PS4 — the wizard says so; set it to PC.

## The default map (T.Flight HOTAS 4, PC mode, winmm order)

Measured on Jay Jay's unit (`hotas-reforger/device-audit.json`) and asserted by
`hotas4_defaults_fly_the_ship_the_right_way`.

| Control on the stick | Raw | Flight control | Sign |
|---|---|---|---|
| Stick left/right | axis 0 (X) | ROLL | right = roll right |
| Stick fore/aft | axis 1 (Y) | PITCH | back = nose up |
| Throttle lever | axis 2 (Z), inverted | THROTTLE | forward = thrust along the nose; centre is zero (THROTTLE ZERO = CENTRE), back half is reverse |
| Throttle rocker (L2/R2 paddle) | axis 4 (V) | STRAFE | right = strafe right |
| Stick twist | axis 5 (R) | YAW | twist right = yaw right |
| — | axis 3 (U) | (unused on this unit) | |
| — | none | LIFT | R/F keys; bindable to any axis |

| Button | Raw | Named control |
|---|---|---|
| Trigger | B0 | FIRE (the trigger, `stick.fire`) |
| L1 (stick face) | B1 | BOOST |
| R3 (stick face) | B2 | AIR BRAKE |
| L3 (stick face) | B3 | DESPIN |
| Throttle face ◀ | B4 | MAP |
| Throttle face ▼ | B5 | LANDING MODE |
| Throttle face ▶ | B6 | NEXT WEAPON |
| Throttle face ▲ | B7 | FLIGHT ASSIST |
| Throttle R2 | B8 | CHAOS DRIVE (hold) |
| Throttle L2 | B9 | WARP STOP |
| Base left | B10 | CHASE CAM |
| Base right | B11 | HOLO3PP |
| Hat up | HAT-U | LOOK LOCK |
| Hat right | HAT-R | RAIL |
| Hat down | HAT-D | HOLD |
| Hat left | HAT-L | CANNON |

Deadzone 8 %, curve 1.5 (a power curve: finer about centre, full at the stop).

## The throttle's gestures, and the cabin's own stick

- **Lever hard back = air brake** (`stick.throttle-brake`, ON): the bottom
  ~5% of the lever's true travel holds the air brake exactly as holding
  Space does. Centre-zero only (with the zero at the bottom, idle would
  brake); an axis the calibration gate is still holding back reads 0 and
  cannot brake by resting.
- **Lever slammed forward = a chaos burst** (`stick.throttle-jump`, ON): at
  least 70% of travel forward within a quarter second, ending past halfway,
  holds the chaos drive for exactly two seconds and lets go — the same as
  holding H; the drive's own charge and entropy rules apply. A smooth push
  can never do it (test `a_smooth_throttle_push_never_jumps`), and holding
  the lever forward does not re-fire: bring it back and slam again.
- **The cabin's control column mirrors the demand** (`cockpit.stick`, ON —
  the CABIN page): the stick on the console leans with pitch and roll,
  its grip twists with yaw, and the lever on the left console slides with
  the throttle — from the HOTAS when it is flying, from the keys when not
  (it reads the summed demand). Render-only: a lane on the cabin uniforms
  (`CabinUniforms::with_stick`, quantised to 0.05 so a settling ramp does
  not re-march the cabin every frame).

Both gestures are listed on the HELP page under DRIVES, and each has a row
with a description on the STICK page (LEVER BRAKE, LEVER JUMP).

## Settings keys

```
stick.enabled = on
stick.pitch = 1            # raw axis index; a trailing - inverts; none
stick.yaw = 5
stick.roll = 0
stick.throttle = 2-
stick.strafe = 4
stick.lift = none
stick.deadzone = 0.08      # 0 .. 0.5 of half travel, symmetric
stick.curve = 1.50         # 1 linear .. 3
stick.throttle-zero = centre   # or bottom: the lever is 0..1 ahead only
stick.throttle-brake = on  # the lever hard back holds the air brake
stick.throttle-jump = on   # a slam forward = two seconds of chaos drive
stick.layout = hotas4      # how raw indices are named (hotas4 | generic); the reader sets it from the USB id
stick.fire = 0             # button number, hat-up/right/down/left, or none
stick.button.boost = 1
stick.button.<named> = ...  # one line per named control
```

One button, one job; one axis, one flight control: binding a button or an axis
that another control holds takes it away from that control (the keyboard's
swap rule, minus the swap — a stick has controls to spare).

## The wizard

Esc → STICK → SETUP WIZARD (or `FARFALL_BENCH_STICK=n` for a capture of step
`n`). It walks the **stick**, not the action list, so a control with no job is
a visible hole rather than an omission:

1. Six axis steps — "PITCH: PULL THE STICK BACK (NOSE UP)". The first reading on
   a step is where everything rests; the axis that then travels furthest from
   rest (at least half travel) is the one you moved, and the *direction* it
   went says whether to invert — so pushing the wrong way is fixed by pushing
   the right way, and **I** flips it by hand. A live bar shows the axis.
2. THROTTLE ZERO (centre / bottom), DEADZONE (let go; if the bar still moves,
   raise it), CURVE.
3. TRIGGER, then one step per named control, most useful first (BOOST, AIR
   BRAKE, DESPIN, CHAOS DRIVE, WARP STOP, FLIGHT ASSIST, LANDING MODE, MAP, …).
   A button already held when the step opened is not a press.
4. A summary: the axis map, the trigger, and a **coverage line that walks the
   hardware** — `COMPLETE: 21 OF 21 HAVE A JOB`, or `18 OF 21 HAVE A JOB. FREE:
   BASE L BASE R HAT-D` — so a control with no job is a visible hole, not an
   omission. The count comes from the attached device (axes, buttons, the hat
   as four) or, with none attached, from the known layout.

Once the stick has been identified (`stick.layout = hotas4`, set by the reader
from the USB id), every page names the physical control rather than the raw
index: TRIGGER, L1, R3, L3, FACE L, FACE D, FACE R, FACE U, R2, L2, BASE L, BASE R,
HAT-U/R/D/L; STICK X, STICK Y, LEVER, ROCKER, TWIST. A stick nobody knows shows
B0… and AXIS 0 (X)… (winmm letters) or AXIS 0… (browser).

Keys: **ENTER** keeps what was detected and moves on · **S** skips (keeps the
current value) · **X** clears this control · **B**/Backspace goes back ·
**I** inverts an axis · **< >** adjust the knobs · **ESC** finishes with what
has been done. Every accepted step is saved at once; there is no "apply".

The STICK page also edits the map by hand: **< >** step an axis row through
NONE, 0, 1, … and **ENTER** flips its direction; DEADZONE, CURVE, THROTTLE ZERO
and TRIGGER are rows too. The KEYS page shows each stick bind beside its key
(`LSHIFT B1`; an axis action shows its flight control, `UP PITCH`), and
pressing a stick button while a named row says PRESS KEY binds it there.

## Adding another stick

1. Plug it in and run the wizard — that is all a player needs. The map is per
   settings file (native and web keep separate ones, because the browser
   numbers axes its own way).
2. To make it *known* (its name shown, its controls named, a shipped default
   map): add its USB ids to `KNOWN` in `stick.rs`; add a `Layout` variant with
   its axis and button name tables (see `HOTAS4_AXES` / `HOTAS4_BUTTONS`) and
   have `poll_stick` pick it by id; and if you want a default map, add a
   constructor beside `StickMap::hotas4()` (today every stick starts from the
   HOTAS 4 map, which only matters until the wizard has run).
3. Measure, don't guess: `hotas-reforger`'s `-Identify` on Windows prints the
   winmm indices in the same order FARFALL uses.

## Other platforms

Linux and macOS have no native reader yet (`platform::find` returns `None`;
the STICK page says NONE FOUND). The road is `gilrs` (MIT/Apache-2.0: evdev on
Linux, IOKit on macOS, WGI on Windows) behind the same `platform::{find, read}`
pair — `Sample` and everything above it would not change. It was not taken now
because the one target unit is on Windows and the browser, and both are covered
without a new dependency.

## Tests

`crates/app/src/stick.rs` (mapping, shaping, settings round trip, the wizard's
detection and navigation, every wizard page fitting the panel) and
`crates/app/src/input.rs::the_stick_sums_with_the_keys_within_range`. Captures:
`farfall-captures/hotas/` (see `docs/polish/hotas.md`).
