//! The effect implementations themselves.
//!
//! Slice 1 ships exactly one. Later slices add modules here and a variant to
//! [`EffectKind`](crate::EffectKind) — nothing else changes.

mod rainbow;

pub use rainbow::Rainbow;
