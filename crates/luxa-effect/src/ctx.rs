//! The render context — the only channel through which an effect learns
//! anything about the outside world.

/// Per-frame inputs handed to every effect.
///
/// `Ctx` is constructed once per frame by the render task and passed down
/// unchanged, so every effect in a frame animates against the same instant.
///
/// The field is private and [`from_millis`](Self::from_millis) is the only
/// constructor. That is not ceremony: it is the enforcement mechanism for the
/// clock boundary. An effect cannot obtain a `Ctx` on its own, so it cannot
/// invent a timestamp, and the narrowing from the platform clock happens in
/// one auditable place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ctx {
    now_ms: u32,
}

impl Ctx {
    /// Builds a context for a frame rendered at `now_ms`.
    ///
    /// `now_ms` is a **wrapping** millisecond counter: it rolls over roughly
    /// every 49.7 days, and effects must treat it as such — use
    /// `wrapping_add`, never assume monotonicity across the whole range.
    ///
    /// Call this from the render task and nowhere else.
    #[inline]
    pub const fn from_millis(now_ms: u32) -> Self {
        Self { now_ms }
    }

    /// Narrows a monotonic microsecond clock to this crate's animation clock.
    ///
    /// This is the single conversion point between the platform's `u64` µs
    /// timebase and the `u32` ms animation timebase. The truncation is
    /// deliberate and lossless in the sense that matters: `u64` ms values
    /// congruent modulo 2³² produce the same frame.
    #[inline]
    pub const fn from_micros_u64(now_us: u64) -> Self {
        Self::from_millis((now_us / 1_000) as u32)
    }

    /// The animation clock for this frame, in milliseconds, wrapping.
    #[inline]
    pub const fn now_ms(&self) -> u32 {
        self.now_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn micros_narrow_to_millis() {
        assert_eq!(Ctx::from_micros_u64(1_500_000).now_ms(), 1_500);
        assert_eq!(Ctx::from_micros_u64(999).now_ms(), 0);
    }

    #[test]
    fn micros_beyond_u32_ms_wrap_rather_than_saturate() {
        // 2^32 ms expressed in µs must land back on 0, not on u32::MAX.
        let two_pow_32_ms_in_us = (1u64 << 32) * 1_000;
        assert_eq!(Ctx::from_micros_u64(two_pow_32_ms_in_us).now_ms(), 0);
        assert_eq!(
            Ctx::from_micros_u64(two_pow_32_ms_in_us + 7_000).now_ms(),
            7
        );
    }
}
