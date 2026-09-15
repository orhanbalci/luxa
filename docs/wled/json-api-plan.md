# WLED JSON API — implementation plan

How Luxa gets from today's `POST /power` + `POST /brightness` to speaking
WLED's state API. The surface being implemented is specified in
[`json-api.md`](json-api.md); section references (§) point there.

Steps name *responsibilities*; where each one lives is set out in
[Crate decisions](#crate-decisions) at the end (some entries are still
proposals).

## Principles

1. **Wire-compatible first, refactor after.** Key names, endpoints, value
   ranges and observable semantics match WLED exactly, so existing WLED
   clients and captured fixtures prove the implementation works. Only once
   that is proven is the API shape open to refactoring into a Luxa-native
   form — with the fixtures as the regression net. Internal types are
   idiomatic Rust from the start and do not mirror WLED's globals.
2. **JSON never reaches the engine.** `bytes → codec → patch → engine →
   snapshot → codec → bytes`. The engine stays transport- and format-agnostic,
   as it is today.
3. **Fixed capacity, no allocator in the portable tier.** Segment count, name
   length, request and response sizes are fixed at compile time — as generic
   parameters the application chooses, not constants inside the crates.
4. **Determinism is injected.** Randomness (`"r"` values, random colours)
   arrives as a closure passed to the engine, the same way time arrives
   through `Ctx`, so every grammar rule is testable.
5. **Every step ships with host tests.** Behaviour is proven with
   `cargo test --workspace` before it touches Wokwi.
6. **WLED's application order is preserved** (§8.1) even where Luxa does not
   yet implement a key, so adding keys never reorders existing behaviour.

---

## Progress

| Step | Status |
|---|---|
| 1 Golden fixtures | ✅ derived — 49 fixtures in `tests/fixtures/api/` traced from WLED source (every Tier A key covered); replace with captures once a device is available |
| 2 Capacities and limits | ◐ capacities are const generics (firmware: 32 segments, 64-byte names); request/response size limits pending with the codec |
| 3 Value types | ✅ `Rgbw` in `luxa-color`; `EffectId`, `PaletteId`, `LightCaps`, `TransitionTime`, `ErrorCode`, `Seq` in `luxa-msg` |
| 4 Segment and state | ✅ `Segment`, `State`, `Layout`, `Name` in `luxa-msg`; defaults match a freshly booted WLED |
| 5 Published state and ack | ✅ `Envelope`, `applied_seq`, publish-when-awaited in `luxa-core`; firmware `submit` numbers and enqueues in one critical section |
| 6 Patch types | ✅ granular `Command::Global` / `Command::Segment` with `U8Op`, `BoolOp`, `ColorSpec`, `SegmentTarget`; every Tier A key representable; envelope ≤ 192 B at 64-byte names. Engine applies brightness, power and transition levels so far |
| 7 Origin | ✅ `Origin` on every `Envelope`; `apply_batch` returns `Outcome { state, origins, changes }` with the origins of commands that changed state |
| 8 Ordered application | ✅ brightness → power (incl. toggle guard) → transition; segment fields in reference order (bounds, name, opacity, on, colours, selection/reverse/mirror, effect, speed, intensity, palette) |
| 9 Power and brightness | ✅ `last_brightness`, toggle, `on:"t"` with brightness, zero is off, relative brightness. Restarting effect time on power-on is left to the renderer |
| 10 Segment lifecycle | ✅ create (appended, orange, selected), delete keeps the slot, reuse keeps colours, fan-out to selected, `len`, bounds clamping, name clearing, main-segment reset, `CompactSegments` threshold |
| 11 Colour resolution | ✅ exact, partial, Kelvin (`luxa_color::kelvin_to_rgb`, matches reference values), random, relay segments (full white) |
| 12 Value grammar | ✅ `resolve_u8` / `resolve_bool` for set, keep, cycle, add with wrap, random with exclusive top, bounds; effect gaps skip forward, overflow falls back to 0; invalid palettes fall back to 0 |
| 13 Change sets | ✅ `Changes` bits; selection and naming publish but are not state changes (no origins) |
| 14 Light capabilities | ✅ `Layout::caps` copied onto segments; gates palette and colour handling |
| 15 Catalogues | ✅ `EffectKind` ids follow the reference numbering (Solid 0, Rainbow 9) with descriptors; `Descriptor` parser (sliders, colours, palette, flags, defaults after the last `;`); palette list (Default only); `luxa_segment::CATALOGUE` feeds the engine |
| 16 Render the new state | ✅ `Compositor` renders each active segment's own effect into its range with `Params` (speed, intensity, colours), mirror, reverse and opacity; per-segment effect instances; later segments on top |

## Phase 0 — Foundations

### 1. Golden fixtures
- Capture request → response pairs from a real WLED build (Wokwi or a
  device): `GET /json/si`, and `POST` bodies covering every Tier A key.
- Store under `tests/fixtures/api/` as `{ before, request, after, response }`.
- Data only — no crate is needed yet. The harness that runs these arrives with
  `luxa-api` in step 19; until then the fixtures are the reference the
  engine tests in Phase 3 are written against.
- **Done when:** every Tier A key in §2/§3 appears in at least one fixture.

### 2. Capacities and limits
- Segment capacity and name length are **const generic parameters** of the
  state types, chosen by the application — not constants inside the crates
  (crate rule 2). The firmware picks the ESP32-C3 values.
- Fix the max request body (WLED's WS limit is 1428 bytes) and the response
  buffer or streaming strategy for `/json/si` on the ESP32-C3.
- Record the numbers in `json-api.md`.
- **Done when:** the constants exist and there is a test asserting that a
  full-capacity `/json/si` serializes within budget.

## Phase 1 — State model (Tier A)

### 3. Value types
- `Rgbw`, `SegmentColors` (3 slots), `EffectId`, `PaletteId`,
  `LightCaps` (RGB / W / CCT bits), transition duration in 100 ms units,
  `ErrorCode`.
- **Done when:** the types exist with unit tests for conversions and ranges.

### 4. Segment and application state
- `Segment`: `start`, `stop`, `on`, `opacity`, `colors`, `fx`, `speed`,
  `intensity`, `palette`, `reverse`, `mirror`, `selected`, `name`.
- `State`: `bri`, `bri_last`, `transition`, a fixed-capacity segment list,
  and `main_seg` (published as-is — see step 5).
- Tier B/C fields are added in their own steps, not stubbed now.
- **Done when:** the default state matches a fresh WLED with one LED bus (one
  segment covering the strip).

### 5. Published state and request acknowledgement
- One `State` type, owned by the engine and published whole
  (replacing today's `AppState`/`Snapshot` pair). At 16 named segments it is
  about 2 KB, and the render task copies it only on change, not per frame.
  Split only if measurement says so.
- Commands travel as `Envelope { seq, origin, patch }`; `State` carries
  `applied_seq`. `seq` is assigned immediately before `try_send` with no
  `await` in between, so on the single-core cooperative executor sequence
  order equals queue order.
- The engine publishes when state changed **or** when the batch contained a
  command whose sender awaits a reply — otherwise a no-op request (e.g.
  `{"on":true}` while on) would wait forever.
- **Done when:** a test proves a no-op command still advances `applied_seq`,
  and a waiter for `seq = n` is never woken by a batch that ended before `n`.

## Phase 2 — Command vocabulary

### 6. Patch types
- **Granular commands.** A request becomes an ordered run of small commands,
  not one big patch: `Command::Global(GlobalPatch)` for fixture-wide keys,
  then one `Command::Segment(SegmentPatch)` per segment. A whole-request patch
  is ~5 KB at 32 segments (a 16-deep queue would pin ~80 KB); an envelope is
  under 192 bytes. The run is queued in one critical section and applied in
  one batch, so key order and "one publish per request" are preserved.
- `GlobalPatch`: `brightness`, `on`, `transition`, `transition_once` (Tier A).
  `v` is not a patch field — it is the envelope's `awaits_reply`.
- `SegmentPatch<NAME>`: a `SegmentTarget` (`Id` or `Selected`; array entries
  without an id get their index) and every Tier A segment key optional.
- `U8Op` modelling the full value grammar from the start (§4): `Set`, `Keep`,
  `Cycle { direction, bounds }`, `Add { delta, wrap, bounds }`,
  `Random { bounds }`.
- `BoolOp`: `Set` or `Toggle`. `ColorSpec`: `Rgbw` (black included),
  `Partial { r, g, b, w }`, `Kelvin`, `Random`.
- `Command::power(on)` and `Command::brightness(level)` replace the old
  variants as constructors. The engine API stays `apply_batch`.
- **Done when:** every row of §2 and §3 marked Tier A is representable, and the
  existing `/power` and `/brightness` handlers still compile against the new
  vocabulary.

### 7. Origin (WLED's `callMode`)
- Every command carries who caused it: `Direct`, `Button`, `Notification`,
  `NoNotify`, … (§5).
- The engine does not act on it; it reports the origins of the commands that
  changed something in `Outcome::origins`, so broadcasting (step 22) and UDP
  sync (later) can suppress feedback loops.
- **Done when:** origin travels from ingress to the published change set.

## Phase 3 — Engine semantics (Tier A)

### 8. Ordered application
- Apply patch keys in §8.1 order, skipping keys not yet implemented.
- **Done when:** a test proves `bri` applies before `seg` and `seg` before
  `ps` placeholders.

### 9. Power and brightness
- `bri_last`, toggle, `bri:0` turns off, power-on restores `bri_last` and
  restarts effect time, `{"on":"t","bri":32}` turns on at 32, reads report
  `bri_last` (§8.2).
- **Done when:** each rule has its own test, and the existing core tests still
  pass.

### 10. Segment lifecycle
- Append when `id ≥ count` and `stop > 0`; delete on `stop:0`; `len` when
  `stop` is absent.
- An object without `id` fans out to selected segments.
- Deleting the main segment resets `main_seg`.
- Compaction rule (§8.6), sent as `Command::CompactSegments { deleted }` after
  a run of segment commands; new-segment default colour; name cleared when
  bounds change without `n`; bounds clamped to LED count. Deleted segments keep
  their slot (`stop == 0`) until compacted.
- **Done when:** fixtures for create, resize, delete, and fan-out-to-selected
  pass.

### 11. Colour resolution
- `ColorSpec → Rgbw`, honouring segment capabilities (§3 colour grammar),
  including Kelvin → RGB (port `colorKtoRGB`), random colours via the injected
  RNG, and the rule for non-RGB segments.
- **Done when:** every row of the colour-grammar table has a test.

### 12. Value grammar evaluation
- `U8Op::apply(current, min, max, rng)`, with per-key ranges: `fx` limited to
  effect count, `pal` to palette count, `c3` to 0..31.
- Negative ints and strings over 12 characters are ignored.
- **Done when:** every row of the §4 table has a test, including wrap-vs-clamp
  edge cases at 0 and max.

### 13. Change sets
- `apply` returns a change set instead of `bool`: per-segment bits
  `BRI / OPT / COL / FX / BOUNDS / GSO / SEL` plus global bits.
- Selection and naming are not state changes: a batch that only selects or
  names segments still publishes (so reads stay current) but reports no
  origins, so nothing is announced to peers or broadcast.
- `apply_batch` exposes the change set together with the origins.
- **Done when:** selection-only patches publish without origins, while every
  other field change is reported as a state change.

### 14. Light capabilities
- Derive `lc` for each segment from the output configuration. With today's
  single WS2812 bus that is RGB; the bus model makes it real later.
- **Done when:** `lc` is serialized and gates `pal` and `col` handling.

## Phase 4 — Catalogues and minimum rendering

### 15. Effect and palette catalogues
- Each effect declares a name and a WLED-format descriptor string (§6).
- The registry exposes the count, names and descriptor for each id,
  parses `fxdef` defaults, and skips reserved ids.
- The palette catalogue exposes names and count, even if it holds only a
  handful of palettes.
- **Done when:** the descriptor parser handles every descriptor in WLED's
  `FX.cpp` consistently. The committed tests cover one real descriptor per
  format variant; the full corpus is checked by an ignored test
  (`LUXA_DESCRIPTOR_CORPUS=… cargo test -p luxa-effect --test
  descriptor_corpus -- --ignored`) so WLED's text is not copied into the repo.

### 16. Render the new state
- The compositor renders each active segment into `[start, stop)` with its own
  effect, on/opacity, reverse, mirror, colours, speed and intensity.
- Effects receive segment parameters alongside `Ctx`.
- The full segment feature set comes from the `FX.h` / `FX_fcn.cpp` analysis;
  this step is only what Tier A needs to be visible.
- **Done when:** a pipeline test shows two segments rendering different
  effects into their own ranges.

## Phase 5 — JSON codec

### 17. Parser
- `&mut [u8] → StatePatch`, `no_std`, no allocator: serde +
  `ser-write-json`, parsing in place (crate decision 3).
- Hand-written `Visitor`s for the polymorphic fields — `seg` as object or
  array; `on` as bool or `"t"`; u8 fields as int or string; all `col` forms —
  and `#[derive]` for everything else. Start from the probe crate.
- Invalid *values* are lenient, as in WLED: out-of-range colour channels
  clamp, extra `col` entries are dropped, uninterpretable values become no-ops
  rather than failing the request.
- Unknown keys are ignored, as WLED does.
- Malformed input maps to error 9.
- **Done when:** fixture requests parse, and a host fuzz target runs clean.

### 18. Serializers
- **State:** §2/§3 read columns, applying the read-side quirks: `bri` reports
  `bri_last`; segment `bri` reports 255 when 0; `col` is always arrays; `w`
  only when a white channel exists.
- **Info:** Tier A subset.
- **Other bodies:** `/json/eff`, `/json/pal`, `{state, info}`, full `/json`,
  `{"success":true}`, `{"error":n}`.
- Output goes to a writer (chunked or streaming), so `/json` does not need a
  single large buffer.
- **Done when:** serialized fixture states match WLED output.

### 19. End-to-end conformance
- A harness in `luxa-api`'s tests runs the step 1 fixtures:
  fixture → parser → engine → serializer, compared semantically (key order
  ignored, volatile `info` fields masked).
- **Done when:** every Tier A fixture passes.

## Phase 6 — Transports

### 20. Portable request router
- `handle(method, path, body) → Response`, implementing WLED's routing order
  including the 501 fallback (§1).
- It lives in `luxa-api::protocol`; the picoserve code in `firmware/` becomes
  a thin adapter.
- **Done when:** routing is tested on the host for every Tier A path.

### 21. HTTP endpoints
- `GET /json`, `/json/state`, `/json/si`, `/json/info`, `/json/eff`,
  `/json/pal`.
- `POST /json`, `/json/state`, `/json/si`, honouring `v`.
- **Request/ack:** a verbose POST must return the state *after* its patch is
  applied. The handler submits the request's run of commands in one critical
  section (`submit_all`, the last marked `awaiting_reply`), then awaits a
  published `State` with `applied_seq ≥` that last number (step 5). The queue
  must be deep enough for a full request (fixture keys plus one command per
  segment).
- Retire `/power`, `/brightness` and `/state`; point the built-in page at
  `/json/si`.
- **Done when:** `curl` against Wokwi reproduces the Tier A fixtures.

### 22. WebSocket `/ws`
- Push `{state, info}` on connect.
- Text frames: `p` ping, `{"v":true}`, and state patches, answered with
  `{"success":true}` or the full state.
- Fragmented messages get error 9.
- Broadcast to all clients on each published change, with a cooldown; honour
  the origin (no echo for `NoNotify`-style changes where WLED suppresses it).
- A client cap sized to RAM.
- **Done when:** two browser tabs stay in sync on Wokwi.

### 23. Discovery
- mDNS advertisement as WLED does (`wled.cpp:939`): `_http._tcp` and
  `_wled._tcp` on port 80, with a `mac` TXT record on `_wled`. Apps then find
  the device without typing an IP.
- **Done when:** the device shows up in an mDNS browser on the LAN.

## Phase 7 — Tier A validation

### 24. Interoperability
- Drive the device with:
  - `python-wled`, the library behind Home Assistant's WLED integration
  - the WLED mobile app
- Record every key those clients require that the fixtures missed, and add
  them as fixtures.
- **Done when:** Home Assistant can discover the device, toggle it, set
  brightness, colour and effect, and reflect changes made elsewhere.

**Milestone: Tier A complete.**

## Phase 8 — Tier B

25. **Geometry:** `grp`, `spc`, `of` (with negative wrap), `rpt`, `frz`, `cct`.
26. **Effect parameters:** `c1`–`c3`, `o1`–`o3`, `fxdef`, and streaming
    `/json/fxdata`.
27. **Individual pixels:** `i`, with the freeze, clear and zero-transition
    semantics.
28. **Sync flags without sync:** `udpn` is stored and serialized, `nn` is
    carried as origin, and `time` is accepted.
29. **Transitions:** `transition` / `tt` fades for brightness and colour. The
    state holds durations; the render/output tier does the crossfade.
30. **Live view:** WS `lv` binary frames (1D format) and `/json/live`.
31. **Palette data:** `/json/palx?page=N`.
32. **Error reporting:** a one-shot `error` in state, reusing the codes from
    step 3.

**Milestone: the 1D state surface is feature-complete.**

## Phase 9 — Tier C

Each item gets its own module analysis doc before implementation.

33. **Presets** (`presets.cpp`): persistence, `ps`/`pd`/`psave`/`pdel`,
    `/presets.json`, and cycling syntax.
34. **Playlists** (`playlist.cpp`).
35. **Nightlight `nl`** (`led.cpp`).
36. **Transition styles `bs` and blend modes `bm`** (`FX_fcn.cpp`).
37. **2D keys:** `startY`, `stopY`, `rY`, `mY`, `tp`, `m12`
    (`FX_2Dfcn.cpp`).
38. **Realtime:** `mainseg`, `lor`, `live` (`udp.cpp`, `e131.cpp`).
39. **Remaining endpoints:** `ledmap`, `/json/nodes`, `/json/net`,
    `/json/cfg` (their respective modules).

---

## Crate decisions

### Rules

1. **No new or split crates unless unavoidable** until the product works.
   Extension points (e.g. separating the `Effect` trait from effect
   implementations for third-party effect packs) are decided afterwards.
   Prefer a module in an existing crate.
2. **Every crate is reusable outside Luxa.** No application constants, no
   runtime types (embassy, picoserve) in portable crates, capacities as
   generic parameters, and no fast-moving third-party traits in public
   signatures.
3. **Nothing is named after WLED.** Crates, modules, types and paths use Luxa
   names. WLED compatibility is the first *shape* of Luxa's API, refactored in
   place once proven — not a separate layer to delete later. WLED strings
   survive only where the wire demands them (JSON keys, the `_wled._tcp` mDNS
   service) until that refactor.

### Crate map

One new portable crate (`luxa-api`); everything else extends what exists.

| Crate | Change | Owns |
|---|---|---|
| `luxa-msg` | grows | `State`, `Segment`, value types; granular commands (`Command::Global(GlobalPatch)`, `Command::Segment(SegmentPatch)`) with `U8Op`, `BoolOp`, `ColorSpec`; `Envelope { seq, awaits_reply, origin, command }` and `Origins`; `Layout` (LED count + light capabilities); `Catalogue` trait (effect/palette counts, names, descriptors). Pure data plus traits, no behaviour. |
| `luxa-core` | grows | `Engine`: owns `State`, resolves `U8Op`/`BoolOp`/`ColorSpec` against catalogue ranges, applies patches in WLED order, tracks `applied_seq`, returns change sets. Generic over `Catalogue`. |
| `luxa-effect` | grows, **not split** | Effect trait, `Ctx`, effect implementations and registry — plus segment parameters (speed, intensity, colours, …) and effect metadata (WLED descriptor strings and their parser). |
| `luxa-segment` | grows | Multi-segment compositor rendering `State` segments with their effects; `CATALOGUE` of the ids it can draw. |
| **`luxa-api`** | **new** | Luxa's control API — WLED-compatible in its first shape (crate rule 3). Two modules: `json` (serde + `ser-write-json` codec for patches, state, info, catalogues) and `protocol` (endpoint routing, `v`/success/error replies, WS frame protocol, broadcast cooldown — pure logic, no IO). Depends only on `luxa-msg`. |
| `luxa-color` | grows | `Rgbw` (configured colours with a white channel), next to `Crgb`. |
| `luxa-output` | narrows | Takes a plain brightness instead of the engine's state, and no longer depends on `luxa-msg` (crate rule 2). |
| `luxa-canvas`, `luxa-wire` | unchanged | — |
| `firmware/luxa-runtime` | grows | picoserve HTTP/WS adapters over `luxa-api::protocol`; `Catalogue` adapter over `luxa-effect`'s registry; mDNS. |

### Decisions

| # | Topic | Status | Decision |
|---|---|---|---|
| 1 | Patch types | **decided** | In `luxa-msg`, WLED semantics with Rust naming. Commands are granular — one request is an ordered run of `Global` and per-`Segment` commands queued together — so an envelope stays under 192 bytes instead of ~5 KB per request. WLED key names exist only in `luxa-api::json`, so the later refactor to a Luxa-native API rewrites that module while the patches, engine and fixtures-as-regression-tests stay. |
| 2 | Value grammar | **decided** | Representation (`U8Op`, `BoolOp`) in `luxa-msg`; resolution in `luxa-core`, which knows the ranges. |
| 3 | JSON parsing | **decided** | serde + `ser-write-json` (`default-features = false`). `serde-json-core` 0.6 cannot be used — its `deserialize_any` returns `AnyIsUnsupported` — and `#[serde(untagged)]` needs `alloc` with any format. `ser-write-json` implements a real, non-buffering `deserialize_any`, so hand-written `Visitor`s for the four polymorphic shapes work with no allocator; everything else is `#[derive]`. Proven by a probe crate: 8 host tests on WLED-shaped bodies, `riscv32imc-unknown-none-elf` release build, no `alloc`/`std` feature in the tree. Visitors are plain serde, so the format crate can be swapped without touching them. Still open: serializer and streaming output. |
| 4 | Codec vs router | **decided** | One crate, `luxa-api`, two modules (rules 1 and 3). Created in Phase 5 (step 17, `json`); `protocol` added in Phase 6 (step 20). Nothing earlier depends on it. |
| 5 | Effect metadata | **decided** | Stays in `luxa-effect`; no split (rule 1). The engine and codec see it only through `luxa-msg::Catalogue`, built as `luxa_segment::CATALOGUE` (the compositor crate already depends on both effects and state), so neither depends on effect implementations and host tests use their own catalogue. |
| 6 | Snapshot | **decided** | One `State` type, published whole; split only if measured (see step 5). |
| 7 | Randomness | **decided** | No RNG crate and no `rand_core` in public APIs — the firmware lock already carries `rand_core` 0.6.4, 0.9.5 and 0.10.1, and 0.10 renamed the core traits, so exposing it would pin users to one major. The engine owns a small seedable generator (xorshift) and the runtime seeds it once from the hardware RNG — simpler than passing a closure on every call, reproducible in tests, and still nothing third-party in the API. |
| 8 | Light capabilities | **decided** | `luxa-msg::Layout` passed to the engine at construction; revisited in the bus-manager analysis. |
