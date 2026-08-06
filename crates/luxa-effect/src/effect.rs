//! The shape every effect has.

use luxa_color::Crgb;

use crate::Ctx;

/// Renders one frame into a canvas view.
///
/// Implementors are concrete structs; the trait exists to give them a uniform
/// shape and to make each independently testable. Dispatch at runtime goes
/// through [`EffectKind`](crate::EffectKind), never through `dyn Effect`.
///
/// # Contract
///
/// - `view` is the effect's *entire* world. It is the pixel range a segment
///   assigned to this effect, already sliced; index 0 is the start of that
///   range, not of the strip. Write to all of it or part of it, but never
///   assume how long it is — including zero.
/// - `ctx` is the effect's *only* source of outside information. Do not read a
///   clock, an RNG seeded from one, or any global.
/// - `&mut self` is for genuine animation state (particle positions, a
///   twinkle's per-pixel phase). Effects that can be a pure function of
///   `ctx` — like [`Rainbow`](crate::effects::Rainbow) — should be, because
///   then a frame is fully reproducible from one `u32`.
/// - Never panic, never allocate, never block.
pub trait Effect {
    /// Draws this effect's frame for `ctx` into `view`.
    fn render(&mut self, view: &mut [Crgb], ctx: &Ctx);
}
