//! The shape every effect has.

use luxa_color::Crgb;

use crate::{Ctx, Params};

/// Renders one frame into a canvas view.
///
/// Implementors are concrete structs; the trait exists to give them a uniform
/// shape and to make each independently testable. Dispatch at runtime goes
/// through [`EffectKind`](crate::EffectKind), never through `dyn Effect`.
///
/// # Contract
///
/// - `view` is the effect's *entire* world. It is the pixel range a segment
///   assigned to this effect, already sliced — and already halved when the
///   segment is mirrored; index 0 is the start of that range, not of the
///   strip. Write to all of it or part of it, but never assume how long it is —
///   including zero.
/// - `view` keeps what this effect instance drew into it last time, and starts
///   black: an effect may draw over its previous frame, or leave it untouched
///   on a frame where nothing moves.
/// - `ctx` and `params` are the effect's *only* sources of outside
///   information: `ctx` says when, `params` says how. Do not read a clock, an
///   RNG seeded from one, or any global.
/// - `&mut self` is for genuine animation state (particle positions, a
///   twinkle's per-pixel phase). Effects that can be a pure function of their
///   inputs — like [`Rainbow`](crate::effects::Rainbow) — should be, because
///   then a frame is fully reproducible.
/// - Reversing, mirroring, opacity and brightness are applied around the
///   effect, never by it.
/// - Never panic, never allocate, never block.
pub trait Effect {
    /// Draws this effect's frame for `ctx` and `params` into `view`.
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx, params: &Params);
}
