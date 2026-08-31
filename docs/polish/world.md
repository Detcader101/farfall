# `world` — the planet, its air, its ground, the nebula, the belt's ring

Branch `fable/world`, worktree `farfall-wt/world`. Captures in
`farfall-captures/world/` (names below). Everything here is render-only:
`shaders/planet.wgsl`, `shaders/bodies.wgsl`, `shaders/nebula.wgsl`,
`crates/render/src/{planet,bodies,nebula}.rs`, three settings and menu rows.

## 1. The "nebula seam" was the ring (fixed — `ring-belt-*`, `ring-oblique-1`, `ring-far-1`)

The baseline's hard vertical edge in the belt sky (belt-spin-1/4/7/10) was
**not** the nebula bake nor the starfield's equirect wrap. Elimination
captures (`seam-nocab-*` with `FARFALL_SKIP=cockpit`, `seam-noneb-*` with the
nebula off, `seam-lvl0-*` sampling the nebula at mip 0 without derivatives,
`seam-nobelt-*`, `seam-nobodies-*`) left only the **bodies pass**: Uranus'
ring was drawn by intersecting the ray with the ring's mid-plane and applying
the far sheet's veil at the hit. Sitting in the belt the camera *is on that
plane*, so the hit was the camera itself for every ray on one side of the
plane and nothing on the other — the plane's great circle across the sky as a
razor edge, alpha 0.15 grey on one side, and the far-rock specks on that side
only.

Fix (`5e4a65d`): the ring is a **slab** the belt's own height (`RING_HAZE_M`
= 1.5 km either side of the plane), and every ray is charged for the length
it runs inside slab ∩ annulus, stopped at the hole's near wall. One rule from
anywhere: from afar a face-on crossing is one thickness and the dark
near-opaque thread is unchanged (`ring-oblique-1`, `ring-far-1`); from inside
a ray to the zenith is out of the haze in a kilometre and a ray along the
plane runs to the annulus edge — a soft band of sun-lit dust on the belt's
own horizon, brightest grazing and toward the Sun (`ring-belt-4/5`). The
far rocks hit the slab face the ray is heading for, so they show on both
sides. `bodies.rs::ring_run_m` mirrors the maths with four tests
(continuity across the plane, the hole's wall, far-above rays, the face).

**Handed to postfx:** the *stars* smear into streaks along converging lines
near the octahedral fold (`zoom-swirl.png`). `oct_encode` folds on z<0, so its
seams are the world half-planes x=0,z<0 and y=0,z<0 — and the ring axis
(0.97,0.14,0.2) puts the belt's plane almost on the x=0 fold. That is
`starfield.wgsl`'s Jacobian across the fold; postfx owns stars.

## 2. The atmosphere is single scattering now (`planet.wgsl`)

Replaced the three-term model (rim glow / aerial mix / dome) with one
marched single-scattering integral: an exponential shell (Rayleigh scale
height 0.07 R, Mie haze 0.012 R, top at 0.42 R), the preset's colour as the
Rayleigh tint (so EARTHLIKE's sky is blue and its sunsets orange; VENUSIAN
amber; ALIEN green — presets keep their colours), a Henyey-Greenstein haze
for the horizon glow and the Sun's forward scatter. Eight samples per ray,
clustered at the ray's lowest point (quadratic spacing either side of the
closest approach), each lit by the Sun through an analytic Chapman column,
zero in the planet's shadow. The march is split at the cloud deck so the air
in front veils the deck and the air behind sits under it. Composition is in
radiance with one tonemap at the end; alpha over the stars is the geometric
transmittance plus a luminance-driven cover (a noon sky hides stars, a
twilight of the same depth does not).

Contract kept: the SKY feature's altitude curve (full blue at 3 km, pale at
8, dim at 15, black at 25 over a 64 km world) is now `sky_column()` in
`planet.rs` with a test, and `SKY` scales the in-scattered light.

Decisions: no LUT — the Chapman closed form is a handful of ops and needs no
asset or bake; sample count fixed at 8 (16 with a deck in the ray) because
the cost is dominated by the ground's noise, not the march.

## 3. The ground (`planet.wgsl`)

- **Relief.** Over the baked continents (2048×1024, ~200 m/texel) a live
  fbm adds hills and ridged noise adds mountains where the continents are
  already high; the height a footprint away along two tangents gives a
  shaded normal (the tangents also displace the baked field, so coasts get
  relief). Octaves are added only up to the pixel's footprint on the ground
  (`fwidth(normal)` → `max_freq`, one octave from orbit, six at a few
  hundred metres, the last fading in — nothing pops). `TERRAIN DETAIL`
  shifts the octave budget (0 = baked only, 200% = an octave finer).
- **Colour.** Water by depth (deep → turquoise shallows), detailed
  coastlines from the summed height, sand at the shore, verdant → arid by
  the baked dryness, grey rock on steep slopes and high ground, snow above
  a line that falls toward the poles (with the baked ice-edge noise).
  Sunlight reaches the ground through the air (`sun_trans`), so it reddens
  at sunset; the sky lights the shadows in its own tint.
- **Clouds.** Four fbm-LOD octaves over the baked deck field where the
  footprint allows; the deck is lit through the air; shadows on the ground
  carry the detail and drift with the weather phase. `CLOUDS` multiplies
  the preset's coverage (OFF clears the sky).
- **City lights.** Night side, on habitable coastal lowland where the baked
  settlement field is high: a street grid (240 m blocks, avenues every
  fifth), lit points on a 60 m lattice (sodium, white, the odd cyan/magenta
  — the SPEC §6.6 neon direction), radiance ×7 for the bloom; footprint-
  banded so from orbit it collapses to a warm glow. `CITY LIGHTS` 0..200%.

## 4. The nebula bake (`nebula.wgsl`, `nebula.rs`)

4096×2048 Rgba16Float with mips (~85 MB, baked once per knob change, ~100 ms
of GPU). Five ridged octaves with a finer domain warp for the emission
cores, a high-octave "lace" of the finest threads, dust lanes with a ragged
finer edge. The fetch is unchanged: one `textureSampleGrad` per pixel.

## 5. Verification (state at the time of writing)

Benchmarks were halted by Jay Jay (`locks/HALT`) after the ring fix was
captured and before the atmosphere/ground/nebula captures could be taken.
Gate: fmt, clippy, `cargo test --workspace`, wasm check — all green.

To look at when benches resume (names as this doc will use them):
`v1-alt{500,3000,12000,80000}-{1..6}` (FARFALL_BENCH_ALT spins),
`v1-globe-night-1` (POS 0,0,400000), `v1-globe-day-1`
(POS 248000,168000,-264000), `v1-night-low-{1..6}`
(POS -40060,-27160,42640 spin), `v1-nebula-{1..12}` (belt spin) and
`FARFALL_BENCH_NEBULA=1`, and the cost: `FARFALL_BENCH_FULL=1
FARFALL_VSYNC=off FARFALL_BENCH_SECONDS=8 FARFALL_BENCH_ALT=500`.

## 6. Left

- Captures and tuning of SUN_I / MIE_K / relief amplitudes by eye.
- The 2880×1800 cost at 500 m.
- Sun-lit far rocks: not touched (the particles agent has the near rocks).
