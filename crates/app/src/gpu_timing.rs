//! PLAN item 1 (`docs/PLAN-VR-PERF.md`): per-pass GPU timestamp queries in
//! VR bench mode, feeding the perf line's `pass_ms_eye0={...}
//! pass_ms_eye1={...}` — measured before any of items 2-5 change what
//! gets drawn.
//!
//! The five names ([`PASS_NAMES`]) are the pass boundaries the VR eye
//! loop in `redraw` (`crates/app/src/lib.rs`) already has: the thermal
//! compute step, the cabin's own progressive-refinement render (0 or 1
//! passes a frame — `CabinPass::update`), the "scene" world pass
//! (starfield/bodies/planet/belt/mimic/heli/scar/debris/tracer/dust/wind/
//! jet/plasma/trajectory/shield/ghost), the "ship" pass (bloom chain,
//! then the cabin/dials/holo drawn over the world), and the "present"
//! pass (blit/map/hologram/hud/pointer). Each is bookended with
//! [`GpuPassTimer::begin`]/[`GpuPassTimer::end`] directly on the
//! command encoder — `Features::TIMESTAMP_QUERY_INSIDE_ENCODERS`, not a
//! per-pass `timestamp_writes` field — so none of `render`'s pass-
//! building code needs to change to be timed: a span just needs to
//! start and end somewhere in the same eye's encoder, whether or not it
//! turns out to draw a `wgpu::RenderPass` this particular frame (the
//! cabin's own cache often doesn't).
//!
//! Read-back piggybacks on native VR's own existing full-queue
//! `device.poll(wait_indefinitely())` after `queue.present` (see
//! `redraw`): that poll already blocks every native-VR frame today
//! (`render_ms`'s own measurement depends on it), so mapping this
//! module's readback buffer right after it adds no new stall — the GPU
//! is already known idle by the time [`GpuPassTimer::read_back`] runs.

// ---------------------------------------------------------------------
// Pure maths — slot indexing, tick-to-ms conversion, the perf-line
// token — the part of this module that runs without a device, and is
// unit-tested accordingly (see `pure_math_tests` below).
// ---------------------------------------------------------------------

/// Every pass this instruments, in the fixed order that fixes each
/// pass's query-set slot pair — see [`slot`].
pub const PASS_NAMES: [&str; 5] = ["thermal", "cabin", "world", "post", "present"];

/// VR is always stereo; a flat (non-VR) frame never touches this module.
pub const EYES: usize = 2;

const SLOTS_PER_PASS: u32 = 2; // begin, end
const SLOTS_PER_EYE: u32 = PASS_NAMES.len() as u32 * SLOTS_PER_PASS;
/// The query set's total slot count: both eyes' begin/end pair for every
/// named pass.
pub const CAPACITY: u32 = SLOTS_PER_EYE * EYES as u32;

/// `pass`'s index into [`PASS_NAMES`], or `None` for a name this module
/// doesn't instrument — a caller passing an unrecognised name is a no-op
/// (see [`GpuPassTimer::begin`]/[`end`]) rather than a panic, so a typo'd
/// span name silently drops that one span instead of crashing a bench
/// row.
pub fn pass_index(pass: &str) -> Option<usize> {
    PASS_NAMES.iter().position(|&n| n == pass)
}

/// The query-set slot a given eye/pass/edge (begin or end) writes to —
/// every (eye, pass, edge) triple gets its own slot, so no write ever
/// overwrites another's.
fn slot(eye: usize, pass: usize, begin: bool) -> u32 {
    (eye as u32) * SLOTS_PER_EYE + (pass as u32) * SLOTS_PER_PASS + u32::from(!begin)
}

/// One pass's duration from its raw begin/end timestamp ticks and the
/// queue's own timestamp period (ns/tick, `Queue::get_timestamp_period`)
/// — `0.0` if the pass never ran this frame (both ticks are the
/// buffer's cleared/stale `0`) or the ticks read backwards (a stale
/// slot from a frame that skipped this pass entirely, or a driver that
/// reordered the writes) — reporting zero rather than a negative or
/// wrapped-looking number keeps a glance at the perf line honest about
/// "didn't run" versus "ran fast".
pub fn duration_ms(begin_ticks: u64, end_ticks: u64, period_ns: f32) -> f32 {
    if end_ticks <= begin_ticks {
        return 0.0;
    }
    (end_ticks - begin_ticks) as f32 * period_ns / 1_000_000.0
}

/// The perf line's own token, built from both eyes' resolved
/// millisecond grid (`ms[eye][pass_index]`) — every name in
/// [`PASS_NAMES`], both eyes, so a per-eye asymmetry (item 5's cache
/// settling at different times per eye, say) is visible without a
/// second bench row.
pub fn format_pass_ms(ms: &[[f32; PASS_NAMES.len()]; EYES]) -> String {
    let one = |eye: usize| {
        PASS_NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| format!("{name}:{:.3}", ms[eye][i]))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!("pass_ms_eye0={{{}}} pass_ms_eye1={{{}}}", one(0), one(1))
}

// ---------------------------------------------------------------------
// The GPU-touching half: a query set, a resolve buffer, a CPU-mappable
// readback buffer. Exercised by compiling and by a bench row, not a
// unit test — there is no GPU in `cargo test`.
// ---------------------------------------------------------------------

/// Per-eye, per-pass milliseconds, resolved from the previous frame's
/// query-set writes.
type MsGrid = [[f32; PASS_NAMES.len()]; EYES];

/// Bookends [`PASS_NAMES`] on a native-VR frame's own command encoders
/// with GPU timestamp writes, resolves them into a CPU-mappable buffer,
/// and reads that buffer back once the caller already knows the queue is
/// idle. `None` (via [`GpuPassTimer::new`]) when the device doesn't
/// offer `TIMESTAMP_QUERY_INSIDE_ENCODERS` — VR bench then runs with no
/// `pass_ms_eye*=` token rather than a half-instrumented line.
pub struct GpuPassTimer {
    query_set: wgpu::QuerySet,
    resolve_buf: wgpu::Buffer,
    readback_buf: wgpu::Buffer,
    /// `Queue::get_timestamp_period`, read once at construction — the
    /// runtime's queue doesn't change period frame to frame.
    period_ns: f32,
    /// The most recently resolved frame's grid — `None` until the first
    /// successful [`read_back`](Self::read_back).
    last_ms: Option<MsGrid>,
}

impl GpuPassTimer {
    /// The two features this needs together: `TIMESTAMP_QUERY` for the
    /// query set/resolve machinery itself, `TIMESTAMP_QUERY_INSIDE_ENCODERS`
    /// for bookending an arbitrary span with `CommandEncoder::write_timestamp`
    /// rather than only a whole render/compute pass's own start/end.
    const REQUIRED: wgpu::Features =
        wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

    /// `None` if the device wasn't opened with both required features
    /// (see `REQUIRED`) — never a panic; a device that doesn't offer
    /// timer queries just runs without one.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(Self::REQUIRED) {
            return None;
        }
        let query_set = device.create_query_set(&wgpu::QuerySetDescriptor {
            label: Some("vr pass timer"),
            ty: wgpu::QueryType::Timestamp,
            count: CAPACITY,
        });
        let byte_size = u64::from(CAPACITY) * 8;
        let resolve_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vr pass timer resolve"),
            size: byte_size,
            usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let readback_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("vr pass timer readback"),
            size: byte_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        Some(Self {
            query_set,
            resolve_buf,
            readback_buf,
            period_ns: queue.get_timestamp_period(),
            last_ms: None,
        })
    }

    /// Write a begin timestamp for `pass` on `eye`'s own encoder — a
    /// no-op for a name [`pass_index`] doesn't recognise.
    pub fn begin(&self, encoder: &mut wgpu::CommandEncoder, eye: usize, pass: &str) {
        if let Some(i) = pass_index(pass) {
            encoder.write_timestamp(&self.query_set, slot(eye, i, true));
        }
    }

    /// Write the matching end timestamp — see [`begin`](Self::begin).
    pub fn end(&self, encoder: &mut wgpu::CommandEncoder, eye: usize, pass: &str) {
        if let Some(i) = pass_index(pass) {
            encoder.write_timestamp(&self.query_set, slot(eye, i, false));
        }
    }

    /// Resolve this frame's writes into the CPU-visible buffer — call
    /// once per eye's own encoder, after every `begin`/`end` pair for
    /// that eye has been recorded and before `encoder.finish()`. Safe to
    /// call twice in one frame (eye 0's encoder, then eye 1's): each
    /// resolves the *whole* query set again, but eye 0's own slots
    /// haven't changed by the time eye 1's encoder runs, so the second
    /// resolve just re-copies the same values for them.
    pub fn resolve(&self, encoder: &mut wgpu::CommandEncoder) {
        encoder.resolve_query_set(&self.query_set, 0..CAPACITY, &self.resolve_buf, 0);
        encoder.copy_buffer_to_buffer(
            &self.resolve_buf,
            0,
            &self.readback_buf,
            0,
            self.resolve_buf.size(),
        );
    }

    /// Map and read the resolved buffer — call only once the caller
    /// already knows the queue is idle (native VR's own per-frame
    /// `device.poll(wait_indefinitely)` after `queue.present`), so this
    /// adds no stall of its own beyond servicing the map callback.
    /// Leaves `last_ms` unchanged (not cleared) on any failure, so a
    /// single bad frame doesn't blank the perf line's last known-good
    /// numbers.
    pub fn read_back(&mut self, device: &wgpu::Device) {
        let slice = self.readback_buf.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
            return;
        }
        let Ok(Ok(())) = rx.recv() else {
            return;
        };
        let Ok(mapped) = slice.get_mapped_range() else {
            return;
        };
        let raw: &[u64] = bytemuck::cast_slice(&mapped);
        let mut ms: MsGrid = [[0.0; PASS_NAMES.len()]; EYES];
        for (eye, row) in ms.iter_mut().enumerate() {
            for (i, out) in row.iter_mut().enumerate() {
                let begin = raw[slot(eye, i, true) as usize];
                let end = raw[slot(eye, i, false) as usize];
                *out = duration_ms(begin, end, self.period_ns);
            }
        }
        drop(mapped);
        self.readback_buf.unmap();
        self.last_ms = Some(ms);
    }

    /// The perf line's `pass_ms_eye0={...} pass_ms_eye1={...}` token —
    /// `None` before the first successful [`read_back`](Self::read_back)
    /// (e.g. the first frame or two of a bench row).
    pub fn pass_ms_line(&self) -> Option<String> {
        self.last_ms.as_ref().map(format_pass_ms)
    }
}

#[cfg(test)]
mod pure_math_tests {
    use super::*;

    #[test]
    fn every_eye_pass_edge_gets_its_own_slot() {
        let mut seen = std::collections::HashSet::new();
        for eye in 0..EYES {
            for pass in 0..PASS_NAMES.len() {
                for begin in [true, false] {
                    assert!(
                        seen.insert(slot(eye, pass, begin)),
                        "slot collision at eye {eye} pass {pass} begin {begin}"
                    );
                }
            }
        }
        assert_eq!(seen.len(), CAPACITY as usize);
        assert!(seen.iter().all(|&s| s < CAPACITY));
    }

    #[test]
    fn a_passs_begin_and_end_slot_are_never_the_same() {
        for eye in 0..EYES {
            for pass in 0..PASS_NAMES.len() {
                assert_ne!(slot(eye, pass, true), slot(eye, pass, false));
            }
        }
    }

    #[test]
    fn pass_index_finds_every_declared_name_at_its_own_position() {
        for (i, name) in PASS_NAMES.iter().enumerate() {
            assert_eq!(pass_index(name), Some(i));
        }
    }

    #[test]
    fn pass_index_is_none_for_an_undeclared_name() {
        assert_eq!(pass_index("bogus"), None);
        assert_eq!(pass_index(""), None);
    }

    #[test]
    fn a_pass_that_never_ran_this_frame_reports_zero() {
        // Both ticks stay at the readback buffer's cleared 0 when a pass
        // (the cabin, most often — CabinWork::Nothing) never wrote to
        // its slots this frame.
        assert_eq!(duration_ms(0, 0, 83.333), 0.0);
    }

    #[test]
    fn end_before_begin_reports_zero_not_a_wrapped_number() {
        assert_eq!(duration_ms(1_000_000, 500_000, 1.0), 0.0);
    }

    #[test]
    fn duration_converts_ticks_through_the_queues_own_period() {
        // 12,000 ticks at a 1ns period (a plain nanosecond clock) is
        // exactly 12 microseconds — 0.012 ms.
        let ms = duration_ms(0, 12_000, 1.0);
        assert!((ms - 0.012).abs() < 1e-6, "{ms}");
    }

    #[test]
    fn a_realistic_period_converts_ticks_to_milliseconds_correctly() {
        // A 1200 MHz timestamp clock (period ~0.8333 ns/tick, roughly
        // what several desktop GPUs report) over 6,000,000 ticks is 5ms.
        let period_ns = 1_000.0 / 1200.0;
        let ms = duration_ms(0, 6_000_000, period_ns);
        assert!((ms - 5.0).abs() < 1e-3, "{ms}");
    }

    #[test]
    fn format_pass_ms_names_every_pass_for_both_eyes() {
        let mut ms = [[0.0f32; PASS_NAMES.len()]; EYES];
        for (i, _) in PASS_NAMES.iter().enumerate() {
            ms[0][i] = (i + 1) as f32;
            ms[1][i] = (i + 1) as f32 * 2.0;
        }
        let line = format_pass_ms(&ms);
        assert!(line.starts_with("pass_ms_eye0={"), "{line}");
        assert!(line.contains("} pass_ms_eye1={"), "{line}");
        for name in PASS_NAMES {
            assert!(
                line.matches(&format!("{name}:")).count() == 2,
                "{name} should appear once per eye: {line}"
            );
        }
    }

    #[test]
    fn format_pass_ms_reflects_a_genuine_per_eye_difference() {
        let mut ms = [[0.0f32; PASS_NAMES.len()]; EYES];
        ms[0][pass_index("cabin").unwrap()] = 1.5;
        ms[1][pass_index("cabin").unwrap()] = 0.0;
        let line = format_pass_ms(&ms);
        assert!(line.contains("cabin:1.500"), "{line}");
        assert!(line.contains("cabin:0.000"), "{line}");
    }
}
