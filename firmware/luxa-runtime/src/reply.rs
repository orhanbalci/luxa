//! Replies, written straight to the socket.
//!
//! The largest document is several kilobytes, more than any buffer here. A
//! reply is measured for its length, then streamed in windows through a small
//! chunk, so no response buffer is ever held.
//!
//! A document must read the same on every pass, so it is written from a
//! snapshot copied once into a leased slot. The reply holds only the lease:
//! a connection's future carries a pointer, not kilobytes of state copied into
//! every layer of the HTTP stack.

use core::fmt::{self, Display, Write as _};

use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::{Mutex, MutexGuard};
use luxa_api::json::{self, Measure, Window};
use luxa_api::protocol::Document;
use luxa_msg::ErrorCode;
use picoserve::io::Write;
use picoserve::response::Content;

use crate::channels::{SNAPSHOTS, State};
use crate::device::{Facts, Registry};
use crate::net::WEB_TASKS;

/// Bytes streamed per window.
const CHUNK: usize = 1024;

/// A slot holding the snapshot a document is written from.
type Slot = Mutex<CriticalSectionRawMutex, Option<State>>;

/// One slot per web task: a connection writes one reply at a time, so a slot
/// is always free.
static SLOTS: [Slot; WEB_TASKS] = [const { Mutex::new(None) }; WEB_TASKS];

/// A snapshot, leased for as long as a document is being written from it.
pub struct Snapshot(MutexGuard<'static, CriticalSectionRawMutex, Option<State>>);

impl Snapshot {
    /// Copies the latest published state into a free slot.
    async fn take() -> Self {
        let mut guard = match SLOTS.iter().find_map(|slot| slot.try_lock().ok()) {
            Some(guard) => guard,
            None => SLOTS[0].lock().await,
        };
        fill(&mut guard);
        Self(guard)
    }
}

/// Copies the latest state in place. Kept out of any future, so the copy
/// passes through the stack only.
fn fill(slot: &mut Option<State>) {
    *slot = SNAPSHOTS.try_get();
}

/// A reply to a request or a WebSocket message.
pub enum Reply {
    /// A document, rendered from one snapshot.
    Document {
        document: Document,
        snapshot: Snapshot,
        facts: Facts,
    },
    /// `{"success":true}`.
    Success,
    /// `{"error":<code>}`.
    Error(ErrorCode),
    /// A literal text reply.
    Text(&'static str),
}

impl Reply {
    /// `document`, rendered from the latest published state.
    pub async fn document(document: Document) -> Self {
        Self::Document {
            document,
            snapshot: Snapshot::take().await,
            facts: Facts::now(),
        }
    }
}

impl Display for Reply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Document {
                document,
                snapshot,
                facts,
            } => match snapshot.0.as_ref() {
                Some(state) => document.write(f, state, &facts.info(), &Registry),
                // Nothing published yet; the engine publishes before the
                // network starts, so this is not expected.
                None => json::write_error(f, ErrorCode::BufferBusy),
            },
            Self::Success => json::write_success(f),
            Self::Error(code) => json::write_error(f, *code),
            Self::Text(text) => f.write_str(text),
        }
    }
}

impl Content for Reply {
    fn content_type(&self) -> &'static str {
        match self {
            Self::Text(_) => "text/plain; charset=utf-8",
            _ => "application/json",
        }
    }

    fn content_length(&self) -> usize {
        let mut measure = Measure::default();
        let _ = write!(measure, "{self}");
        measure.0
    }

    async fn write_content<W: Write>(self, mut writer: W) -> Result<(), W::Error> {
        let mut chunk = [0u8; CHUNK];
        let mut offset = 0;
        loop {
            let mut window = Window::new(&mut chunk, offset);
            let _ = write!(window, "{self}");
            writer.write_all(window.filled()).await?;
            offset += window.filled().len();
            if window.is_complete() || window.filled().is_empty() {
                return Ok(());
            }
        }
    }
}
