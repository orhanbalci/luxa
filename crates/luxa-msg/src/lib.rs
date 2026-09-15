//! The vocabulary every Luxa ingress speaks, and the state it talks about.
//!
//! This crate is pure data: it knows nothing about transports, pixels or
//! executors. An HTTP handler, an MQTT subscriber, a physical button and a
//! scheduler are all just different producers of the same [`Command`], and
//! [`luxa-core`](../luxa_core/index.html) consumes them without ever learning
//! which one sent it.
//!
//! The two halves of the seam:
//!
//! - **In:** a [`Command`] is intent — a [`GlobalPatch`] for the whole fixture
//!   or a [`SegmentPatch`] for segments. An [`Envelope`] wraps it with its
//!   place in the input order, whether a reply is awaited, and its [`Origin`].
//! - **Out:** [`State`] is everything the fixture currently is. The engine owns
//!   one and publishes copies; the renderer and every API read those copies.
//!
//! # Capacity is the application's choice
//!
//! [`State`] is `State<SEGMENTS, NAME>`: how many segments it can hold and how
//! long a segment name may be are const generic parameters, not constants in
//! this crate. A firmware for a small MCU and a desktop simulator can use the
//! same types at different sizes, and nothing here allocates.

#![no_std]
#![forbid(unsafe_code)]

mod command;
mod name;
mod patch;
mod state;
mod value;

pub use command::{Command, Envelope, Origin, Origins};
pub use luxa_color::Rgbw;
pub use name::Name;
pub use patch::{
    BoolOp, Bounds, ColorSpec, Direction, GlobalPatch, SegmentPatch, SegmentTarget, U8Op,
};
pub use state::{Layout, Segment, State};
pub use value::{EffectId, ErrorCode, LightCaps, PaletteId, Seq, TransitionTime};
