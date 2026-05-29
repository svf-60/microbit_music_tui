//! Playback definitions and the pure seek logic.
//!
//! The in-progress stream state ([`Playback`]) and the seek-target maths live
//! here, kept separate from the orchestration in [`crate::app`] — which owns the
//! connection, the song library and the current stream, and does the actual
//! decoding and chunk pumping.

/// Sample rate we resample WAV files to for the micro:bit v2 speaker.
pub const PCM_RATE: u32 = 7812;
/// Bytes of PCM written to the wire per chunk.
pub const PCM_CHUNK: usize = 512;
/// Chunks allowed in flight before we wait for a credit (`K`).
pub const PCM_WINDOW: usize = 4;
/// How far a forward/back control seeks within a song.
pub const SEEK_SECS: usize = 5;

/// State of an in-progress PCM stream.
#[derive(Debug)]
pub struct Playback {
    pub song_index: usize,
    pub samples: Vec<u8>,
    /// Next sample byte to send — the play position. It runs ~¼ s ahead of the
    /// actual sound by the in-flight window; we accept that slop.
    pub pos: usize,
    /// Chunks sent but not yet credited back.
    pub in_flight: usize,
    /// Whether all bytes plus the end marker have been written.
    pub ended: bool,
    /// Held by a pause control; streaming resumes on the next toggle.
    pub paused: bool,
    pub rate: u32,
}

impl Playback {
    /// Progress in 0.0..=1.0, for the gauge.
    pub fn progress(&self) -> f64 {
        if self.samples.is_empty() {
            0.0
        } else {
            self.pos as f64 / self.samples.len() as f64
        }
    }

    /// Current play position, in whole seconds.
    pub fn position_secs(&self) -> usize {
        self.pos / self.rate.max(1) as usize
    }

    /// Total duration, in whole seconds.
    pub fn duration_secs(&self) -> usize {
        self.samples.len() / self.rate.max(1) as usize
    }
}

/// Outcome of a within-song seek calculation.
#[derive(Debug, PartialEq, Eq)]
pub enum Seek {
    /// Stay in the song, at this byte offset.
    To(usize),
    /// Ran off the end (forward) or start (back) — cross to the adjacent song.
    Boundary,
}

pub fn forward_target(pos: usize, total: usize, step: usize) -> Seek {
    let target = pos + step;
    if target >= total {
        Seek::Boundary
    } else {
        Seek::To(target)
    }
}

pub fn back_target(pos: usize, step: usize) -> Seek {
    if pos <= step {
        Seek::Boundary
    } else {
        Seek::To(pos - step)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn forward_seeks_within_then_crosses_at_end() {
        assert_eq!(forward_target(0, 100, 30), Seek::To(30));
        assert_eq!(forward_target(80, 100, 30), Seek::Boundary);
        assert_eq!(forward_target(100, 100, 30), Seek::Boundary);
    }

    #[test]
    fn back_rewinds_then_crosses_at_start() {
        assert_eq!(back_target(50, 30), Seek::To(20));
        assert_eq!(back_target(30, 30), Seek::Boundary);
        assert_eq!(back_target(10, 30), Seek::Boundary);
    }
}
