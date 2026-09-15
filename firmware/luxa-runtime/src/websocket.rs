//! WebSocket ingress at `/ws`, and broadcasts of state to every client.
//!
//! A client is pushed state and info on connect, sends the same state requests
//! as the HTTP API, and hears about every change — its own and everyone
//! else's. When to broadcast and how to answer each message are decided by
//! `luxa_api::protocol`; this module moves the frames.

use core::cell::{Cell, RefCell};

use embassy_futures::select::select;
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::signal::Signal;
use embassy_sync::watch::Watch;
use embassy_time::{Instant, Timer};
use luxa_api::protocol::{self, Broadcast, Document, Frame, WsReply};
use luxa_msg::{ErrorCode, Seq};
use picoserve::futures::Either as Received;
use picoserve::io::{Read, Write};
use picoserve::response::IntoResponse;
use picoserve::response::ws::{Message, SocketRx, SocketTx, WebSocketCallback, WebSocketUpgrade};

use crate::channels::{self, QueueFull};
use crate::config::{MAX_REQUEST_BYTES, MAX_SEGMENTS, SEGMENT_NAME_LEN, WEBSOCKET_CLIENTS};
use crate::reply::Reply;

/// When the next broadcast goes out.
static SCHEDULE: Mutex<CriticalSectionRawMutex, RefCell<Broadcast>> =
    Mutex::new(RefCell::new(Broadcast::new()));

/// Wakes the broadcaster when something other than a state change schedules
/// a broadcast.
static RESCHEDULED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

/// The broadcast generation; every session watches it. A receiver slot per
/// client is also what caps the number of clients.
static BROADCASTS: Watch<CriticalSectionRawMutex, u32, WEBSOCKET_CLIENTS> = Watch::new();

/// Connected clients.
static CLIENTS: Mutex<CriticalSectionRawMutex, Cell<u8>> = Mutex::new(Cell::new(0));

/// How often a scheduled broadcast checks whether its cooldown has passed.
const COOLDOWN_POLL_MS: u64 = 50;

/// WebSocket close code: the server is overloaded, try again later.
const TRY_AGAIN_LATER: u16 = 1013;

/// Connected WebSocket clients.
pub fn clients() -> u8 {
    CLIENTS.lock(Cell::get)
}

/// A verbose HTTP reply carried the state to its client; every WebSocket
/// client still hears about it, one cooldown later.
pub fn replied_over_http() {
    SCHEDULE.lock(|schedule| schedule.borrow_mut().replied_over_http(now_ms()));
    RESCHEDULED.signal(());
}

/// `GET /ws`.
pub async fn upgrade(upgrade: WebSocketUpgrade) -> impl IntoResponse {
    upgrade.on_upgrade(Session)
}

/// Sends state to every client whenever a broadcast is due.
#[embassy_executor::task]
pub async fn broadcaster() {
    let sender = BROADCASTS.sender();
    let mut generation = 0u32;
    loop {
        select(channels::STATE_CHANGES.wait(), RESCHEDULED.wait()).await;

        loop {
            let (due, pending) = SCHEDULE.lock(|schedule| {
                let mut schedule = schedule.borrow_mut();
                // Changes that arrived while waiting ride on this broadcast.
                if channels::STATE_CHANGES.try_take().is_some() {
                    schedule.changed();
                }
                (schedule.due(now_ms()), schedule.is_pending())
            });
            if due {
                generation = generation.wrapping_add(1);
                sender.send(generation);
            }
            if !pending || due {
                break;
            }
            Timer::after_millis(COOLDOWN_POLL_MS).await;
        }
    }
}

/// One connected client.
struct Session;

impl WebSocketCallback for Session {
    async fn run<R: Read, W: Write<Error = R::Error>>(
        self,
        mut rx: SocketRx<R>,
        mut tx: SocketTx<W>,
    ) -> Result<(), W::Error> {
        let Some(mut broadcasts) = BROADCASTS.receiver() else {
            return tx.close(Some((TRY_AGAIN_LATER, "too many clients"))).await;
        };
        let _client = Client::join();
        // Broadcasts from before this client connected are not news to it.
        let _ = broadcasts.try_changed();

        tx.send_display(Reply::document(Document::StateAndInfo).await)
            .await?;

        let mut buffer = [0u8; MAX_REQUEST_BYTES];
        loop {
            let base = buffer.as_ptr() as usize;
            let message = match rx.next_message(&mut buffer, broadcasts.changed()).await? {
                Received::First(message) => message,
                Received::Second(_) => {
                    tx.send_display(Reply::document(Document::StateAndInfo).await)
                        .await?;
                    continue;
                }
            };
            match message {
                Ok(Message::Text(text)) => {
                    let start = text.as_ptr() as usize - base;
                    let end = start + text.len();
                    if let Some(reply) = answer(&mut buffer[start..end]).await {
                        tx.send_display(reply).await?;
                    }
                }
                Ok(Message::Ping(data)) => tx.send_pong(data).await?,
                Ok(Message::Close(_)) => return tx.close(None).await,
                Ok(Message::Binary(_) | Message::Pong(_)) => {}
                Err(_) => {
                    // Too long or malformed: the stream cannot be trusted to
                    // be at a message boundary any more.
                    tx.send_display(Reply::Error(ErrorCode::Json)).await?;
                    return tx.close(None).await;
                }
            }
        }
    }
}

/// A text message, classified and, if it is a state request, queued.
enum Incoming {
    Pong,
    SendState,
    Ignore,
    /// The command queue could not take the request.
    Busy,
    /// Queued; `seq` is the last command's number, if it had any.
    Queued {
        verbose: bool,
        seq: Option<Seq>,
    },
}

/// Classifies a text message and queues its commands.
///
/// Synchronous on purpose: the parsed request is kilobytes, and here it lives
/// on the stack for a moment instead of in the session's future.
fn receive(text: &mut [u8]) -> Incoming {
    match protocol::ws_frame::<MAX_SEGMENTS, SEGMENT_NAME_LEN>(text) {
        Frame::Pong => Incoming::Pong,
        Frame::SendState => Incoming::SendState,
        Frame::Ignore => Incoming::Ignore,
        Frame::Apply(request) => match channels::submit_all(|| request.commands()) {
            Err(QueueFull) => Incoming::Busy,
            Ok(seq) => Incoming::Queued {
                verbose: request.verbose,
                seq,
            },
        },
    }
}

/// The reply to a text message, if it gets one.
async fn answer(text: &mut [u8]) -> Option<Reply> {
    let (verbose, seq) = match receive(text) {
        Incoming::Pong => return Some(Reply::Text("pong")),
        Incoming::SendState => return Some(Reply::document(Document::StateAndInfo).await),
        Incoming::Ignore => return None,
        Incoming::Busy => return Some(Reply::Error(ErrorCode::BufferBusy)),
        Incoming::Queued { verbose, seq } => (verbose, seq),
    };
    if let Some(seq) = seq {
        channels::applied(seq).await;
    }
    let broadcast_pending = seq.is_some_and(channels::changed_since)
        || SCHEDULE.lock(|schedule| schedule.borrow().is_pending());

    match protocol::ws_reply(verbose, broadcast_pending)? {
        WsReply::Success => Some(Reply::Success),
        WsReply::StateAndInfo => Some(Reply::document(Document::StateAndInfo).await),
    }
}

/// Counts a client for as long as it is connected.
struct Client;

impl Client {
    fn join() -> Self {
        CLIENTS.lock(|count| count.set(count.get() + 1));
        Self
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        CLIENTS.lock(|count| count.set(count.get() - 1));
    }
}

/// Milliseconds since boot, wrapping.
fn now_ms() -> u32 {
    Instant::now().as_millis() as u32
}
