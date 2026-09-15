//! Effects: pure functions from (time, pixel range, settings) to colour.
//!
//! This crate is where animation math lives, and it is deliberately starved of
//! everything else. It cannot read a clock, allocate, block, or reach a
//! peripheral: it depends on [`luxa_color`] for pixels and on
//! [`smart_leds_fx`], which supplies most of its effects. That is what makes an
//! effect testable on a laptop by asserting on pixels.
//!
//! Three structural decisions are fixed here, all cheap now and expensive to
//! change once many effects exist:
//!
//! # 1. Dispatch is an enum, not `dyn`
//!
//! Effects are stored and dispatched through the [`EffectKind`] registry: a
//! plain enum, sized to its largest variant, matched statically. There is no
//! trait object, no vtable and no allocation anywhere in the render path.
//!
//! The [`Effect`] trait still exists, because it is what makes each effect
//! independently testable and gives them a uniform shape — but it is only ever
//! called through a concrete type. Adding an effect is one new struct plus one
//! new variant; the dispatch mechanism does not change.
//!
//! # 2. Time arrives through [`Ctx`], never ambiently
//!
//! An effect is a function of its inputs. It never asks the system what time
//! it is; it is *told*, via [`Ctx::now_ms`]. The render task narrows the
//! platform's monotonic clock (a `u64` of microseconds) to a wrapping `u32` of
//! milliseconds in exactly one place, and every effect downstream sees that
//! same value for a given frame.
//!
//! That buys three things: a frame is reproducible from a single number, a
//! test can render frame 5000 without waiting five seconds, and every segment
//! in a frame is animated against an identical clock instead of drifting
//! apart by however long the frame took to render.
//!
//! # 3. Every effect has a numeric id and a descriptor
//!
//! [`EffectKind::id`] is a stable number — part of the control API's wire
//! contract — and [`EffectKind::descriptor`] is a compact string naming the
//! effect and the controls a UI should show for it (see [`Descriptor`]).
//! Settings a segment carries reach the effect as [`Params`].

#![no_std]
#![forbid(unsafe_code)]

mod ctx;
mod descriptor;
mod effect;
pub mod effects;
mod kind;
mod palette;
mod params;

pub use ctx::Ctx;
pub use descriptor::{Control, Descriptor, Flags};
pub use effect::Effect;
pub use kind::EffectKind;
pub use palette::{PALETTES, Palette};
pub use params::Params;
