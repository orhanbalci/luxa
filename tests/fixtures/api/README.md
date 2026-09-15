# Control API fixtures

Request/response pairs for the control API's first, WLED-compatible shape
(see [`docs/wled/json-api.md`](../../../docs/wled/json-api.md) and step 1 of
[`json-api-plan.md`](../../../docs/wled/json-api-plan.md)).

## Provenance — read this first

**These fixtures are derived from WLED's source, not captured from a device.**
Every expected value was traced through the code at WLED `main` @ `06ae26d`
(2026-09-15); each fixture lists the functions it was derived from in `refs`
and a `confidence`:

- `high` — a direct read of the code path, no timing or allocation involved.
- `medium` — correct as far as the code shows, but depends on behaviour not
  fully traced (e.g. pixel buffer reallocation when a slot is reused).

When a real device becomes available, re-run each request against it and
replace `"provenance": "derived"` with `"captured"` fixture by fixture.

## The device every fixture assumes

A freshly flashed WLED with no `cfg.json`, no presets and no usermods:

| Property | Value | Source |
|---|---|---|
| Platform | ESP32 without PSRAM | `MAX_NUM_SEGMENTS` 32 (`FX.h`) |
| LEDs | 30 × WS2812 RGB on one bus | `DEFAULT_LED_COUNT`, `DEFAULT_LED_TYPE` (`const.h`) |
| Layout | 1D, one automatic segment `[0, 30)` | `resetSegments` / `makeAutoSegments` |
| Brightness | 128, last brightness 128 | `briS`, `briLast` (`wled.h`) |
| Transition | 750 ms (reported as `7`) | `transitionDelay` (`wled.h`) |
| Light capabilities | RGB only → `lc: 1`, colours reported as 3 channels | `Bus::hasWhite`, `refreshLightCapabilities` |
| Effect catalogue | WLED's (`fxcount` 220; index = effect id, gaps are `"RSVD"`) | `setupEffectData` |

Luxa's harness must configure the same layout (30 LEDs, RGB) and a catalogue
with at least WLED's effect and palette *counts* before replaying these.

## File format

Each `*.json` file holds `{ "provenance": …, "fixtures": [ … ] }`. A fixture:

```jsonc
{
  "name": "power-off-remembers-brightness",
  "keys": ["on", "bri"],              // Tier A keys exercised (coverage)
  "refs": ["json.cpp:deserializeState", "led.cpp:toggleOnOff"],
  "confidence": "high",
  "before": [ { "bri": 90 } ],        // bodies POSTed to /json/state first, in order; responses ignored
  "request": { "method": "POST", "path": "/json/state", "body": { "on": false, "v": true } },
  "response": { "status": 200, "match": "subset", "body": { … } },
  "after": { "match": "subset", "body": { … } }   // optional: expected GET /json/state afterwards
}
```

- `request.body_raw` replaces `body` when the body is deliberately not valid
  JSON. HTTP requests are sent with `Content-Type: application/json`.
- WebSocket fixtures use `request.transport: "ws"` with `frames` to send
  after connecting. `response.frames` lists what the client must receive, in
  order; each has a `delivery` — `"direct"` (a reply to this client),
  `"broadcast"` (to every client, within 1000 ms of the change) or `"none"`
  (nothing may arrive within 1500 ms). Frame `match` kinds are those below,
  plus `"text"` (the raw text frame equals `body`) and `"any"` (a frame must
  arrive; its content is not checked).

## Matching

- `"match": "exact"` — the document must equal `body` exactly (key order
  ignored).
- `"match": "subset"` — every key in `body` must be present and match; keys
  not mentioned are ignored. Arrays must have the **same length** and match
  element by element (elements are themselves subset-matched).
- Special values, usable anywhere in `body`:
  - `{ "$any": "number" | "string" | "boolean" | "object" | "array" }` — any
    value of that type (device-specific: MAC, uptime, version, counts that
    depend on the catalogue).
  - `{ "$absent": true }` — as an object member: the key must not be present.
  - `{ "$prefix": [ … ] }` — an array that starts with these elements.

## Tier A coverage

| Key / endpoint | Fixtures |
|---|---|
| `on` | `power.json` |
| `bri` | `power.json` |
| `transition`, `tt` | `power.json` |
| `v` | every POST fixture; `post-without-v-returns-success` |
| `seg.id`, `start`, `stop`, `len` | `segments.json` |
| `seg.on`, `seg.bri` | `segments.json` |
| `seg.col` | `colors.json` |
| `seg.fx`, `sx`, `ix`, `pal` | `segments.json` |
| `seg.rev`, `mi`, `sel`, `n` | `segments.json` |
| `seg.lc` | `endpoints.json` (`get-state-fresh`) |
| `/json`, `/json/state`, `/json/si`, `/json/info`, `/json/eff`, `/json/pal`, errors | `endpoints.json` |
| `/ws` connect, ping, `{"v":true}`, patches | `websocket.json` |
