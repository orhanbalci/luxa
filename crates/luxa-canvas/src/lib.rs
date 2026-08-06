//! The Luxa framebuffer.
//!
//! A [`Canvas`] is a fixed-length, stack-allocated buffer of pixels — nothing
//! more. It has no notion of chipset, transport, brightness or time, which is
//! exactly why it does not move when any of those change.
//!
//! # One dimension, for now
//!
//! Slice 1 is a single strip, so a canvas is linear and its only geometry is
//! "index 0 is the pixel nearest the controller". XY mapping, serpentine
//! layouts and multi-panel addressing are later slices; they will arrive as
//! *views over* this buffer, not as a replacement for it.
//!
//! # Views
//!
//! A **canvas view** is just `&mut [Crgb]`. Renderers — effects, the
//! compositor, the brightness stage — all take a plain mutable pixel slice, so
//! a segment can hand an effect its sub-range without any of them needing to
//! know the canvas type. `Canvas` derefs to `[Crgb]`, so passing a whole
//! canvas where a view is wanted is `&mut canvas`.

#![no_std]
#![forbid(unsafe_code)]

use core::ops::{Deref, DerefMut};

use luxa_color::Crgb;

/// A fixed-length pixel buffer of `N` pixels.
///
/// `N` is a const parameter rather than a runtime length because the buffer
/// lives on the stack or in a `static` on a device with no allocator. The
/// *physical* strip length can be shorter than `N` — that is a property of the
/// LED profile, and the render path simply works on a prefix view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Canvas<const N: usize> {
    pixels: [Crgb; N],
}

impl<const N: usize> Canvas<N> {
    /// The number of pixels this canvas holds.
    pub const LEN: usize = N;

    /// Creates an all-black canvas.
    #[inline]
    pub const fn black() -> Self {
        Self {
            pixels: [Crgb::new(0, 0, 0); N],
        }
    }

    /// Creates a canvas with every pixel set to `color`.
    #[inline]
    pub const fn solid(color: Crgb) -> Self {
        Self { pixels: [color; N] }
    }

    /// Creates a canvas from an existing pixel array.
    #[inline]
    pub const fn from_pixels(pixels: [Crgb; N]) -> Self {
        Self { pixels }
    }

    /// Borrows the pixels as an immutable slice.
    #[inline]
    pub const fn as_slice(&self) -> &[Crgb] {
        &self.pixels
    }

    /// Borrows the pixels as a mutable view, ready to render into.
    #[inline]
    pub const fn as_mut_slice(&mut self) -> &mut [Crgb] {
        &mut self.pixels
    }

    /// Sets every pixel to `color`.
    #[inline]
    pub fn fill(&mut self, color: Crgb) {
        self.pixels.fill(color);
    }

    /// Sets every pixel to black.
    #[inline]
    pub fn clear(&mut self) {
        self.fill(Crgb::new(0, 0, 0));
    }
}

impl<const N: usize> Default for Canvas<N> {
    fn default() -> Self {
        Self::black()
    }
}

impl<const N: usize> Deref for Canvas<N> {
    type Target = [Crgb];

    #[inline]
    fn deref(&self) -> &[Crgb] {
        &self.pixels
    }
}

impl<const N: usize> DerefMut for Canvas<N> {
    #[inline]
    fn deref_mut(&mut self) -> &mut [Crgb] {
        &mut self.pixels
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn black_is_all_zero() {
        let c = Canvas::<8>::black();
        assert_eq!(c.len(), 8);
        assert!(c.iter().all(Crgb::is_black));
    }

    #[test]
    fn fill_and_clear() {
        let mut c = Canvas::<4>::black();
        c.fill(Crgb::new(10, 20, 30));
        assert!(c.iter().all(|p| *p == Crgb::new(10, 20, 30)));
        c.clear();
        assert!(c.iter().all(Crgb::is_black));
    }

    #[test]
    fn indexing_and_slicing_go_through_deref() {
        let mut c = Canvas::<4>::black();
        c[2] = Crgb::new(1, 2, 3);
        assert_eq!(c[2], Crgb::new(1, 2, 3));
        assert_eq!(c.as_slice()[2], Crgb::new(1, 2, 3));

        // A sub-range is a valid canvas view — this is how segments will hand
        // effects their slice of the strip.
        let view: &mut [Crgb] = &mut c.as_mut_slice()[1..3];
        assert_eq!(view.len(), 2);
        view[0] = Crgb::new(9, 9, 9);
        assert_eq!(c[1], Crgb::new(9, 9, 9));
    }

    #[test]
    fn solid_and_from_pixels() {
        assert_eq!(
            Canvas::<2>::solid(Crgb::new(5, 6, 7)).as_slice(),
            Canvas::<2>::from_pixels([Crgb::new(5, 6, 7); 2]).as_slice()
        );
    }
}
