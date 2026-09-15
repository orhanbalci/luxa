//! The endpoint protocol: what to do with each request, as pure logic.
//!
//! A runtime owns the sockets and the engine. This module tells it how to
//! answer each HTTP request under `/json` and each WebSocket text frame, and
//! when to broadcast state to every client — so every transport behaves the
//! same, and all of it is testable without a network.

use core::fmt::{self, Write};

use luxa_msg::State;

use crate::json::{self, Info, Names, StateRequest};

/// An HTTP method the API distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// Read a document.
    Get,
    /// Change state.
    Post,
}

/// A JSON document the API sends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Document {
    /// The state document.
    State,
    /// The info document.
    Info,
    /// `{"state":…,"info":…}`.
    StateAndInfo,
    /// State, info, effect names and palette names.
    Everything,
    /// Effect names in id order.
    EffectNames,
    /// Palette names in id order.
    PaletteNames,
}

impl Document {
    /// Writes this document.
    pub fn write<W: Write, const SEGMENTS: usize, const NAME: usize>(
        self,
        out: &mut W,
        state: &State<SEGMENTS, NAME>,
        info: &Info<'_>,
        names: &impl Names,
    ) -> fmt::Result {
        match self {
            Self::State => json::write_state(out, state),
            Self::Info => json::write_info(out, info, state, names),
            Self::StateAndInfo => json::write_state_and_info(out, state, info, names),
            Self::Everything => json::write_everything(out, state, info, names),
            Self::EffectNames => json::write_effect_names(out, names),
            Self::PaletteNames => json::write_palette_names(out, names),
        }
    }
}

/// HTTP status codes the API replies with.
pub mod status {
    /// The request was handled.
    pub const OK: u16 = 200;
    /// The body was not a state request; reply `{"error":9}`.
    pub const BAD_REQUEST: u16 = 400;
    /// The endpoint is not implemented; reply `{"error":4}`.
    pub const NOT_IMPLEMENTED: u16 = 501;
}

/// What to do with an HTTP request under `/json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Reply `200` with this document.
    Read(Document),
    /// Parse the body as a state request — replying `400 {"error":9}` if it
    /// is not one — and apply its commands. Then reply `200`: with this
    /// document if the request asked for state (`"v":true`), otherwise with
    /// `{"success":true}`. A request that asked for state on a path with no
    /// document (`None`) is applied, then answered `501 {"error":4}`.
    Apply(Option<Document>),
    /// Reply `501 {"error":4}`.
    NotImplemented,
}

/// Routes an HTTP request, or returns `None` when `path` is outside the JSON
/// API (anything other than `/json` and `/json/…`). `path` excludes any query
/// string.
pub fn route(method: Method, path: &str) -> Option<Route> {
    if path != "/json" && !path.starts_with("/json/") {
        return None;
    }
    Some(match method {
        Method::Get => document_for(path).map_or(Route::NotImplemented, Route::Read),
        // Configuration is a separate surface, not a state request.
        Method::Post if contains(path, "cfg") => Route::NotImplemented,
        Method::Post => Route::Apply(document_for(path)),
    })
}

/// The document a `/json` path names.
///
/// Matching is by substring, in the reference implementation's order — which
/// is how `/json/effects` and `/json/palettes` reach the name lists. Endpoints
/// not implemented yet (nodes, palette data, effect data, networks,
/// configuration, pins, live pixels) and unknown paths have no document.
fn document_for(path: &str) -> Option<Document> {
    const UNIMPLEMENTED: [&str; 7] = ["nodes", "palx", "fxda", "net", "cfg", "pins", "live"];
    let found = |needle| contains(path, needle);

    if found("state") {
        Some(Document::State)
    } else if found("info") {
        Some(Document::Info)
    } else if found("si") {
        Some(Document::StateAndInfo)
    } else if found("nodes") {
        None
    } else if found("eff") {
        Some(Document::EffectNames)
    } else if UNIMPLEMENTED.iter().any(|needle| found(needle)) {
        None
    } else if found("pal") {
        Some(Document::PaletteNames)
    } else if path.len() > "/json/".len() {
        None
    } else {
        Some(Document::Everything)
    }
}

/// Whether `needle` occurs in `path` after its first character.
fn contains(path: &str, needle: &str) -> bool {
    path.find(needle).is_some_and(|at| at > 0)
}

/// What to do with a WebSocket text frame.
#[derive(Debug, Clone, PartialEq, Eq)]
// A request is a few kilobytes held briefly on a connection task; there is no
// allocator to box it.
#[allow(clippy::large_enum_variant)]
pub enum Frame<const SEGMENTS: usize, const NAME: usize> {
    /// An application-level ping: reply `pong`.
    Pong,
    /// `{"v":true}` alone: reply to this client with state and info.
    SendState,
    /// A state request: apply its commands, then [`ws_reply`] decides the
    /// answer.
    Apply(StateRequest<SEGMENTS, NAME>),
    /// Not a state request: no reply at all.
    Ignore,
}

/// Classifies a WebSocket text frame, parsing it in place.
pub fn ws_frame<const SEGMENTS: usize, const NAME: usize>(
    text: &mut [u8],
) -> Frame<SEGMENTS, NAME> {
    if text.first() == Some(&b'p') && text.len() < 10 {
        return Frame::Pong;
    }
    match json::parse_state(text) {
        Err(_) => Frame::Ignore,
        Ok(request) if request.is_verbose_only() => Frame::SendState,
        Ok(request) => Frame::Apply(request),
    }
}

/// The direct reply to a WebSocket client after applying its request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsReply {
    /// `{"success":true}`.
    Success,
    /// State and info.
    StateAndInfo,
}

/// The direct reply to a WebSocket request that was just applied, or `None`
/// when a broadcast is pending — the broadcast then answers every client,
/// this one included.
pub const fn ws_reply(verbose: bool, broadcast_pending: bool) -> Option<WsReply> {
    if broadcast_pending {
        None
    } else if verbose {
        Some(WsReply::StateAndInfo)
    } else {
        Some(WsReply::Success)
    }
}

/// The shortest time between two broadcasts of state to every client.
pub const BROADCAST_COOLDOWN_MS: u32 = 1000;

/// Decides when every WebSocket client is sent the current state.
///
/// A change schedules a broadcast; broadcasts go out at most once per
/// [`BROADCAST_COOLDOWN_MS`]. A verbose HTTP reply already carries the state
/// to its client, so it delays the broadcast by one cooldown.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Broadcast {
    pending: bool,
    last_ms: Option<u32>,
}

impl Broadcast {
    /// Nothing pending, nothing sent yet.
    pub const fn new() -> Self {
        Self {
            pending: false,
            last_ms: None,
        }
    }

    /// State changed in a way clients should hear about.
    pub fn changed(&mut self) {
        self.pending = true;
    }

    /// Whether a broadcast is waiting to go out.
    pub const fn is_pending(&self) -> bool {
        self.pending
    }

    /// Whether to broadcast at `now_ms`. When it returns `true` the broadcast
    /// counts as sent.
    pub fn due(&mut self, now_ms: u32) -> bool {
        if !self.pending {
            return false;
        }
        if let Some(last) = self.last_ms {
            if now_ms.wrapping_sub(last) < BROADCAST_COOLDOWN_MS {
                return false;
            }
        }
        self.pending = false;
        self.last_ms = Some(now_ms);
        true
    }

    /// A verbose HTTP reply carried the state at `now_ms`: every client still
    /// hears about it, one cooldown later.
    pub fn replied_over_http(&mut self, now_ms: u32) {
        self.pending = true;
        self.last_ms = Some(now_ms);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_outside_the_json_api_are_not_routed() {
        assert_eq!(route(Method::Get, "/"), None);
        assert_eq!(route(Method::Get, "/jsonx"), None);
        assert_eq!(route(Method::Post, "/state"), None);
    }

    #[test]
    fn documents_by_path() {
        let read = |path| route(Method::Get, path);
        assert_eq!(read("/json"), Some(Route::Read(Document::Everything)));
        assert_eq!(read("/json/"), Some(Route::Read(Document::Everything)));
        assert_eq!(read("/json/state"), Some(Route::Read(Document::State)));
        assert_eq!(read("/json/info"), Some(Route::Read(Document::Info)));
        assert_eq!(read("/json/si"), Some(Route::Read(Document::StateAndInfo)));
        assert_eq!(read("/json/eff"), Some(Route::Read(Document::EffectNames)));
        assert_eq!(
            read("/json/effects"),
            Some(Route::Read(Document::EffectNames))
        );
        assert_eq!(read("/json/pal"), Some(Route::Read(Document::PaletteNames)));
        assert_eq!(
            read("/json/palettes"),
            Some(Route::Read(Document::PaletteNames))
        );
    }

    #[test]
    fn unimplemented_and_unknown_paths() {
        for path in [
            "/json/nodes",
            "/json/palx",
            "/json/fxdata",
            "/json/net",
            "/json/cfg",
            "/json/pins",
            "/json/live",
            "/json/nope",
        ] {
            assert_eq!(
                route(Method::Get, path),
                Some(Route::NotImplemented),
                "{path}"
            );
        }
    }

    #[test]
    fn posts_apply_and_reply_with_the_path_s_document() {
        assert_eq!(
            route(Method::Post, "/json/state"),
            Some(Route::Apply(Some(Document::State)))
        );
        assert_eq!(
            route(Method::Post, "/json/si"),
            Some(Route::Apply(Some(Document::StateAndInfo)))
        );
        assert_eq!(
            route(Method::Post, "/json"),
            Some(Route::Apply(Some(Document::Everything)))
        );
        assert_eq!(route(Method::Post, "/json/nope"), Some(Route::Apply(None)));
        assert_eq!(
            route(Method::Post, "/json/cfg"),
            Some(Route::NotImplemented)
        );
    }

    fn frame(text: &str) -> Frame<4, 16> {
        let mut bytes = [0u8; 128];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        ws_frame(&mut bytes[..text.len()])
    }

    #[test]
    fn websocket_frames() {
        assert_eq!(frame("p"), Frame::Pong);
        assert_eq!(frame("ping"), Frame::Pong);
        assert_eq!(frame(r#"{"v":true}"#), Frame::SendState);
        assert_eq!(frame(r#"{"on":tru}"#), Frame::Ignore);
        assert_eq!(frame("hello"), Frame::Ignore);
        assert!(matches!(frame(r#"{"v":true,"bri":5}"#), Frame::Apply(r) if r.verbose));
        assert!(matches!(frame(r#"{"on":true}"#), Frame::Apply(r) if !r.verbose));
        assert!(
            matches!(frame(r#"{"pal":1234567890}"#), Frame::Apply(_)),
            "a long frame starting with p would still be JSON"
        );
    }

    #[test]
    fn websocket_replies() {
        assert_eq!(ws_reply(false, false), Some(WsReply::Success));
        assert_eq!(ws_reply(true, false), Some(WsReply::StateAndInfo));
        assert_eq!(ws_reply(true, true), None, "the broadcast answers");
    }

    #[test]
    fn broadcasts_wait_for_a_change() {
        let mut b = Broadcast::new();
        assert!(!b.due(5000));
        b.changed();
        assert!(b.is_pending());
        assert!(b.due(5000), "the first broadcast goes out at once");
        assert!(!b.is_pending());
        assert!(!b.due(5001));
    }

    #[test]
    fn broadcasts_are_spaced_by_the_cooldown() {
        let mut b = Broadcast::new();
        b.changed();
        assert!(b.due(1000));
        b.changed();
        assert!(!b.due(1999));
        assert!(b.due(2000));
    }

    #[test]
    fn a_verbose_http_reply_delays_the_broadcast() {
        let mut b = Broadcast::new();
        b.replied_over_http(10_000);
        assert!(b.is_pending());
        assert!(!b.due(10_500));
        assert!(b.due(11_000));
    }

    #[test]
    fn the_cooldown_survives_the_clock_wrap() {
        let mut b = Broadcast::new();
        b.changed();
        assert!(b.due(u32::MAX - 100));
        b.changed();
        assert!(!b.due(500), "600 ms later, across the wrap");
        assert!(b.due(1000));
    }
}
