//! Luxa's control API, as pure logic.
//!
//! This crate turns request bytes into [`luxa_msg`] commands and state into
//! response bytes. It opens no sockets and runs no tasks: a runtime hands it
//! bodies and paths, and sends back what it produces.
//!
//! The API's first shape is compatible with existing LED controller clients —
//! the same keys, value forms and quirks — so those clients can drive Luxa
//! unchanged. Only this crate knows that shape; the engine sees commands.
//!
//! - [`json`] — the JSON format: parsing state requests into commands, and
//!   writing the state, info and catalogue documents.

#![no_std]
#![forbid(unsafe_code)]

mod grammar;
pub mod json;
