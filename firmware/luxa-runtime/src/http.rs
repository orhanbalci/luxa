//! HTTP ingress.
//!
//! This module is allowed to know about HTTP because it is the IO tier —
//! sockets are its job. What it is *not* allowed to do is decide anything.
//!
//! Every request under `/json` goes the same way: `luxa_api::protocol` says
//! what the path means, `luxa_api::json` turns a body into commands, and the
//! commands go onto the same queue every other ingress uses. What they *mean*
//! is decided in `luxa-core`, which has never heard of HTTP.

use core::convert::Infallible;
use core::str::FromStr;

use luxa_api::json;
use luxa_api::protocol::{self, Method, Route, status};
use luxa_msg::{ErrorCode, Seq};
use picoserve::ResponseSent;
use picoserve::io::{Read, Write};
use picoserve::request::Request;
use picoserve::response::{Content, IntoResponse, ResponseWriter, StatusCode};
use picoserve::routing::{MethodHandlerService, PathRouter, get, parse_path_segment};

use crate::channels::{self, QueueFull};
use crate::config::{MAX_SEGMENTS, SEGMENT_NAME_LEN};
use crate::reply::Reply;
use crate::websocket;

/// Builds the route table.
///
/// The concrete router type is deliberately never named. Each web task builds
/// its own and keeps it on the stack, which avoids needing `impl Trait` in a
/// type alias — still unstable — for what is a zero-cost tree of unit structs.
pub fn router() -> picoserve::Router<impl PathRouter> {
    picoserve::Router::new()
        .route("/", get(index))
        .route_service("/json", JsonApi)
        .route_service(("/json", parse_path_segment::<AnySegment>()), JsonApi)
        .route("/ws", get(websocket::upgrade))
}

/// Matches any one path segment; the API reads the whole path itself.
struct AnySegment;

impl FromStr for AnySegment {
    type Err = Infallible;

    fn from_str(_: &str) -> Result<Self, Infallible> {
        Ok(Self)
    }
}

/// `/json` and everything under it, for every method.
///
/// A service rather than handler functions, so the body is parsed in place in
/// the connection's buffer instead of being copied into the handler.
struct JsonApi;

impl<PathParameters> MethodHandlerService<(), PathParameters> for JsonApi {
    async fn call_method_handler_service<R: Read, W: ResponseWriter<Error = R::Error>>(
        &self,
        _state: &(),
        _path_parameters: PathParameters,
        method: &str,
        mut request: Request<'_, R>,
        response_writer: W,
    ) -> Result<ResponseSent, W::Error> {
        let path = request.parts.path().encoded();
        let method = match method {
            "GET" => Method::Get,
            "POST" => Method::Post,
            _ => {
                let connection = request.body_connection.finalize().await?;
                return not_implemented()
                    .write_to(connection, response_writer)
                    .await;
            }
        };

        let (status, reply) = match protocol::route(method, path).unwrap_or(Route::NotImplemented) {
            Route::Read(document) => (ok(), Reply::document(document).await),
            Route::NotImplemented => not_implemented(),
            Route::Apply(document) => {
                let submitted = match request.body_connection.body().read_all().await {
                    Ok(body) => submit(body),
                    Err(error) => {
                        let connection = request.body_connection.finalize().await?;
                        return error.write_to(connection, response_writer).await;
                    }
                };
                apply(submitted, document).await
            }
        };

        let connection = request.body_connection.finalize().await?;
        (status, reply).write_to(connection, response_writer).await
    }
}

/// A request body, parsed and queued.
enum Submitted {
    /// The body was not a state request.
    Invalid(ErrorCode),
    /// The command queue could not take the request.
    Busy,
    /// Queued; `seq` is the last command's number, if it had any.
    Queued { verbose: bool, seq: Option<Seq> },
}

/// Parses a body and queues its commands.
///
/// Synchronous on purpose: the parsed request is kilobytes, and here it lives
/// on the stack for a moment instead of in the connection's future.
fn submit(body: &mut [u8]) -> Submitted {
    match json::parse_state::<MAX_SEGMENTS, SEGMENT_NAME_LEN>(body) {
        Err(error) => Submitted::Invalid(error.code()),
        Ok(request) => match channels::submit_all(|| request.commands()) {
            Err(QueueFull) => Submitted::Busy,
            Ok(seq) => Submitted::Queued {
                verbose: request.verbose,
                seq,
            },
        },
    }
}

/// Answers an applied request: after the engine has published its result,
/// with the path's document if the request asked for state.
async fn apply(submitted: Submitted, document: Option<protocol::Document>) -> (StatusCode, Reply) {
    let verbose = match submitted {
        Submitted::Invalid(code) => {
            return (StatusCode::new(status::BAD_REQUEST), Reply::Error(code));
        }
        // Backpressure: the engine is not keeping up, and holding the socket
        // open would just move the backlog into the TCP stack.
        Submitted::Busy => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                Reply::Error(ErrorCode::BufferBusy),
            );
        }
        Submitted::Queued { verbose, seq } => {
            if let Some(seq) = seq {
                channels::applied(seq).await;
            }
            verbose
        }
    };

    match (verbose, document) {
        (false, _) => (ok(), Reply::Success),
        (true, Some(document)) => {
            let reply = Reply::document(document).await;
            websocket::replied_over_http();
            (ok(), reply)
        }
        (true, None) => not_implemented(),
    }
}

const fn ok() -> StatusCode {
    StatusCode::new(status::OK)
}

fn not_implemented() -> (StatusCode, Reply) {
    (
        StatusCode::new(status::NOT_IMPLEMENTED),
        Reply::Error(ErrorCode::NotImplemented),
    )
}

/// Serves the control page.
async fn index() -> Html {
    Html(INDEX_HTML)
}

/// `text/html` wrapper — picoserve types a bare `&str` as `text/plain`.
struct Html(&'static str);

impl Content for Html {
    fn content_type(&self) -> &'static str {
        "text/html; charset=utf-8"
    }

    fn content_length(&self) -> usize {
        self.0.len()
    }

    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error> {
        self.0.write_content(writer).await
    }
}

/// The control page: a toggle and a slider, and nothing else.
///
/// It speaks the same JSON API as every other client: it reads `/json/si`,
/// posts state requests to `/json/state`, and follows changes made elsewhere
/// over `/ws`. Inlined as a `&'static str` because there is no filesystem — it
/// lives in flash and is served straight out of it.
const INDEX_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Luxa</title>
<style>
  :root { color-scheme: dark light; }
  body { font: 16px/1.5 system-ui, sans-serif; max-width: 22rem;
         margin: 4rem auto; padding: 0 1rem; }
  h1 { font-size: 1.3rem; letter-spacing: .02em; }
  .row { display: flex; align-items: center; justify-content: space-between;
         gap: 1rem; margin: 1.5rem 0; }
  input[type=range] { width: 100%; }
  button { font: inherit; padding: .4rem 1.1rem; border-radius: 999px;
           border: 1px solid currentColor; background: none; color: inherit;
           cursor: pointer; }
  button[aria-pressed=true] { background: currentColor; }
  button[aria-pressed=true] span { filter: invert(1); }
  output { font-variant-numeric: tabular-nums; opacity: .7; }
</style>
</head>
<body>
<h1>Luxa</h1>

<div class="row">
  <label for="power">Power</label>
  <button id="power" aria-pressed="true"><span>on</span></button>
</div>

<div class="row">
  <label for="brightness">Brightness</label>
  <output id="value">128</output>
</div>
<input id="brightness" type="range" min="1" max="255" value="128">

<script>
const power = document.getElementById('power');
const brightness = document.getElementById('brightness');
const value = document.getElementById('value');

const send = request =>
  fetch('/json/state', { method: 'POST', body: JSON.stringify(request) });

// `bri` stays at the last level while off, so the slider keeps it.
function paint(state) {
  power.setAttribute('aria-pressed', state.on);
  power.firstElementChild.textContent = state.on ? 'on' : 'off';
  brightness.value = state.bri;
  value.textContent = state.bri;
}

power.addEventListener('click', () => {
  const on = power.getAttribute('aria-pressed') !== 'true';
  paint({ on, bri: Number(brightness.value) });
  send({ on });
});

// Fire on every drag step so the strip tracks the slider live.
brightness.addEventListener('input', () => {
  const bri = Number(brightness.value);
  paint({ on: true, bri });
  send({ on: true, bri });
});

fetch('/json/si').then(r => r.json()).then(si => paint(si.state)).catch(() => {});

// Changes made by other clients arrive as broadcasts.
function follow() {
  const socket = new WebSocket(`ws://${location.host}/ws`);
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.state) paint(message.state);
  };
  socket.onclose = () => setTimeout(follow, 3000);
}
follow();
</script>
</body>
</html>
"#;
