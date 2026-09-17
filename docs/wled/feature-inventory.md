# WLED feature inventory

What WLED does beyond its JSON API, module by module, ranked by what makes
Luxa's device *do* what its API already claims. The API surface itself is in
[`json-api.md`](json-api.md); its implementation plan is
[`json-api-plan.md`](json-api-plan.md).

**Source:** [`wled/WLED`](https://github.com/wled/WLED) `main` @ `06ae26d`.
Files read: `FX_fcn.cpp` (segments, transitions, blending, palettes, frame
loop), `FX.h`, `FX.cpp` (effects), `FX_2Dfcn.cpp`, `led.cpp` (brightness
transition, nightlight), `colors.cpp`, `palettes.cpp`, `const.h`, `wled.h`,
`wled.cpp` (main loop), `bus_manager.{h,cpp}` (output, current limiter),
`presets.cpp`, `playlist.cpp`, `ntp.cpp` (timers), `udp.cpp` (sync, realtime),
`e131.cpp`, `button.cpp`, `ir.cpp`, `remote.cpp`, `mqtt.cpp`, `alexa.cpp`,
`hue.cpp`, `overlay.cpp`, `dmx_*.cpp`, `improv.cpp`, `file.cpp`,
`ota_update.cpp`, `image_loader.cpp`, and the `usermods/` directory listing.
Effect counts and classifications were computed from `FX.cpp` registrations
and descriptor strings.

---

## 0. How this is ranked

Luxa's API already accepts more than its device renders, and clients built
for the reference expect more than Luxa lists. A client that sets
`transition` gets a success reply and a state document echoing it, and the
strip changes instantly. A client that offers the reference's 216 effects or
72 palettes finds two effects and one palette, and other ids fall back
silently. Closing that gap comes first; new surface comes after.

| Rank | Kind | Why |
|---|---|---|
| **1** | Transitions | `transition`/`tt` are accepted and stored; nothing fades |
| **2** | Effects | clients expect the reference's 216 effects; Luxa renders 2 |
| **3** | Palettes | clients expect 72 palettes; Luxa renders 1 |
| **4** | Output fidelity | colours and brightness don't look like the reference: no gamma, no current limit |
| **5** | Segment geometry and effect parameters | Tier B keys the renderer must honour (plan steps 25–27) |
| **6** | Presets, playlists, nightlight, timers | automation; needs storage and a clock |
| **7** | Sync and realtime | device-to-device and pixel streaming |
| **8** | Inputs and integrations | buttons, IR, MQTT, voice assistants |
| **9** | Hardware breadth | more LED types, buses, 2D panels |

Each section says what WLED does, where Luxa stands, and what it implies for
Luxa's design.

---

## 1. Where Luxa stands

| API | Device |
|---|---|
| `transition` 750 ms default, `tt` one-shot, both stored | changes land on the next frame |
| `fx`: `/json/eff` lists 68 effects under reference ids and names, `RSVD` gaps between | Solid and Rainbow natively, 66 from `smart-leds-fx` (41 of WS2812FX lineage and 25 written from the reference's descriptors; none has the reference's exact look); unknown ids fall back to Solid |
| `pal`: `/json/pal` lists palettes 0–12 | in-order stepped effects and Rainbow draw from them; palette changes fade; 13–71 not yet |
| `col[1]`, `col[2]` | only effects that read them; Rainbow reads none |
| `bri` | linear `nscale8` over the frame, no gamma |
| `info.leds.maxpwr` | reported 0: no current limiter |
| `info.leds.fps` | 62 fps render loop (reference default 42) |

The compositor (`luxa-segment`) keeps each segment's frame in a pixel pool
between renders, so effects draw over their previous frame. There is still no
second render of a segment, which effect crossfades need (§2.5).

---

## 2. Rank 1 — Transitions

Two independent mechanisms, both driven by one duration
(`strip.getTransition()`, set from `transition` in deciseconds × 100).

### 2.1 Global brightness fade (`led.cpp`)

- `stateUpdated()` runs after every change. With a non-zero duration and a
  brightness or state change it sets `transitionActive`, stamps
  `transitionStartTime`, and puts every segment into transition mode
  (`strip.setTransitionMode(true)`).
- `handleTransitions()` runs every loop iteration:
  `briT = briOld + (bri − briOld) × elapsed / duration`, linear in time,
  applied to the output with `strip.setBrightness(briT)`. When time is up it
  applies the final brightness, ends segment transitions, and restores the
  global duration if the change used `tt` (`jsonTransitionOnce`).
- A change arriving mid-fade restarts from the brightness currently shown
  (`briOld = briT`), so there is no jump.
- Duration 0 applies brightness immediately (`applyFinalBri`).
- Power on/off is a brightness fade between 0 and `briLast`. Turning on from 0
  resets the effect timebase, so effects start from the beginning.

### 2.2 Per-segment transitions (`FX_fcn.cpp`)

Every segment setter calls `startTransition()` **before** changing the value:
colour, CCT, opacity, `on`, effect, palette (and name, for the scrolling-text
effect).

- A `Transition` records the *from* values: the three colours, opacity (0 if
  off), CCT, palette id and the loaded palette.
- An **effect change** (or any change under a non-fade style) also copies the
  whole segment, so the old effect keeps running alongside the new one.
- Progress is `elapsed × 65535 / duration`, linear.
- **Fade style**, per frame:
  - colours: `color_blend16(from, to, progress)`;
  - opacity and CCT: linear blend;
  - palette: stepped toward the new one with `nblendPaletteTowardPalette(…, 48)`,
    about 255 steps over the duration;
  - effect change: both effects render and their pixels are crossfaded by
    progress.
- **Interruptions** capture the current visual blend as the new *from* and,
  for pure fades, restart the timer. An effect change in flight is left to
  finish.

### 2.3 Transition styles `bs` (`FX_fcn.cpp: blendSegment`)

18 styles (`TRANSITION_COUNT`): fade, fairy dust, swipe right/left,
outside-in, inside-out, swipe up/down, open horizontal/vertical, four
diagonal swipes, circular out/in, push right/left/up/down. (Four diagonal
pushes are defined but unused.)

Non-fade styles set a clipping rectangle that grows with progress: pixels
inside show the new segment, pixels outside the old copy. Push styles also
shift the canvas. On a 1D strip the meaningful ones are fade, fairy dust,
swipe left/right, outside-in, inside-out and push left/right. Single-pixel
segments always fade.

### 2.4 Random palette cycling

Palette 1 ("Random Cycle") generates a new palette every `rpc` seconds
(default 5; harmonic by default, `hrp`) and blends toward it over the
transition duration (`Segment::handleRandomPalette`, once per frame).

### 2.5 Implications for Luxa

- **The renderer interpolates, not the engine.** The engine publishes
  discrete states; the render task holds the previous published state, diffs
  each new one, and starts transitions from what is *currently shown*. This
  keeps `luxa-core` free of time.
- **The global fade belongs to the output stage** (`luxa-output`): a
  brightness that moves from shown to target over the duration.
- **Segment fades need per-segment state in the compositor:** from-colours,
  from-opacity, and the palette being blended.
- **Effect crossfades need two renders per transitioning segment**, which
  means scratch pixel buffers. A fixed budget of transitioning segments,
  each needing up to `LEDS` pixels, is the no-allocator way.
- `transition_once` is already carried by the engine; the renderer needs the
  duration that applied to each change, not only the current setting.
- Styles (`bs`) are Tier C (plan step 36). Fade first; the clip-rectangle
  model extends to 1D swipe/inside-out/outside-in cheaply once two renders
  exist.

---

## 3. Rank 2 — Effects

### 3.1 The catalogue

- **220 ids** (0–219). **216 effects** are registered; ids 142 and 169–171
  are reserved (`RSVD`) gaps.
- Classified by descriptor flags (`2` = 2D, `v`/`f` = audio) and particle-system
  use:

| | plain | audio | particle system | audio + particle |
|---|---|---|---|---|
| **1D** | 115 | 24 | 11 | 4 |
| **2D** | 40 | 5 | 13 | 4 |

- Audio effects run without a microphone: `getAudioData()` falls back to
  `simulateSound(soundSim)` when the audio-reactive usermod is absent.
- 2D effects need a matrix layout (`startY`/`stopY`, panels) — Tier C.
- Particle-system effects share `FXparticleSystem.{h,cpp}` (~2400 lines) and
  allocate per-segment particle arrays.

### 3.2 What effects call

Counts of use across `FX.cpp` define the effect runtime a port must provide:

| Group | Calls (use count) |
|---|---|
| Parameters | `SEGMENT.speed` (296), `intensity` (231), `custom1–3` (278), `check1–3` (223), `palette`, `cct`, `name` |
| Per-segment memory | `SEGENV.aux0` (272), `step` (183), `aux1` (146), `call` (144), `data` + `allocateData` (136) |
| Time | `strip.now` (198) — one shared timebase, synced between devices |
| Drawing | `setPixelColor` (213), `color_from_palette` (131), `fill` (45), `color_wheel` (31), `fade_out` (27), `blur` (27), `fadeToBlackBy` (24), `getPixelColor` (20), `blendPixelColor` (11) |
| Randomness | `hw_random16` (141), `hw_random8` (135), `random8` (17) |
| Waves | `beatsin8_t` (57), `sin8_t` (32), `cos8_t` (24), `sin16_t` (17), `beatsin16_t` (15), `beatsin88_t` (12), `cubicwave8` (11), `beat8/16`, `triwave8` |
| Noise | `perlin8` (30), `perlin16` (10), `inoise8` |
| Colour maths | `color_blend` (59), `ColorFromPalette` (45), `CHSV` (25), `scale8`, `qadd8`, `qsub8`, `color_fade`, `color_add`, `gamma8` |

Two properties matter most:

- **Many effects read back and fade the previous frame.** `fade_out`,
  `fadeToBlackBy`, `blur` and `getPixelColor` operate on the segment's own
  buffer (`Segment::pixels`), which persists between frames. Trails, twinkles,
  fire and meteors depend on it.
- **Default palette means two things.** `color_from_palette(i, …, mcol)`
  returns the segment's colour `mcol` when the palette is 0 and `mcol < 3`.
  Only an index of 255 uses a palette, and then palette 0 resolves to the
  effect's own default (`pal=` in its descriptor, else Party).

### 3.3 Luxa's gap

- `color8` (via `luxa-color`) already provides `Crgb`/`Chsv`, `blend`,
  `nscale8`, 16/32/256-entry palettes with `color_from_palette*`, gradient
  conversion and `heat_color`.
- **Missing:**
  - wave maths: `sin8`, `beatsin*`, `cubicwave8`, `triwave8`;
  - noise: `perlin8`, `perlin16`;
  - randomness inside `Ctx`;
  - palette and `custom1–3`/`check1–3` in `Params`;
  - per-segment persistent pixels.
- Per-effect memory (`SEGENV`) already has an equivalent: each compositor slot
  owns its effect instance. Effects needing buffers (`allocateData`) need a
  no-allocator budget: const-generic arrays sized by the firmware.

### 3.4 Porting order

The 115 plain 1D effects (Solid included), profiled from their functions and
the helpers they call:

- **93** sample palettes.
- **40** read back the previous frame or keep per-segment data (28 of them
  allocate data).
- **8** more need noise but nothing else.
- **66** besides Solid need none of these (Rainbow, one of them, is already
  ported).

| Wave | Criterion | Effects (id) | Needs first |
|---|---|---|---|
| **A** | no frame readback, no data, no noise (65) | Blink 1, Breathe 2, Wipe 3, Wipe Random 4, Random Colors 5, Sweep 6, Colorloop 8, Scan 10, Scan Dual 11, Fade 12, Theater 13, Theater Rainbow 14, Running 15, Saw 16, Sparkle 20, Sparkle Dark 21, Sparkle+ 22, Strobe 23, Strobe Rainbow 24, Strobe Mega 25, Blink Rainbow 26, Chase 28, Chase Random 29, Chase Rainbow 30, Chase Flash 31, Chase Flash Rnd 32, Rainbow Runner 33, Colorful 34, Traffic Light 35, Sweep Random 36, Chase 2 37, Stream 39, Fire Flicker 45, Gradient 46, Loading 47, Two Dots 50, Running Dual 52, Chase 3 54, Tri Wipe 55, Tri Fade 56, Lightning 57, ICU 58, Stream 2 61, Bpm 68, Lake 75, Railway 78, Twinklefox 80, Twinklecat 81, Solid Pattern 83, Solid Pattern Tri 84, Spots 85, Spots Fade 86, Glitter 87, Plasma 97, Percent 98, Heartbeat 100, Pacifica 101, Solid Glitter 103, Sunrise 104, Twinkleup 106, Sine 108, Flow 110, Washing Machine 113, Flow Stripe 179, Wavesins 184 | palettes in `Params`, `c1–c3`/`o1–o3`, wave maths, randomness |
| **B** | noise only (8) | Fill Noise 69, Noise 1–4 70–73, Phased 105, Phased Noise 109, Color Clouds 218 | perlin noise |
| **C** | frame readback, no data (12) | Twinkle 17, Scanner 40, Lighthouse 41, Scanner Dual 60, Pride 2015 63, Juggle 64, Colorwaves 67, Sinelon 92, Sinelon Dual 93, Sinelon Rainbow 94, Chunchun 111, Perlin Move 147 | per-segment persistent pixels |
| **D** | per-segment data (28) | Dynamic 7, Dissolve 18, Dissolve Rnd 19, Android 27, Aurora 38, Tetrix 44, Rolling Balls 48, Fairy 49, Fairytwinkle 51, Multi Comet 59, Oscillate 62, Fire 2012 66, Colortwinkles 74, Meteor 76, Candle 88, Fireworks Starburst 89, Bouncing Balls 91, Popcorn 95, Drip 96, Candle Multi 102, Noise Pal 107, Dancing Shadows 112, Blends 115, TV Simulator 116, Dynamic Smooth 117, PacMan 151, Shimmer 161, Slow Transition 219 | effect data budget (and C's buffers for the 11 that also read back) |
| **E** | 1D audio (24) | the `v`/`f` 1D effects | a sound source in `Ctx` |
| **F** | 2D (62) and 1D particle-system (15) | — | matrix geometry (plan step 37), particle system |

Sunrise (104, wave A) is also what nightlight mode 3 runs (§7.3).

Every ported effect also needs its descriptor string (already parsed by
`luxa-effect::Descriptor`), so `/json/eff`, `fxdef` defaults and the sliders
clients show come for free.

---

## 4. Rank 3 — Palettes

### 4.1 The id space (`const.h`, `FX_fcn.cpp: loadPalette`)

| Ids | What | Source |
|---|---|---|
| 0 | **Default** — the effect's own default (`pal=` in its descriptor), else Party; and "use segment colours" inside `color_from_palette` | virtual |
| 1 | **Random Cycle** — regenerated every `rpc` s, blended (§2.4) | generated |
| 2 | Color 1 — primary only | segment colours |
| 3 | Colors 1&2 — `(prim, prim, sec, sec)` | segment colours |
| 4 | Color Gradient — `(ter, sec, prim)` | segment colours |
| 5 | Colors Only — primary/secondary halves, or thirds with tertiary | segment colours |
| 6–12 | FastLED: Party, Cloud, Lava, Ocean, Forest, Rainbow, Rainbow Bands — Party, Rainbow and Rainbow Bands are gamma-corrected (`_gc22`) variants | 16-entry tables |
| 13–71 | 59 cpt-city gradient palettes (Sunset, Rivendell, Aurora, …) — `index,r,g,b` stops, up to 18 | progmem byte arrays → 16-entry palette |
| 72–200 | user custom palettes `/paletteN.json` (two formats: `[i,r,g,b,…]` or `[i,"RRGGBB",…]`), growing down from 200 | filesystem |
| 201–255 | usermod palettes, growing down from 255 | runtime |

Palettes are 16 entries, sampled with `ColorFromPalette` and a global blend
mode `paletteBlend`: 0 wraps when moving, 1 always wraps, 2 never wraps, 3 no
interpolation.

### 4.2 Luxa's gap

- `color8` ships Party, Cloud, Lava, Ocean, Forest, Rainbow and Rainbow Stripe
  as stock FastLED tables. The three `_gc22` variants differ and would need
  their own tables.
- Needed:
  - the 59 gradient byte arrays;
  - dynamic palettes 2–5, built from segment colours per frame;
  - a random palette generator (harmonic and plain);
  - the palette-0 rule;
  - the names list for `/json/pal`.
- Custom palettes (72–200) need storage (rank 6).
- A palette is 48 bytes as `CrgbPalette16`; building it per segment per frame
  is cheap, and the transition blend (§2.2) needs one more per transitioning
  segment.

---

## 5. Rank 4 — Output fidelity

### 5.1 The reference frame pipeline (`FX_fcn.cpp: service, blendSegment, show`)

1. **Service** at the target frame rate (default **42 fps**, `WLED_FPS`), on
   one shared clock `strip.now = millis() + timebase`.
2. **Effects draw each active segment into its own buffer.** A segment in
   transition also runs its old effect.
3. **Blend into the frame buffer** (cleared each show), segment by segment in
   order, applying:
   - blend mode `bm` (17 modes: top, bottom, add, subtract, difference,
     average, multiply, divide, lighten, darken, screen, overlay, hardlight,
     softlight, dodge, burn, stencil);
   - opacity (transitioned);
   - grouping, spacing and offset;
   - reverse and mirror;
   - transition clipping.
4. **Gamma** on colours (`gammaCorrectCol`, **on** by default, γ = 2.2),
   applied after blending and before brightness. Brightness gamma
   (`gammaCorrectBri`) is off by default.
5. **Per bus:**
   - white balance from CCT (`colorBalanceFromKelvin`);
   - auto-white for RGBW (`autoWhiteCalc`);
   - global brightness with video scaling (`color_fade(c, bri, true)`);
   - CCT white-channel split;
   - colour order;
   - the driver.
6. **Current limiter (ABL)** before sending:
   - estimates per-bus current as `Σchannels × mA_per_LED / (255 × channels)`
     plus 1 mA standby per LED;
   - subtracts the ESP's own share;
   - scales brightness to stay under the limit.
   - Defaults: **850 mA** global, **55 mA** per LED; a WS2815 model is
     available.

### 5.2 Luxa's gap and implications

- **Gamma table:** 256-entry `const` in `luxa-output`, applied before
  brightness, switchable.
- **Brightness:** switch from `nscale8` to video scaling, so a dimmed pixel
  never goes fully dark.
- **ABL:** a pure function of the finished frame, the LED count and a
  milliamp budget. It belongs in `luxa-output`, reports usage for
  `info.leds.pwr`, and makes `maxpwr` honest.
- **White channel:** `Rgbw` state exists, but the pipeline is `Crgb`; RGBW
  strips and auto-white need a 4-channel path through canvas, output and wire.
- **Blend modes:** done. The compositor blends each segment's frame onto
  what lies beneath by its `bm`, then mixes in its opacity.
- **Frame rate:** 62 fps vs 42. Effects that count frames (`SEGENV.call`)
  rather than read time would run faster; ported effects should be checked, or
  the default matched.

---

## 6. Rank 5 — Segment geometry and effect parameters

Already scheduled as plan steps 25–27; listed here for render consequences.

| Keys | Renderer must |
|---|---|
| `grp` / `spc` / `of` | expand each virtual pixel to `grp` physical pixels, skip `spc`, rotate by `of` (effects see the virtual length) |
| `c1`–`c3`, `o1`–`o3` | pass to effects; descriptors name them |
| `frz` | skip the effect call but keep showing its last frame (needs the per-segment buffer) |
| `cct` | per-segment CCT into the white-balance stage |
| `i` | individual pixels: freeze the segment, write pixels directly |
| `si`, `m12` | sound simulation mode; 1D-to-2D mapping (Tier C) |

---

## 7. Rank 6 — Presets, playlists, nightlight, timers

All of these need **persistent storage** and some need a **wall clock**; Luxa
has neither yet.

### 7.1 Presets (`presets.cpp`)

- Stored in `/presets.json`, ids 1–250, each a state document plus `n`
  (name) and `ql` (quick-load label).
- **Save** (`psave`) options: `ib` include brightness, `sb` include segment
  bounds, `sc` selected segments only, `ledmap`. Preset 255 is a temporary
  preset, always saved with brightness and bounds.
- **Apply** (`ps`, from any source) is deferred: `applyPreset()` sets
  `presetToApply` and `handlePresets()` loads it from the loop, then runs
  `deserializeState` with no notification. Applying a preset may itself start
  a playlist.
- Boot preset (`info.leds.bootps`), preset cycling syntax `"1~5~"`, and
  `/presets.json` served raw.

### 7.2 Playlists (`playlist.cpp`)

- `playlist: {ps[], dur[], transition[], repeat, end, r}`:
  - durations and transitions in tenths of a second, single value or per
    entry;
  - `dur` 0 means infinite;
  - `repeat` 0 means forever, negative means forever and shuffled;
  - `end` is the preset to apply when done (255 restores);
  - `r` shuffles.
- `handlePlaylist()` advances on time or `np`; an effect change from a client
  stops the playlist.

### 7.3 Nightlight (`led.cpp: handleNightlight`, 10 updates/s)

- **Mode 0 (set):** at the end, set brightness to `tbri`.
- **Mode 1 (fade):** brightness moves linearly from start to `tbri` over
  `dur` minutes.
- **Mode 2 (colour fade):** fade plus primary colour moves toward secondary.
- **Mode 3 (sunrise/sunset):** switches the first selected segment to the
  **Sunrise** effect (id 104) with speed = minutes (+60 for sunset), then
  restores the previous effect.
- When done: optionally applies preset `macroNl`, and clears `nl.on`.
  `nl.rem` reports remaining seconds.

### 7.4 Timers and time (`ntp.cpp`)

- NTP with timezone.
- Up to **64 timers** (16 on ESP8266), each firing a preset. A timer has:
  - hour and minute, or an offset from sunrise/sunset (computed from
    latitude/longitude);
  - a weekday mask;
  - an optional month/day range.
- Checked once a minute. Also a countdown target, used by the clock overlay.

### 7.5 Implications

- A storage crate or module over flash (NVS-style or a small filesystem) is a
  prerequisite. Presets are JSON documents, which `luxa-api` can already parse
  and write.
- Nightlight is engine logic on a clock (brightness target over time) plus
  one effect (Sunrise). It fits the engine if time arrives as a command, the
  way `Ctx` keeps effects clock-free.
- Timers need a real-time clock source: SNTP over `embassy-net`.

---

## 8. Rank 7 — Sync and realtime

### 8.1 Device sync (`udp.cpp: notify, parseNotifyPacket`)

- UDP port **21324** (plus supplemental 65506). Packet: `41 + segments ×
  UDP_SEG_SIZE` bytes. It carries:
  - call mode and brightness;
  - primary, secondary and tertiary colours;
  - nightlight state;
  - the main segment's effect, speed, intensity and palette;
  - transition duration and a follow-up flag;
  - the effect **timebase**;
  - time source and Unix time in milliseconds;
  - then every segment.
- Sync groups: 8-bit send and receive masks. Receive filters for brightness,
  colour, effects and palette.
- `udpn.nn` suppresses one broadcast. Node discovery (`sendSysInfoUDP`) fills
  `/json/nodes`.
- A shared timebase keeps effects phase-aligned across devices.

### 8.2 Realtime pixel streams

| Protocol | Where |
|---|---|
| WARLS (1), DRGB (2), DRGBW (3), DNRGB (4), DNRGBW (5) | UDP, notifier port, first byte selects |
| Hyperion | UDP 19446 |
| TPM2.net | UDP |
| E1.31 / sACN | UDP 5568, universes, several DMX modes (single RGB, effect control, per-LED RGB/RGBW, per-segment, preset) |
| Art-Net | UDP (configured port; standard 6454) |
| DDP | UDP |
| Adalight / serial | `wled_serial.cpp` |
| WebSocket binary | E1.31, Art-Net, DDP payloads over `/ws` |
| JSON or HTTP API over UDP | a `{` or letter as first byte |

- Realtime takes over the frame until `realtimeTimeoutMs` (default 2500 ms)
  passes without data. `lor` overrides it; `live` enters it manually.
- The main segment only, if configured (`useMainSegmentOnly`).
- **Outputs:** network LED buses (virtual types 80–89) send DDP, E1.31 or
  Art-Net to other devices. DMX output and input run over serial.

---

## 9. Rank 8 — Inputs and integrations

| Module | Does |
|---|---|
| `button.cpp` | push (active low/high), switch, PIR, touch, touch switch, analog, inverted analog; short, long and double press run presets |
| `ir.cpp` | 24/40/44/21/6/9-key remotes and JSON-defined remotes: brightness, effect, palette, speed, intensity, colour, white |
| `remote.cpp` | WiZmote over ESP-NOW: on/off, brightness, night mode, presets |
| `mqtt.cpp` | brightness and colour topics, JSON API over MQTT, state publish |
| `alexa.cpp` | Espalexa device: on/off, brightness, colour |
| `hue.cpp` | polls a Philips Hue light and follows it |
| `improv.cpp` | Improv Wi-Fi provisioning over serial |
| `ota_update.cpp` | firmware and bootloader updates over HTTP, validated |
| `overlay.cpp` | analog clock and countdown overlays |
| `image_loader.cpp`, `fontmanager.cpp` | GIF playback into a segment; fonts for scrolling text |
| `usermods/` | ~75 optional modules: audio-reactive (microphone FFT), sensors, displays, rotary encoders, relays, word clocks, … |

---

## 10. Rank 9 — Hardware breadth (`bus_manager`, `const.h`)

- **LED types:**
  - one-wire digital: WS281x RGB/WWA/1-ch, SK6812 RGBW, TM1814, TM1829,
    TM1914, UCS8903/8904, APA106, FW1906, WS2805, SM16825, GS8608;
  - on/off relay;
  - PWM 1–6 channels (analog RGB, CCT);
  - two-wire SPI: WS2801, APA102, LPD8806, P9813, LPD6803;
  - HUB75 matrix panels;
  - network (DDP, E1.31, Art-Net).
- Several buses at once, per-bus colour order map, skipped first LEDs (status
  LED), LED maps (`ledmap.json` remapping), 2D matrices assembled from panels.
- Luxa has one WS2812 bus over RMT; `luxa-wire`'s isolation already anticipates
  more chipsets.

---

## 11. Ranked summary

| # | Item | Makes true | Depends on | Size | Plan |
|---|---|---|---|---|---|
| 1 | Global brightness fade | `transition`, `tt`, on/off fades | — | S | 29 |
| 2 | Segment colour/opacity/palette fade | `transition` for colour and palette changes | per-segment compositor state | M | 29 |
| 3 | Palettes in `Params` + dynamic 2–5 + FastLED 6–12 | `pal` 0–12 | — | S | new |
| 4 | Effect runtime: waves, randomness, `c1–c3`/`o1–o3` | prerequisite for wave A | — | S | 26 |
| 5 | Effects wave A (65) | the most common `fx` choices | 3, 4 | M | new |
| 6 | Gamma + video-scaled brightness | colours match the reference | — | S | new |
| 7 | Per-segment persistent pixel buffers | frame readback, `frz`, blend modes, effect crossfade | memory budget | M | new |
| 8 | Effect crossfade | `transition` on `fx` changes | 7 | M | 29 |
| 9 | Effects waves B–C (20) | noise, trails, sinelons, Pride, Colorwaves | perlin noise, 7 | M | new |
| 10 | 59 gradient palettes + names + random palette | `pal` 13–71, 1 | 3 | M | new |
| 11 | Current limiter | `maxpwr`, `pwr`, safe power supplies | — | S | new |
| 12 | Grouping, spacing, offset, freeze, CCT | Tier B segment keys | 7 | M | 25 |
| 13 | Effects wave D (28) | fire, meteors, candles, balls, twinkles | effect data budget | L | new |
| 14 | Storage | presets, custom palettes | flash layout | M | 33 |
| 15 | Presets, playlists | `ps`, `psave`, `playlist` | 14 | M | 33–34 |
| 16 | Nightlight | `nl` | Sunrise effect, clock | S | 35 |
| 17 | Transition styles | `bs` | 8 | M | 36 |
| 18 | SNTP + timers | scheduled presets | 14, 15 | M | new |
| 19 | UDP sync | multi-device `udpn` | timebase | M | 28 / new |
| 20 | Realtime (DDP, E1.31, Art-Net) | `live`, `lor`, pixel streaming | output takeover | M | 38 |
| 21 | 2D matrices | 2D keys and the 62 2D effects | geometry | L | 37 |
| 22 | RGBW / white channel | `col` W, auto-white | 4-channel pipeline | M | new |
| 23 | Inputs (buttons, IR) | physical control | presets | M | new |
| 24 | Audio effects | 24 1D audio effects (37 counting 2D and particle) | sound source | M | new |

"new" means not yet in `json-api-plan.md`. S / M / L are relative effort
estimates, not commitments.
