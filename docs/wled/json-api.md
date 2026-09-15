# WLED JSON API — analysis

What WLED exposes to apps, and what each request changes about LED state.
This is the compatibility surface Luxa targets; the implementation plan lives
in [`json-api-plan.md`](json-api-plan.md).

**Source:** [`wled/WLED`](https://github.com/wled/WLED) `main` @ `06ae26d`
(2026-09-15). Files read: `wled00/json.cpp`, `ws.cpp`, `wled_server.cpp`
(`/json` handler), `led.cpp`, `util.cpp` (`getVal`/`parseNumber`), `const.h`,
`FX_fcn.cpp` (`Segment::setMode`), `FX.cpp` (effect metadata), and
`data/index.js` (the bundled UI, as the reference client).

---

## 0. The one-sentence model

**There is a single state document, and every transport funnels into one
function.** HTTP POST, WebSocket text frames, presets, buttons/IR, MQTT JSON
and playlists all call `deserializeState(root, callMode)` (`json.cpp:370`).
That function *is* the command surface; `serializeState()` (`json.cpp:647`) is
the snapshot surface.

## 1. Transports

| Endpoint | Method | Returns |
|---|---|---|
| `/json` | GET | `{state, info, effects[], palettes[]}` |
| `/json/state` | GET / POST | state (POST applies a patch) |
| `/json/si` | GET / POST | `{state, info}` — **the bundled UI uses this for everything** |
| `/json/info` | GET | device info (read-only) |
| `/json/eff` | GET | effect names `["Solid","Blink",…]` |
| `/json/fxdata` | GET | effect metadata strings (streamed in chunks) |
| `/json/pal` | GET | palette names |
| `/json/palx?page=N` | GET | palette colour stops, 8 per page (5 on ESP8266) |
| `/json/nodes` | GET | other WLED devices discovered on the LAN |
| `/json/net` | GET | WiFi scan results |
| `/json/cfg` | GET / POST | configuration (PIN-protected) — separate surface from state |
| `/json/pins` | GET | GPIO ownership and capabilities |
| `/json/live` | GET | `{"leds":["RRGGBB",…],"n":step}` (build flag `WLED_ENABLE_JSONLIVE`) |
| `/ws` | WebSocket | see below |
| `/presets.json` | GET | presets file, served raw from the filesystem |

Routing in `serveJson()` (`json.cpp:1350`) is by `url.indexOf(...)`, so the
match order matters (`state` → `info` → `si` → `nodes` → `eff` → `palx` →
`fxda` → `net` → `cfg` → `pins` → `live` → `pal`); anything else longer than
`/json` returns 501 with `ERR_NOT_IMPL`.

### POST `/json[/state|/si]` (`wled_server.cpp:422`)

- Body parse failure → HTTP 400, `{"error":9}`.
- A `pin` key is checked against the settings PIN.
- Response is `{"success":true}` unless the body contains `"v":true`, in which
  case the response is the full document for that URL (via `serveJson`), and
  the WS broadcast is deferred by one cooldown so the requesting client isn't
  sent the same data twice.
- If the shared JSON buffer is locked, the response is deferred, not failed.

### WebSocket `/ws` (`ws.cpp`)

**On connect:** server pushes `{state, info}`.

**Client → server, text:**

| Frame | Effect |
|---|---|
| `"p…"` (len < 10) | reply `"pong"` — app-level heartbeat |
| `{"v":true}` (only key) | reply `{state, info}` to this client |
| `{"lv":true\|false}` | subscribe / unsubscribe this client to the live pixel stream (one subscriber at a time) |
| any other object | `deserializeState()`; reply `{"success":true}`, or full state if `v` |
| fragmented message | `{"error":9}` — split frames are not supported |

**Client → server, binary:** first byte selects a realtime protocol, the rest
is the raw packet: `0` E1.31, `1` Art-Net, `2` DDP. Realtime-over-WebSocket.

**Server → client, binary live frames** (every 40 ms, only if the client's
send queue is empty):

```
1D:  'L', 0x01, r,g,b, r,g,b, …                 (≤1024 px, every n-th if more)
2D:  'L', 0x02, width, height, r,g,b, …         (downsampled by 2 or 4 if large)
```

White is folded into RGB with a saturating add; brightness 0 sends black.

**Broadcast:** after any state change, `updateInterfaces()` (`led.cpp:141`)
pushes `{state, info}` to *all* WS clients, rate-limited by
`INTERFACE_UPDATE_COOLDOWN`, then publishes MQTT and updates Alexa. On OOM the
server closes all sockets with code 1013.

### Reference client behaviour (`index.js:1756`)

- Uses WS when open; falls back to `POST /json/si` if WS is down, the payload
  is over 1340 bytes, or over 500 bytes on ESP8266.
- Every command gets `v:true` and `time:<unix seconds>` added, plus
  `transition` if the user changed the transition field.
- A response with `success` is ignored; otherwise `json.state ?? json` is fed
  to `readState()` and `json.info` to `parseInfo()`.
- Reads `/json/eff`, `/json/fxdata`, `/json/palettes`, `/json/palx?page=`,
  `/json/nodes`, `/json/cfg`, `/presets.json`.

---

## 2. State document — top level

W = accepted on write, R = present on read.

| Key | Type | W | R | Meaning |
|---|---|:-:|:-:|---|
| `on` | bool \| `"t"` | ✓ | ✓ | power; `"t"` toggles |
| `bri` | 0–255 | ✓ | ✓ | master brightness; **R returns `briLast`**, so it survives power-off |
| `transition` | u16 ×100 ms | ✓ | ✓ | default transition duration |
| `tt` | u16 ×100 ms | ✓ | | transition duration for **this request only** |
| `bs` | 0–19 | ✓ | ✓ | transition style (§5); masked to 5 bits |
| `tb` | u32 | ✓ | | effect timebase, for phase-syncing devices |
| `ps` | id \| `"1~5~"` \| `"r"` | ✓ | ✓ | apply preset (R: −1 = none) |
| `pd` | id | ✓ | | "preset direct": body already carries the preset content; only record it as current |
| `psave` | 1–250 | ✓ | | save current state as preset (`n`, `ib`, `sb`, `ql`… in the same body) |
| `pdel` | 1–250 | ✓ | | delete preset |
| `pl` | i8 | | ✓ | active playlist id (−1 none) |
| `playlist` | object | ✓ | | start playlist (`ps[]`, `dur[]`, `transition[]`, `repeat`, `end`, `r`) |
| `np` | bool | ✓ | | advance to next playlist entry |
| `nl` | `{on,dur,mode,tbri,rem}` | ✓ | ✓ | nightlight; `mode` 0 set · 1 fade · 2 colour-fade · 3 sunrise; `rem` read-only seconds |
| `udpn` | `{send,recv,sgrp,rgrp,nn}` | ✓ | ✓ | UDP sync flags; `nn:true` = don't broadcast *this* change |
| `lor` | 0–2 | ✓ | ✓ | live override: 0 none · 1 once · 2 always (ignore realtime input) |
| `live` | bool | ✓ | | enter / exit realtime mode manually |
| `mainseg` | u8 | ✓ | ✓ | main segment (legacy APIs act on it); ignored while realtime |
| `seg` | object \| array | ✓ | ✓ | **segments (§3)** |
| `rSeg` | bool | ✓ | | reset segments to automatic layout (one per bus) |
| `ledmap` | u8 | ✓ | ✓ | select LED map file |
| `rmcpal` | u8 | ✓ | | delete custom palette file |
| `time` | unix s | ✓ | | set clock (fallback when NTP is unavailable) |
| `rb` | bool | ✓ | | reboot (ignored if `psave` present) |
| `win` | string | ✓ | | tunnel a legacy HTTP-API query (`"A=128&FX=5"`) |
| `wifi.ap` | bool | ✓ | | force the soft-AP on/off |
| `v` | bool | ✓ | | request verbose response |
| `error` | u8 | | ✓ | one-shot error code, cleared once serialized |
| *(usermods)* | any | ✓ | ✓ | each usermod reads/writes its own keys |

Read-only-in-presets: when serializing *for a preset*, `error`, `ps`, `pl`,
`ledmap`, `nl`, `udpn`, `lor` and segment `len` are omitted, and slots beyond
the active segments are emitted as `{"stop":0}` so loading the preset deletes
extra segments.

## 3. Segment object

| Key | Type | Meaning | Notes |
|---|---|---|---|
| `id` | u8 | index | `seg` as **object without `id`** → applied to every active **selected** segment |
| `start` / `stop` / `len` | u16 | range `[start, stop)` | `stop:0` **deletes**; `id ≥ count` with `stop>0` **appends**; `len` used only if `stop` absent |
| `startY` / `stopY` | u16 | 2D range | serialized only when the strip is a matrix |
| `grp` / `spc` / `of` | u16 | grouping, spacing, offset | `of` negative wraps from the end; clamped to `len-1` |
| `rpt` | bool | repeat this segment until the strip is filled | alternates `rev` per copy; 1D only |
| `n` | string ≤32 (64 on larger builds) | name | **cleared** if bounds change and `n` is absent |
| `on` | bool \| `"t"` | segment power | |
| `bri` | u8 | segment opacity | `bri:0` turns segment off (keeps last opacity); R reports 255 if 0 |
| `frz` | bool | freeze: stop rendering, keep pixels | |
| `cct` | u8 or Kelvin | white temperature | |
| `col` | `[c0,c1,c2]` | primary / background / custom | see colour grammar below; R always `[[r,g,b(,w)],…]` (w only if any bus has white) |
| `fx` | u8 | effect id | stops a playlist (unless loading a preset); `fxdef:true` loads effect defaults; reserved (`"RSVD"`) ids skip forward; out of range → 0 |
| `sx` / `ix` | u8 | speed / intensity | |
| `c1` / `c2` | u8 | custom sliders | |
| `c3` | 0–31 | custom slider 3 (5-bit) | |
| `o1` / `o2` / `o3` | bool | effect checkboxes | |
| `pal` | u8 | palette id | ignored when the segment has no RGB capability; unknown custom id → 0 |
| `sel` | bool | selected in UI | changing it is **not** a state change (no broadcast, preset kept) |
| `rev` / `mi` | bool | reverse / mirror | changing `mi` resets the effect |
| `rY` / `mY` / `tp` | bool | 2D reverse-Y / mirror-Y / transpose | changing `mY` or `tp` resets the effect |
| `m12` | 0–4 | 1D→2D mapping: Pixels · Bar · Arc · Corner · Pinwheel | |
| `si` | 0–3 | sound simulation: BeatSin · WeWillRockYou · 10/13 · 14/3 | |
| `bm` | 0–16 | blend mode with layer below (§5) | |
| `set` | 0–3 | segment set (UI grouping) | |
| `lc` | R-only bitmask | light capabilities: `1` RGB · `2` W · `4` CCT | UI hides controls from this |
| `i` | array | **individual pixels** | see below |
| `lx` / `ly` | int | Loxone colour formats | build flag only |

### Colour grammar (`col[k]`, `json.cpp:205`)

| Form | Meaning |
|---|---|
| `[r,g,b]` / `[r,g,b,w]` | direct channels; empty array = leave unchanged |
| `"RRGGBB"` / `"WWRRGGBB"` | hex |
| `{"r":…,"g":…,"b":…,"w":…}` | per-channel; missing channels keep current value (`{}` is valid) |
| integer `> 0` | Kelvin colour temperature |
| `0` | black |
| `"r"` | random colour |

On a segment with neither RGB nor W capability (e.g. an on/off relay bus),
any `col` write sets `c0 = full white`, `c1 = black`.

A **new** segment created outside of preset loading gets `c0 = DEFAULT_COLOR`
(warm orange) as a visual hint.

### Individual pixel API (`i`, `json.cpp:314`)

```
"i": [ 0, "FF0000",  2, [0,255,0],  10, 20, "0000FF" ]
       idx  colour    idx  colour    start stop colour
```

Side effects: sets transition to 0 for this request, applies brightness
immediately, **freezes** the segment and clears it to black (only if it wasn't
already frozen). Writes raw segment pixels — no grouping/spacing/2D expansion.

## 4. Value grammar

### `getVal` (`util.cpp:85`) — any u8 field (`bri`, `fx`, `sx`, `ix`, `pal`, `c1`…, `ps`)

| Input | Result |
|---|---|
| int ≥ 0 | set |
| int < 0 | **ignored** (`{"ps":-1}` is a no-op) |
| `"~"` / `"~-"` | +1 / −1, **wrapping** at field limits |
| `"~N"` / `"~-N"` | add / subtract N, **clamped** |
| `"w~N"` / `"w~-N"` | add / subtract N, wrap if already at the limit |
| `"r"` | random in field range |
| `"A~B~"` | increment, cycling within A..B |
| `"A~B~-"` | decrement, cycling within A..B |
| `"A~Br"` | random within A..B |
| `"~0"` | no change |
| string longer than 12 | ignored |

### `getBoolVal` (`util.cpp:105`)

`true`/`false`, or a string starting with `t` → **toggle** relative to the
current value.

This grammar is what makes buttons, IR remotes and presets expressive without
any client-side logic — "next effect" is literally `{"seg":{"fx":"~"}}`.

## 5. Enumerations

### `bs` — transition style (`index.htm`)

| 1D | | 2D only | |
|---|---|---|---|
| 0 | Fade | 6 / 7 | Swipe up / down |
| 1 | Fairy Dust | 8 / 9 | Open H / V |
| 2 / 3 | Swipe right / left | 10–13 | Swipe TL / TR / BR / BL |
| 4 / 5 | Outside-in / Inside-out | 14 / 15 | Circular out / in |
| 16 / 17 | Push right / left | 18 / 19 | Push up / down |

### `bm` — segment blend mode

0 Top/Default · 1 Bottom/None · 2 Add · 3 Subtract · 4 Difference · 5 Average ·
6 Multiply · 7 Divide · 8 Lighten · 9 Darken · 10 Screen · 11 Overlay ·
12 Hard Light · 13 Soft Light · 14 Dodge · 15 Burn · 16 Stencil

### `error` codes (`const.h:480`)

| Code | Meaning |
|---|---|
| 0 | none |
| 1 | permission denied |
| 2 | concurrency (client active) |
| 3 | JSON buffer busy |
| 4 | not implemented |
| 7 / 8 | no RAM for pixels / effect RAM depleted |
| 9 | JSON parse failed |
| 10–19 | filesystem (10 begin · 11 quota · 12 preset missing · 13 ir.json missing · 14 remote.json missing · 19 general) |
| 30–32 | over-temp / over-current / under-voltage |
| 33–37 | low memory (RAM · segment · WS · pixel buffer) |
| 90 / 91 | reboot after error / brownout |
| 100 / 101 | reboot / power-cycle needed after hardware change |

### `callMode` — who caused a change (`const.h:273`)

0 init · 1 direct change · 2 button · 3 notification (UDP/DMX) · 4 nightlight ·
5 no-notify · 7 Hue · 10 Alexa · 11 WS-send-only · 12 button preset
(6, 8, 9 are retired).

Used to break feedback loops: a change caused by a notification is never
re-broadcast over UDP; `NO_NOTIFY` suppresses UDP but still updates WS/MQTT.

## 6. Effect metadata (`/json/fxdata`)

Every effect has a descriptor string; the UI builds its controls from it.

```
"Fire 2012@Cooling,Spark rate,,2D Blur,Boost;;!;1;pal=35,sx=64,ix=160,m12=1,c2=128"
 └ name ─┘ └──────── sliders ─────────────┘ │ │ │ └──────── defaults ─────────────┘
                                        colours │ flags
                                          palette
```

| Field | Content |
|---|---|
| name | before `@`; this alone is what `/json/eff` returns |
| sliders | comma list, positional: `sx, ix, c1, c2, c3, o1, o2, o3`. `!` = show with default label, empty = hide, text = custom label. Positions 5–7 are checkboxes. |
| colours | up to 3: `!` = default label (Fx / Bg / Cs), text = custom label, empty = hide |
| palette | `!` = show palette selector, empty = hide |
| flags | `0` works on a single LED · `1` 1D · `2` 2D · `v` volume-reactive · `f` frequency-reactive |
| defaults | `key=value` list applied by `setMode(fx, loadDefaults)`: `sx ix c1 c2 c3 o1 o2 o3 m12 si rev mi rY mY pal` |

Missing descriptor → UI treats it as `";;!;1"` (defaults, 1D). `/json/fxdata`
returns only the part after `@` for each effect; entry 0 is replaced by the
UI with `";!;"`.

`setMode` details (`FX_fcn.cpp:607`): with defaults, fields not named in the
descriptor reset to `DEFAULT_SPEED`/`DEFAULT_INTENSITY`/`DEFAULT_C1..3`,
checks to false, `m12` to Pixels. `pal` is *always* extracted to set the
segment's "default palette" (palette 0 resolves to it; 6 = Party if unset).
Any mode change starts a transition, marks the segment for reset, and sets
`stateChanged`.

## 7. Info document (read-only, `json.cpp:706`)

| Key | Content |
|---|---|
| `ver`, `vid`, `cn`, `release`, `repo`, `deviceId` | version identity |
| `leds.count` | total LEDs |
| `leds.pwr`, `leds.maxpwr` | estimated current draw / ABL limit (mA) |
| `leds.fps` | measured frame rate |
| `leds.maxseg` | segment capacity |
| `leds.bootps` | boot preset |
| `leds.matrix{w,h}` | only for 2D setups |
| `leds.lc` | OR of all segments' `lc` |
| `leds.seglc[]`, `leds.rgbw`, `leds.wv`, `leds.cct` | deprecated but still emitted |
| `str`, `name`, `udpport`, `simplifiedui` | misc |
| `live`, `liveseg`, `lm`, `lip` | realtime active / segment / source name / source IP |
| `ws` | WS client count (−1 if WS disabled) |
| `fxcount`, `palcount`, `cpalcount`, `umpalcount`, `cpalmax`, `umpalnames[]` | catalogue sizes |
| `maps[]` | available LED maps `{id, n}` |
| `wifi{bssid,rssi,signal,channel,band,ap}` | radio |
| `fs{u,t,pmt}` | filesystem used/total KB, presets modified time |
| `ndc` | discovered node count (−1 if disabled) |
| `arch`, `core`, `clock`, `flash`, `freeheap`, `psram`, `psrSz`, `uptime`, `time` | platform |
| `opt` | build feature bitmask: `0x01` OTA · `0x02` Adalight · `0x04` Hue · `0x08` FS · `0x10` Cronixie · `0x40` Alexa · `0x80` debug · `0x100` net debug |
| `brand`, `product`, `mac`, `ip` | identity |

## 8. Semantics only visible in the code

1. **Application order is fixed** (`deserializeState`):
   `bri` → `on` (+toggle) → unfreeze-on-power-on → `transition`, `bs`, `tt`, `tb`
   → `nl` → `udpn` → `time` → `rb` → `mainseg` → `lor` → `live`
   → `seg` (strip suspended while applying) → `rSeg` → usermods → `ledmap`
   → `psave` → `pdel` → `win` → `pd` / `ps` → `playlist` → `rmcpal` → `np`
   → `wifi` → `stateUpdated(callMode)`.
   A `ps` with no other state change **returns early** and loads the preset
   asynchronously.
2. **Power and brightness are three values.** `bri` is the target, `briT` the
   in-transition value, `briLast` the level remembered for power-on.
   `{"on":false}` stores `briLast` and sets `bri=0`; `{"on":true}` restores it
   and restarts effect runtime. `{"on":"t","bri":32}` turns on at 32 (the
   toggle is skipped if `bri` already turned it on).
3. **Change detection is per segment.** Before applying, a small copy of the
   segment's fields is taken; `differs()` produces a bitmask
   (`BRI 0x01 · OPT 0x02 · COL 0x04 · FX 0x08 · BOUNDS 0x10 · GSO 0x20 · SEL 0x80`).
   Any bit except `SEL` sets `stateChanged`.
4. **`stateChanged` has consequences** (`stateUpdated`, `led.cpp:87`):
   clears `currentPreset`; sends a UDP sync notification unless
   `callMode ∈ {NOTIFICATION, NO_NOTIFY}`; broadcasts node info if brightness
   changed; schedules WS + MQTT; notifies usermods; starts or finalizes the
   brightness transition.
5. **Transitions are implicit.** `setColor`, `setOpacity`, `setOption(ON)`,
   `setMode` and `setPalette` each start a transition of the current duration
   in style `bs`. Exceptions: `tt` (one-shot duration, restored afterwards) and
   `i` / `live` (force 0).
6. **Segment batch deletion**: if an array `seg` deletes at least half of more
   than three segments, the segment list is purged/compacted afterwards.
7. **Deleting the main segment** resets `mainseg` to 0.
8. **Writers are serialized by a pause lock** (`strip.suspend()` +
   `waitForIt()`), because JSON can arrive while effects are rendering.
9. **Legacy globals** (`colPri`, `effectCurrent`, `effectSpeed`, …) are
   refreshed from the *first selected* segment after every change; the legacy
   HTTP API and nightlight act on those and push them back to all selected
   segments.

## 9. Compatibility tiers

| Tier | Scope | Unlocks |
|---|---|---|
| **A** | `on`, `bri`, `transition`/`tt`; `seg[]` with `id`, `start`, `stop`, `len`, `on`, `bri`, `col`, `fx`, `sx`, `ix`, `pal`, `rev`, `mi`, `sel`, `n`, `lc`; `v`; `/json`, `/json/state`, `/json/si`, `/json/info`, `/json/eff`, `/json/pal`; WS push + text commands | official WLED apps and Home Assistant can read and control the device |
| **B** | `grp`/`spc`/`of`, `c1`–`c3`, `o1`–`o3`, `frz`, `cct`, `i`, `rpt`, `fxdef`; full `getVal` grammar and toggles; `/json/fxdata`, `/json/palx`; `udpn.nn`; error codes; WS `lv` live stream | the full 1D state feature set |
| **C** | presets (`ps`/`pd`/`psave`/`pdel`, `/presets.json`), playlists, `nl`, `bs`/`bm`, `tb`, 2D keys (`startY`/`stopY`/`rY`/`mY`/`tp`/`m12`), `mainseg`, `lor`/`live`, `ledmap`, `/json/nodes`, `/json/net`, `/json/cfg` | feature parity for the state surface |
