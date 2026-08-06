//! HTTP ingress.
//!
//! This module is allowed to know about HTTP because it is the IO tier —
//! sockets are its job. What it is *not* allowed to do is decide anything.
//!
//! Every handler below has the same shape and the same ceiling: parse the
//! request, produce a [`Command`], put it on the channel. None of them touch
//! `AppState`, scale a pixel, or consult the strip. `POST /brightness` with a
//! body of `128` becomes `Command::Brightness(128)` and the handler is done;
//! what that *means* is decided in `luxa-core`, which has never heard of HTTP.
//!
//! That thinness is the whole reason a later slice can add MQTT or a physical
//! button without touching the engine: they are simply different producers of
//! the same `Command` onto the same channel.

use core::fmt::Write as _;

use heapless::String;
use luxa_msg::Command;
use picoserve::io::Write;
use picoserve::response::{Content, StatusCode};
use picoserve::routing::{get, post};

use crate::channels::{COMMANDS, SNAPSHOTS};

/// Builds the route table.
///
/// Four routes: one page to look at, two to change something, one to read the
/// current state back. Adding effect selection in slice 2 is one more line
/// here and one more `Command` variant — no new architecture.
///
/// The concrete router type is deliberately never named. Each web task builds
/// its own and keeps it on the stack, which avoids needing `impl Trait` in a
/// type alias — still unstable — for what is a zero-cost tree of unit structs.
pub fn router() -> picoserve::Router<impl picoserve::routing::PathRouter> {
    picoserve::Router::new()
        .route("/", get(index))
        .route("/state", get(state))
        .route("/power", post(set_power))
        .route("/brightness", post(set_brightness))
}

/// Serves the control page.
async fn index() -> Html {
    Html(INDEX_HTML)
}

/// Reports the currently published snapshot.
///
/// This reads the same `Watch` the renderer reads — it observes published
/// state rather than reaching into the engine, which is the difference between
/// a reporter and a second writer.
async fn state() -> Json<64> {
    let snapshot = SNAPSHOTS.try_get().unwrap_or_default();

    let mut body: String<64> = String::new();
    // Writing into a fixed buffer cannot fail for a payload this size, and a
    // truncated status report is not worth taking the connection down over.
    let _ = write!(
        body,
        r#"{{"power":{},"brightness":{}}}"#,
        snapshot.power, snapshot.brightness
    );

    Json(body)
}

/// `POST /power` with a body of `on`/`off` (also accepts `true`/`false`, `1`/`0`).
async fn set_power(body: Body) -> (StatusCode, &'static str) {
    let Some(on) = parse_power(body.trim()) else {
        return (StatusCode::BAD_REQUEST, "expected on|off\n");
    };

    send(Command::Power(on))
}

/// `POST /brightness` with a decimal body in `0..=255`.
async fn set_brightness(body: Body) -> (StatusCode, &'static str) {
    let Ok(level) = body.trim().parse::<u8>() else {
        return (StatusCode::BAD_REQUEST, "expected an integer 0-255\n");
    };

    send(Command::Brightness(level))
}

/// Request bodies here are a handful of bytes. An owned, fixed-capacity
/// buffer rather than a borrowed `&str` because picoserve's handler bound
/// quantifies the extractor over all lifetimes, which a borrow cannot satisfy.
type Body = String<32>;

fn parse_power(body: &str) -> Option<bool> {
    match body {
        "on" | "true" | "1" => Some(true),
        "off" | "false" | "0" => Some(false),
        _ => None,
    }
}

/// Enqueues a command, or reports backpressure.
///
/// Deliberately non-blocking. If the queue is full the engine is not keeping
/// up, and the honest answer is 503 — holding the socket open would just move
/// the backlog from the queue into the TCP stack.
fn send(command: Command) -> (StatusCode, &'static str) {
    match COMMANDS.try_send(command) {
        Ok(()) => (StatusCode::OK, "ok\n"),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "command queue full\n"),
    }
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

/// `application/json` wrapper.
struct Json<const N: usize>(String<N>);

impl<const N: usize> Content for Json<N> {
    fn content_type(&self) -> &'static str {
        "application/json"
    }

    fn content_length(&self) -> usize {
        self.0.len()
    }

    async fn write_content<W: Write>(self, writer: W) -> Result<(), W::Error> {
        self.0.as_str().write_content(writer).await
    }
}

/// The control page: a toggle and a slider, and nothing else.
///
/// Inlined as a `&'static str` because there is no filesystem — it lives in
/// flash and is served straight out of it.
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
<input id="brightness" type="range" min="0" max="255" value="128">

<script>
const power = document.getElementById('power');
const brightness = document.getElementById('brightness');
const value = document.getElementById('value');

const post = (path, body) =>
  fetch(path, { method: 'POST', body: String(body) });

function paint(state) {
  power.setAttribute('aria-pressed', state.power);
  power.firstElementChild.textContent = state.power ? 'on' : 'off';
  brightness.value = state.brightness;
  value.textContent = state.brightness;
}

power.addEventListener('click', () => {
  const on = power.getAttribute('aria-pressed') !== 'true';
  paint({ power: on, brightness: Number(brightness.value) });
  post('/power', on ? 'on' : 'off');
});

// Fire on every drag step so the strip tracks the slider live.
brightness.addEventListener('input', () => {
  value.textContent = brightness.value;
  post('/brightness', brightness.value);
});

fetch('/state').then(r => r.json()).then(paint).catch(() => {});
</script>
</body>
</html>
"#;
