# Luxa

An LED controller for ESP32, built as a Rust workspace where each crate owns
exactly one concern.

**Slice 1 — "it lights up, and I can control it from a browser."** One WS2812
strip, one animated effect, a power toggle and a brightness slider, served from
the device itself and demoed in [Wokwi](https://wokwi.com).

## The frame path

```
POST /json/state ─┐   luxa-api     numbered
                  ├─▶ Command ──▶ Envelope ──▶ luxa-core ──▶ State ──▶ watch
/ws text frame ───┘               (channel)   (single writer)             │
                                                                          ▼
   luxa-segment ──▶ luxa-canvas ──▶ luxa-output ──▶ luxa-wire ──▶ driver ──▶ strip
   which effect      the pixels      brightness      WS2812 bytes    RMT
   renders where
```

## Crates

The workspace is split in two, and the split is load-bearing: the root
workspace holds portable crates that build and test on your laptop, and
`firmware/` is a separate workspace targeting `riscv32imc-unknown-none-elf`.
Nothing in `crates/` *can* reach for a HAL, because the HAL is not in that
dependency graph at all.

### Portable (`crates/`) — `no_std`, allocation-free, laptop-testable

| Crate | Owns | Would it move if… |
|---|---|---|
| `luxa-color` | Pixel types (re-exported from [`color8`]), `ColorOrder`, and `Rgbw` for configured colours | …you swapped chipsets? The *enum* is vocabulary; applying it is `luxa-wire`'s job. |
| `luxa-canvas` | The framebuffer — a fixed-length buffer of pixels | …ESP32→RP2350? No. WS2812→APA102? No. |
| `luxa-effect` | The `Effect` trait, the `Ctx` clock boundary, the effect registry | …you changed transport or driver? No — phase math is phase math. |
| `luxa-segment` | Which effect renders into which pixels | …you changed chips? No. |
| `luxa-output` | Brightness, applied to a finished frame (off *is* brightness zero) | …you swapped the driver? No — so it must not live *in* the driver. |
| `luxa-wire` | WS2812 framing: pixels → bytes, plus the line timing | …ESP32→RP2350? No. WS2812→APA102? **Yes** — hence the isolation. |
| `luxa-msg` | `Command`, `Envelope` and `State` (segments, brightness, sequence numbers): the transport-neutral vocabulary, with capacities chosen by the application | …n/a. Depends only on `luxa-color`. |
| `luxa-core` | The drain-then-publish engine, sole writer of `State` | …you changed transport? No. It has never heard of HTTP. |
| `luxa-api` | The control API as pure logic: parsing request bodies into commands, writing state, info and catalogue documents, and the endpoint protocol (routing, WebSocket replies, broadcast cooldown) | …you changed transport? No — it never touches a socket. …you changed the wire format? **Yes** — this is the only crate that knows it. |

### IO tier (`firmware/`) — the parts that are *supposed* to move

| Crate | Owns |
|---|---|
| `luxa-driver-esp-rmt` | The RMT sink. The only crate touching silicon. |
| `luxa-runtime` | Executor, peripherals, Wi-Fi, HTTP, and the task graph. Glue only. |

## Two decisions this slice fixes

**Effect dispatch is an enum, not `dyn`.** `EffectKind` is a plain enum sized
to its largest variant, matched statically — no vtable, no allocator. Slice 1's
one-variant enum is trivial on purpose: establishing the pattern now means the
slice with forty effects adds forty variants and no new architecture.

**Time arrives through `Ctx`, never ambiently.** The render task narrows the
platform's `u64` µs clock to a wrapping `u32` ms in exactly one place and
passes it down. Effects cannot read a clock — `Ctx`'s field is private and
`from_millis` is the only constructor. A frame is therefore reproducible from
one number, which is what makes `a_frame_is_reproducible_from_its_timestamp_alone`
in the pipeline test possible at all.

## Running the tests

Six of the seven behavioural crates are provably correct before anything
touches a simulator:

```sh
cargo test --workspace
```

That includes `crates/luxa-segment/tests/pipeline.rs`, which drives the whole
portable chain — engine → state → compositor → canvas → output → wire — and
asserts on the bytes that would go down the data line.

## Running the demo

Needs [`wokwi-cli`](https://docs.wokwi.com/wokwi-ci/cli-installation) and a
`WOKWI_CLI_TOKEN`, or the Wokwi VS Code extension.

```sh
cd firmware
cargo build --release            # riscv32imc, via .cargo/config.toml
wokwi-cli .                      # or: F1 → "Wokwi: Start Simulator"
```

The firmware associates with `Wokwi-GUEST`, takes a DHCP lease, and prints the
URL to open. `wokwi.toml` forwards `localhost:8080` to port 80 on the simulated
device, so the control page is at <http://localhost:8080/>.

The strip starts animating before the network comes up — networking failing
costs you the controls, not the light.

### Control API

The API uses the same keys and endpoints as existing LED controller clients;
[`docs/wled/json-api.md`](docs/wled/json-api.md) describes it in full.

| Route | Method | Returns |
|---|---|---|
| `/` | GET | The control page |
| `/json` | GET | state, info, effect names and palette names |
| `/json/state`, `/json/si`, `/json/info` | GET | state, `{state, info}`, info |
| `/json/eff`, `/json/pal` | GET | effect names, palette names |
| `/json`, `/json/state`, `/json/si` | POST | applies a state request; `{"success":true}`, or the path's document with `"v":true` |
| `/ws` | WebSocket | `{state, info}` on connect and on every change; accepts the same state requests |

```sh
curl -X POST -d '{"on":false}'        http://localhost:8080/json/state
curl -X POST -d '{"bri":32,"v":true}' http://localhost:8080/json/si
curl http://localhost:8080/json/si
```

## What Wokwi does and does not prove

It runs the real `esp-hal` firmware, so it validates the driver logic, the
task graph, the HTTP surface and the frame pipeline end to end. It does **not**
validate electrical behaviour — current draw, timing margin against a real
strip's tolerances, or supply droop. Every hardware-derived number is a runtime
`LedProfile` parameter (`firmware/luxa-runtime/src/config.rs`) rather than a
constant, so the hardware-truth gate can move to the slice that first needs a
physical strip without any of these crates changing.

## Deliberately not in slice 1

Effect selection, palettes, multiple segments, presets, persistence, an event
bus, MQTT, and the realtime protocol bypass. Each is its own later slice with
its own crate-responsibility ledger.

[`color8`]: https://crates.io/crates/color8
