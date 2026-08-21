//! farfall-audio — the ship's sound, synthesised live (SPEC P2, applied to
//! audio: zero samples, zero files; every sound is DSP driven by sim state).
//!
//! Split like the renderer: [`synth`] is pure and fully unit-tested with no
//! device; this module owns the real-time thread. The app writes [`Levels`]
//! into lock-free atomics each frame; the audio callback reads them. No locks,
//! no allocation, nothing that can block on the real-time thread.

#![forbid(unsafe_code)]

pub mod synth;

pub use synth::Levels;

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

/// Lock-free parameter mailbox: f32 stored as bits in atomics.
#[derive(Default)]
struct Shared {
    effort: AtomicU32,
    wind_q: AtomicU32,
    vacuum: AtomicU32,
    brake: AtomicU32,
    rcs: AtomicU32,
    entry: AtomicU32,
    supersonic: AtomicU32,
    hoops: AtomicU32,
    warp: AtomicU32,
    master: AtomicU32,
}

impl Shared {
    fn store(&self, l: &Levels) {
        self.effort.store(l.effort.to_bits(), Ordering::Relaxed);
        self.wind_q.store(l.wind_q.to_bits(), Ordering::Relaxed);
        self.vacuum.store(l.vacuum.to_bits(), Ordering::Relaxed);
        self.brake.store(l.brake.to_bits(), Ordering::Relaxed);
        self.rcs.store(l.rcs.to_bits(), Ordering::Relaxed);
        self.entry.store(l.entry.to_bits(), Ordering::Relaxed);
        self.supersonic
            .store(l.supersonic.to_bits(), Ordering::Relaxed);
        self.hoops.store(l.hoops.to_bits(), Ordering::Relaxed);
        self.warp.store(l.warp.to_bits(), Ordering::Relaxed);
        self.master.store(l.master.to_bits(), Ordering::Relaxed);
    }
    fn load(&self) -> Levels {
        Levels {
            effort: f32::from_bits(self.effort.load(Ordering::Relaxed)),
            wind_q: f32::from_bits(self.wind_q.load(Ordering::Relaxed)),
            vacuum: f32::from_bits(self.vacuum.load(Ordering::Relaxed)),
            brake: f32::from_bits(self.brake.load(Ordering::Relaxed)),
            rcs: f32::from_bits(self.rcs.load(Ordering::Relaxed)),
            entry: f32::from_bits(self.entry.load(Ordering::Relaxed)),
            supersonic: f32::from_bits(self.supersonic.load(Ordering::Relaxed)),
            hoops: f32::from_bits(self.hoops.load(Ordering::Relaxed)),
            warp: f32::from_bits(self.warp.load(Ordering::Relaxed)),
            master: f32::from_bits(self.master.load(Ordering::Relaxed)),
        }
    }
}

pub struct Audio {
    shared: Arc<Shared>,
    // Held for its Drop: the stream stops when Audio does.
    _stream: cpal::Stream,
}

impl Audio {
    /// Open the default output device and start the synth. Returns None when
    /// no device exists (CI, headless) — the game must run silent rather than
    /// not run.
    pub fn start() -> Option<Self> {
        let device = cpal::default_host().default_output_device()?;
        let config = device.default_output_config().ok()?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            log::warn!(
                "audio: unsupported sample format {:?}",
                config.sample_format()
            );
            return None;
        }
        let channels = config.channels() as usize;
        let sample_rate = config.sample_rate() as f32;
        let shared = Arc::new(Shared::default());
        shared.store(&Levels::default());

        let cb_shared = shared.clone();
        let mut synth = synth::Synth::new(sample_rate, 0x5EED_51D0);
        // Stereo scratch, resized inside the callback only downward.
        let mut scratch = vec![0.0f32; 4096 * 2];

        let stream = device
            .build_output_stream(
                config.into(),
                move |out: &mut [f32], _| {
                    let frames = out.len() / channels.max(1);
                    let need = frames * 2;
                    if scratch.len() < need {
                        // Growing allocates on the audio thread once, at most:
                        // buffer sizes are fixed after stream start.
                        scratch.resize(need, 0.0);
                    }
                    let levels = cb_shared.load();
                    synth.render(&levels, &mut scratch[..need]);
                    for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                        let (l, r) = (scratch[i * 2], scratch[i * 2 + 1]);
                        for (c, sample) in frame.iter_mut().enumerate() {
                            *sample = match c {
                                0 => l,
                                1 => r,
                                // Extra channels (5.1 etc): fold to mono.
                                _ => (l + r) * 0.5,
                            };
                        }
                    }
                },
                |e| log::warn!("audio stream error: {e}"),
                None,
            )
            .ok()?;
        stream.play().ok()?;
        log::info!("audio: {sample_rate} Hz, {channels} ch, live synthesis");
        Some(Self {
            shared,
            _stream: stream,
        })
    }

    /// Update the synth's control levels. Lock-free; call at any rate.
    pub fn set(&self, levels: &Levels) {
        self.shared.store(levels);
    }
}
