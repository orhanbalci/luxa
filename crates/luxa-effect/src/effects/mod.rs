//! The effect implementations themselves.
//!
//! Adding an effect is a module here and a variant in
//! [`EffectKind`](crate::EffectKind) — nothing else changes.

mod rainbow;
mod solid;

pub use rainbow::Rainbow;
pub use solid::Solid;
