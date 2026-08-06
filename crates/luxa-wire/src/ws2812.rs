//! WS2812 / WS2812B framing.

use luxa_color::{ColorOrder, Crgb};

use crate::{BitOrder, BitTiming, WireError};

/// The WS2812 family's wire format.
///
/// One pixel is three bytes, each clocked out most-significant bit first, in
/// the strip's [`ColorOrder`] — `GRB` for genuine WS2812B, though clones vary,
/// which is precisely why the order is a runtime field and not a constant.
///
/// ```
/// use luxa_color::{ColorOrder, Crgb};
/// use luxa_wire::Ws2812;
///
/// let chipset = Ws2812::new(ColorOrder::Grb);
/// let mut buf = [0u8; Ws2812::buffer_len(2)];
/// let n = chipset.encode(&[Crgb::new(1, 2, 3), Crgb::new(4, 5, 6)], &mut buf)?;
///
/// assert_eq!(n, 6);
/// assert_eq!(buf, [2, 1, 3, 5, 4, 6]);
/// # Ok::<(), luxa_wire::WireError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Ws2812 {
    /// The channel order this particular strip expects on the wire.
    pub order: ColorOrder,
}

impl Ws2812 {
    /// Bytes on the wire per pixel.
    pub const BYTES_PER_PIXEL: usize = 3;

    /// Bits are clocked out most-significant first.
    pub const BIT_ORDER: BitOrder = BitOrder::MsbFirst;

    /// WS2812B datasheet timing, with the conservative 280 µs reset that
    /// newer WS2812B-V5 dies require (older parts latch after 50 µs; sending
    /// the longer gap is safe on both).
    pub const TIMING: BitTiming = BitTiming {
        t0h_ns: 400,
        t0l_ns: 850,
        t1h_ns: 800,
        t1l_ns: 450,
        reset_us: 280,
    };

    /// A WS2812 chipset driving a strip with the given channel order.
    pub const fn new(order: ColorOrder) -> Self {
        Self { order }
    }

    /// Bytes needed to encode `pixel_count` pixels.
    ///
    /// `const` so callers can size a fixed buffer: `[0u8; Ws2812::buffer_len(N)]`.
    pub const fn buffer_len(pixel_count: usize) -> usize {
        pixel_count * Self::BYTES_PER_PIXEL
    }

    /// Encodes `pixels` into `out`, returning the number of bytes written.
    ///
    /// The caller's buffer may be longer than needed; only the returned prefix
    /// is meaningful.
    ///
    /// # Errors
    ///
    /// [`WireError::BufferTooSmall`] if `out` cannot hold the whole frame.
    /// Encoding is all-or-nothing: on error `out` is untouched, so a
    /// half-written frame can never reach a strip.
    pub fn encode(&self, pixels: &[Crgb], out: &mut [u8]) -> Result<usize, WireError> {
        let needed = Self::buffer_len(pixels.len());
        if out.len() < needed {
            return Err(WireError::BufferTooSmall {
                needed,
                got: out.len(),
            });
        }

        for (pixel, chunk) in pixels
            .iter()
            .zip(out.chunks_exact_mut(Self::BYTES_PER_PIXEL))
        {
            chunk.copy_from_slice(&self.order.permute(*pixel));
        }

        Ok(needed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Known-good vector: a genuine WS2812B strip is GRB, so pure red goes out
    /// as `00 FF 00`. Getting this backwards is the classic first-light bug,
    /// so it is pinned here rather than discovered on hardware.
    #[test]
    fn pure_red_on_a_grb_strip() {
        let mut out = [0u8; 3];
        let n = Ws2812::new(ColorOrder::Grb)
            .encode(&[Crgb::new(255, 0, 0)], &mut out)
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(out, [0x00, 0xFF, 0x00]);
    }

    #[test]
    fn pure_green_and_blue_on_a_grb_strip() {
        let chipset = Ws2812::new(ColorOrder::Grb);
        let mut out = [0u8; 3];

        chipset.encode(&[Crgb::new(0, 255, 0)], &mut out).unwrap();
        assert_eq!(out, [0xFF, 0x00, 0x00]);

        chipset.encode(&[Crgb::new(0, 0, 255)], &mut out).unwrap();
        assert_eq!(out, [0x00, 0x00, 0xFF]);
    }

    #[test]
    fn multiple_pixels_are_contiguous_in_strip_order() {
        let mut out = [0u8; Ws2812::buffer_len(3)];
        let n = Ws2812::new(ColorOrder::Grb)
            .encode(
                &[
                    Crgb::new(0x10, 0x11, 0x12),
                    Crgb::new(0x20, 0x21, 0x22),
                    Crgb::new(0x30, 0x31, 0x32),
                ],
                &mut out,
            )
            .unwrap();
        assert_eq!(n, 9);
        assert_eq!(
            out,
            [0x11, 0x10, 0x12, 0x21, 0x20, 0x22, 0x31, 0x30, 0x32],
            "pixel 0 must be first on the wire"
        );
    }

    #[test]
    fn order_is_a_runtime_property() {
        let px = [Crgb::new(0x11, 0x22, 0x33)];
        let mut grb = [0u8; 3];
        let mut rgb = [0u8; 3];
        Ws2812::new(ColorOrder::Grb).encode(&px, &mut grb).unwrap();
        Ws2812::new(ColorOrder::Rgb).encode(&px, &mut rgb).unwrap();
        assert_eq!(grb, [0x22, 0x11, 0x33]);
        assert_eq!(rgb, [0x11, 0x22, 0x33]);
    }

    #[test]
    fn oversized_buffer_leaves_the_tail_alone() {
        let mut out = [0xAAu8; 8];
        let n = Ws2812::default()
            .encode(&[Crgb::new(1, 2, 3)], &mut out)
            .unwrap();
        assert_eq!(n, 3);
        assert_eq!(&out[..3], &[2, 1, 3]);
        assert_eq!(&out[3..], &[0xAA; 5], "tail must not be scribbled on");
    }

    #[test]
    fn short_buffer_errors_without_writing() {
        let mut out = [0xAAu8; 5];
        let err = Ws2812::default()
            .encode(&[Crgb::new(1, 2, 3), Crgb::new(4, 5, 6)], &mut out)
            .unwrap_err();
        assert_eq!(err, WireError::BufferTooSmall { needed: 6, got: 5 });
        assert_eq!(out, [0xAA; 5], "buffer must be untouched on error");
    }

    #[test]
    fn empty_frame_is_ok() {
        assert_eq!(Ws2812::default().encode(&[], &mut []).unwrap(), 0);
    }

    #[test]
    fn timing_is_self_consistent() {
        let t = Ws2812::TIMING;
        assert!(t.t1h_ns > t.t0h_ns, "a one must be held longer than a zero");
        // WS2812B's nominal bit period is 1.25 µs; both encodings must match it
        // closely or the strip resynchronises mid-frame.
        assert_eq!(t.t0h_ns + t.t0l_ns, 1250);
        assert_eq!(t.t1h_ns + t.t1l_ns, 1250);
        assert_eq!(t.bit_period_ns(), 1250);
        assert!(t.reset_us >= 50, "reset must latch even the oldest dies");
    }

    #[test]
    fn buffer_len_is_three_bytes_per_pixel() {
        assert_eq!(Ws2812::buffer_len(0), 0);
        assert_eq!(Ws2812::buffer_len(60), 180);
    }
}
