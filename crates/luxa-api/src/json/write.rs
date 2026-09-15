//! Writing responses.
//!
//! Every writer takes a [`core::fmt::Write`], so a runtime can fill a fixed
//! buffer, stream to a socket, or [`Measure`] a response before sending it.
//!
//! The documents follow the reference implementation's shape. Keys for
//! features Luxa does not model yet (nightlight, sync groups, segment grouping,
//! effect custom sliders…) are written with a freshly booted fixture's values,
//! so clients that expect them find them.

use core::fmt::{self, Write};

use luxa_msg::{ErrorCode, LightCaps, Segment, State};

/// Effect and palette names, for the catalogue endpoints and the counts in the
/// info document.
pub trait Names {
    /// One past the highest effect id.
    fn effect_count(&self) -> u16;
    /// The name of effect `id`, or `None` for an id with no effect.
    fn effect_name(&self, id: u8) -> Option<&str>;
    /// One past the highest palette id.
    fn palette_count(&self) -> u16;
    /// The name of palette `id`, or `None` for an id with no palette.
    fn palette_name(&self, id: u8) -> Option<&str>;
}

/// Device facts for the info document that the state does not carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Info<'a> {
    /// The fixture's display name.
    pub name: &'a str,
    /// Firmware version string.
    pub version: &'a str,
    /// Firmware version as a number.
    pub version_id: u32,
    /// Brand reported to clients.
    pub brand: &'a str,
    /// Product reported to clients.
    pub product: &'a str,
    /// Chip or platform name.
    pub arch: &'a str,
    /// MAC address, lowercase hex without separators.
    pub mac: &'a str,
    /// IP address, or empty when not connected.
    pub ip: &'a str,
    /// Total LEDs.
    pub led_count: u16,
    /// Frames per second.
    pub fps: u16,
    /// Estimated current draw, in milliamps.
    pub power_ma: u32,
    /// Current limit, in milliamps; `0` when there is none.
    pub max_power_ma: u32,
    /// Port for state sync with peers.
    pub udp_port: u16,
    /// Connected WebSocket clients; `-1` when WebSockets are unavailable.
    pub websocket_clients: i32,
    /// Seconds since boot.
    pub uptime_s: u32,
    /// Free heap, in bytes.
    pub free_heap: u32,
    /// Build option bits.
    pub options: u16,
    /// The wireless connection.
    pub wifi: Wifi<'a>,
}

/// The wireless connection, for the info document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wifi<'a> {
    /// Access point address.
    pub bssid: &'a str,
    /// Signal strength, in dBm.
    pub rssi: i16,
    /// Signal quality, `0`–`100`.
    pub signal: u8,
    /// Radio channel.
    pub channel: u8,
    /// Band, e.g. `"2.4GHz"`, or `"not connected"`.
    pub band: &'a str,
    /// Whether the fixture is running its own access point.
    pub access_point: bool,
}

/// A writer that only counts bytes: for a `Content-Length`, or to size a
/// buffer.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Measure(pub usize);

impl Write for Measure {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.0 += s.len();
        Ok(())
    }
}

/// A writer that keeps one window of the output: the bytes from `offset`
/// onwards, as many as fit its buffer.
///
/// This streams a document of any size through a small buffer without an
/// allocator: write the whole document once per window, send
/// [`filled`](Self::filled), advance the offset by that much, and repeat
/// until [`is_complete`](Self::is_complete). Writing stops with an error as
/// soon as the buffer overflows, so the document must be written the same way
/// every time.
#[derive(Debug)]
pub struct Window<'a> {
    buffer: &'a mut [u8],
    offset: usize,
    position: usize,
    filled: usize,
    overflowed: bool,
}

impl<'a> Window<'a> {
    /// A window over `buffer` that skips the first `offset` bytes written.
    pub fn new(buffer: &'a mut [u8], offset: usize) -> Self {
        Self {
            buffer,
            offset,
            position: 0,
            filled: 0,
            overflowed: false,
        }
    }

    /// The bytes captured so far.
    pub fn filled(&self) -> &[u8] {
        &self.buffer[..self.filled]
    }

    /// Whether the output ended inside this window, so no bytes are left for
    /// another one.
    pub fn is_complete(&self) -> bool {
        !self.overflowed
    }
}

impl Write for Window<'_> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let bytes = s.as_bytes();
        let start = self.position;
        self.position += bytes.len();
        if self.position <= self.offset {
            return Ok(());
        }
        let skip = self.offset.saturating_sub(start);
        let wanted = &bytes[skip..];
        let room = self.buffer.len() - self.filled;
        let taken = wanted.len().min(room);
        self.buffer[self.filled..self.filled + taken].copy_from_slice(&wanted[..taken]);
        self.filled += taken;
        if taken < wanted.len() {
            self.overflowed = true;
            return Err(fmt::Error);
        }
        Ok(())
    }
}

/// `{"success":true}`: the reply to a change that did not ask for state.
pub fn write_success<W: Write>(out: &mut W) -> fmt::Result {
    out.write_str(r#"{"success":true}"#)
}

/// `{"error":<code>}`.
pub fn write_error<W: Write>(out: &mut W, code: ErrorCode) -> fmt::Result {
    write!(out, r#"{{"error":{}}}"#, code.code())
}

/// The state document.
pub fn write_state<W: Write, const SEGMENTS: usize, const NAME: usize>(
    out: &mut W,
    state: &State<SEGMENTS, NAME>,
) -> fmt::Result {
    write!(
        out,
        r#"{{"on":{},"bri":{},"transition":{},"bs":0,"ps":-1,"pl":-1,"ledmap":0,"#,
        state.is_on(),
        state.last_brightness,
        state.transition.as_deciseconds(),
    )?;
    out.write_str(concat!(
        r#""nl":{"on":false,"dur":60,"mode":1,"tbri":0,"rem":-1},"#,
        r#""udpn":{"send":false,"recv":true,"sgrp":1,"rgrp":1},"lor":0,"#,
    ))?;
    write!(out, r#""mainseg":{},"seg":["#, state.main_segment)?;
    // Colours carry a white channel only when some segment can show one.
    let white = state
        .active_segments()
        .any(|(_, s)| s.caps.contains(LightCaps::WHITE));
    for (n, (id, segment)) in state.active_segments().enumerate() {
        if n > 0 {
            out.write_char(',')?;
        }
        write_segment(out, id, segment, white)?;
    }
    out.write_str("]}")
}

fn write_segment<W: Write, const NAME: usize>(
    out: &mut W,
    id: usize,
    seg: &Segment<NAME>,
    white: bool,
) -> fmt::Result {
    write!(
        out,
        r#"{{"id":{id},"start":{},"stop":{},"len":{},"grp":1,"spc":0,"of":0,"on":{},"frz":false,"bri":{},"cct":127,"set":0,"lc":{}"#,
        seg.start,
        seg.stop,
        seg.len(),
        seg.on,
        // An opacity of zero reads back as full.
        if seg.opacity == 0 { 255 } else { seg.opacity },
        seg.caps.bits(),
    )?;
    if !seg.name.is_empty() {
        out.write_str(r#","n":"#)?;
        write_string(out, seg.name.as_str())?;
    }
    out.write_str(r#","col":["#)?;
    for (n, c) in seg.colors.iter().enumerate() {
        if n > 0 {
            out.write_char(',')?;
        }
        if white {
            write!(out, "[{},{},{},{}]", c.r, c.g, c.b, c.w)?;
        } else {
            write!(out, "[{},{},{}]", c.r, c.g, c.b)?;
        }
    }
    write!(
        out,
        r#"],"fx":{},"sx":{},"ix":{},"pal":{},"c1":128,"c2":128,"c3":16,"sel":{},"rev":{},"mi":{},"o1":false,"o2":false,"o3":false,"si":0,"m12":0,"bm":0}}"#,
        seg.effect.0,
        seg.speed,
        seg.intensity,
        seg.palette.0,
        seg.selected,
        seg.reverse,
        seg.mirror,
    )
}

/// The info document.
pub fn write_info<W: Write, const SEGMENTS: usize, const NAME: usize>(
    out: &mut W,
    info: &Info<'_>,
    state: &State<SEGMENTS, NAME>,
    names: &impl Names,
) -> fmt::Result {
    out.write_str(r#"{"ver":"#)?;
    write_string(out, info.version)?;
    write!(
        out,
        r#","vid":{},"leds":{{"count":{},"pwr":{},"fps":{},"maxpwr":{},"maxseg":{},"bootps":0,"seglc":["#,
        info.version_id,
        info.led_count,
        info.power_ma,
        info.fps,
        info.max_power_ma,
        state.segment_capacity(),
    )?;
    let mut lc = 0u8;
    let mut rgbw = false;
    for (n, (_, segment)) in state.active_segments().enumerate() {
        if n > 0 {
            out.write_char(',')?;
        }
        write!(out, "{}", segment.caps.bits())?;
        lc |= segment.caps.bits();
        rgbw |= segment.caps.contains(LightCaps::RGB) && segment.caps.contains(LightCaps::WHITE);
    }
    write!(
        out,
        r#"],"lc":{lc},"rgbw":{rgbw},"wv":{},"cct":{}}},"str":false,"name":"#,
        lc & LightCaps::WHITE.bits(),
        lc & LightCaps::CCT.bits(),
    )?;
    write_string(out, info.name)?;
    write!(
        out,
        concat!(
            r#","udpport":{},"simplifiedui":false,"live":false,"liveseg":-1,"lm":"","lip":"","#,
            r#""ws":{},"fxcount":{},"palcount":{},"cpalcount":0,"umpalcount":0,"cpalmax":0,"#,
            r#""maps":[{{"id":0}}],"wifi":{{"bssid":"#,
        ),
        info.udp_port,
        info.websocket_clients,
        names.effect_count(),
        names.palette_count(),
    )?;
    write_string(out, info.wifi.bssid)?;
    write!(
        out,
        r#","rssi":{},"signal":{},"channel":{},"band":"#,
        info.wifi.rssi, info.wifi.signal, info.wifi.channel,
    )?;
    write_string(out, info.wifi.band)?;
    write!(
        out,
        r#","ap":{}}},"fs":{{"u":0,"t":0,"pmt":0}},"ndc":0,"arch":"#,
        info.wifi.access_point
    )?;
    write_string(out, info.arch)?;
    write!(
        out,
        r#","freeheap":{},"uptime":{},"opt":{},"brand":"#,
        info.free_heap, info.uptime_s, info.options,
    )?;
    write_string(out, info.brand)?;
    out.write_str(r#","product":"#)?;
    write_string(out, info.product)?;
    out.write_str(r#","mac":"#)?;
    write_string(out, info.mac)?;
    out.write_str(r#","ip":"#)?;
    write_string(out, info.ip)?;
    out.write_char('}')
}

/// `{"state":…,"info":…}`.
pub fn write_state_and_info<W: Write, const SEGMENTS: usize, const NAME: usize>(
    out: &mut W,
    state: &State<SEGMENTS, NAME>,
    info: &Info<'_>,
    names: &impl Names,
) -> fmt::Result {
    out.write_str(r#"{"state":"#)?;
    write_state(out, state)?;
    out.write_str(r#","info":"#)?;
    write_info(out, info, state, names)?;
    out.write_char('}')
}

/// `{"state":…,"info":…,"effects":[…],"palettes":[…]}`.
pub fn write_everything<W: Write, const SEGMENTS: usize, const NAME: usize>(
    out: &mut W,
    state: &State<SEGMENTS, NAME>,
    info: &Info<'_>,
    names: &impl Names,
) -> fmt::Result {
    out.write_str(r#"{"state":"#)?;
    write_state(out, state)?;
    out.write_str(r#","info":"#)?;
    write_info(out, info, state, names)?;
    out.write_str(r#","effects":"#)?;
    write_effect_names(out, names)?;
    out.write_str(r#","palettes":"#)?;
    write_palette_names(out, names)?;
    out.write_char('}')
}

/// Effect names in id order. An id with no effect is `"RSVD"`, so a name's
/// position is always its id.
pub fn write_effect_names<W: Write>(out: &mut W, names: &impl Names) -> fmt::Result {
    write_names(out, names.effect_count(), |id| {
        names.effect_name(id).unwrap_or("RSVD")
    })
}

/// Palette names in id order.
pub fn write_palette_names<W: Write>(out: &mut W, names: &impl Names) -> fmt::Result {
    write_names(out, names.palette_count(), |id| {
        names.palette_name(id).unwrap_or("")
    })
}

fn write_names<'n, W: Write>(out: &mut W, count: u16, name: impl Fn(u8) -> &'n str) -> fmt::Result {
    out.write_char('[')?;
    for id in 0..count.min(256) {
        if id > 0 {
            out.write_char(',')?;
        }
        write_string(out, name(id as u8))?;
    }
    out.write_char(']')
}

/// A JSON string literal, escaped.
fn write_string<W: Write>(out: &mut W, text: &str) -> fmt::Result {
    out.write_char('"')?;
    let mut start = 0;
    for (i, c) in text.char_indices() {
        let escape = match c {
            '"' => "\\\"",
            '\\' => "\\\\",
            '\n' => "\\n",
            '\r' => "\\r",
            '\t' => "\\t",
            '\u{8}' => "\\b",
            '\u{c}' => "\\f",
            c if u32::from(c) < 0x20 => {
                out.write_str(&text[start..i])?;
                write!(out, "\\u{:04x}", u32::from(c))?;
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        out.write_str(&text[start..i])?;
        out.write_str(escape)?;
        start = i + c.len_utf8();
    }
    out.write_str(&text[start..])?;
    out.write_char('"')
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_msg::{Layout, Name, Rgbw};

    /// A fixed buffer to write into, without an allocator.
    struct Buf {
        bytes: [u8; 4096],
        len: usize,
    }

    impl Buf {
        const fn new() -> Self {
            Self {
                bytes: [0; 4096],
                len: 0,
            }
        }
        fn as_str(&self) -> &str {
            core::str::from_utf8(&self.bytes[..self.len]).unwrap()
        }
    }

    impl Write for Buf {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let end = self.len + s.len();
            if end > self.bytes.len() {
                return Err(fmt::Error);
            }
            self.bytes[self.len..end].copy_from_slice(s.as_bytes());
            self.len = end;
            Ok(())
        }
    }

    fn written(f: impl FnOnce(&mut Buf) -> fmt::Result) -> Buf {
        let mut buf = Buf::new();
        f(&mut buf).unwrap();
        buf
    }

    #[test]
    fn replies() {
        assert_eq!(written(write_success).as_str(), r#"{"success":true}"#);
        assert_eq!(
            written(|b| write_error(b, ErrorCode::Json)).as_str(),
            r#"{"error":9}"#
        );
    }

    #[test]
    fn windows_stream_a_document_in_pieces() {
        let document = written(|b| write_error(b, ErrorCode::NotImplemented));
        let expected = document.as_str();

        for size in 1..=expected.len() + 1 {
            let mut streamed = Buf::new();
            let mut chunk = [0u8; 32];
            let mut offset = 0;
            loop {
                let mut window = Window::new(&mut chunk[..size], offset);
                let result = write_error(&mut window, ErrorCode::NotImplemented);
                assert_eq!(result.is_ok(), window.is_complete());
                streamed
                    .write_str(core::str::from_utf8(window.filled()).unwrap())
                    .unwrap();
                offset += window.filled().len();
                if window.is_complete() {
                    break;
                }
            }
            assert_eq!(streamed.as_str(), expected, "window of {size}");
        }
    }

    #[test]
    fn a_window_that_ends_with_the_document_is_complete() {
        let mut chunk = [0u8; 16];
        let mut window = Window::new(&mut chunk, 0);
        write_success(&mut window).unwrap();
        assert!(window.is_complete());
        assert_eq!(window.filled(), br#"{"success":true}"#);
    }

    #[test]
    fn strings_are_escaped() {
        let out = written(|b| write_string(b, "a\"b\\c\nd\u{1}é"));
        // A control character without a short escape gets a six-character one.
        let expected = concat!(r#""a\"b\\c\nd"#, "\\", "u0001é\"");
        assert_eq!(out.as_str(), expected);
    }

    #[test]
    fn a_name_is_written_only_when_set() {
        let mut state = State::<2, 16>::new(Layout::new(10));
        assert!(
            !written(|b| write_state(b, &state))
                .as_str()
                .contains(r#""n":"#)
        );
        state.segments_mut()[0].name = Name::new("Desk");
        assert!(
            written(|b| write_state(b, &state))
                .as_str()
                .contains(r#""n":"Desk""#)
        );
    }

    #[test]
    fn white_channels_appear_with_a_white_capable_segment() {
        let mut state = State::<2, 16>::new(Layout::new(10));
        state.segments_mut()[0].colors[0] = Rgbw::new(1, 2, 3, 4);
        assert!(
            written(|b| write_state(b, &state))
                .as_str()
                .contains(r#""col":[[1,2,3],"#)
        );
        state.segments_mut()[0].caps = LightCaps::RGB.union(LightCaps::WHITE);
        assert!(
            written(|b| write_state(b, &state))
                .as_str()
                .contains(r#""col":[[1,2,3,4],"#)
        );
    }

    #[test]
    fn a_zero_opacity_reads_back_as_full() {
        let mut state = State::<2, 16>::new(Layout::new(10));
        state.segments_mut()[0].opacity = 0;
        assert!(
            written(|b| write_state(b, &state))
                .as_str()
                .contains(r#""bri":255,"cct""#)
        );
    }

    #[test]
    fn deleted_segments_are_left_out_but_ids_are_kept() {
        let mut state = State::<3, 16>::new(Layout::new(30));
        state.push_segment(Segment::new(10, 20)).unwrap();
        state.push_segment(Segment::new(20, 30)).unwrap();
        state.segments_mut()[1].stop = 0;
        let out = written(|b| write_state(b, &state));
        assert!(out.as_str().contains(r#"{"id":0,"#));
        assert!(!out.as_str().contains(r#"{"id":1,"#));
        assert!(out.as_str().contains(r#"{"id":2,"#));
    }

    #[test]
    fn measuring_counts_what_writing_writes() {
        let state = State::<2, 16>::new(Layout::new(10));
        let mut measure = Measure::default();
        write_state(&mut measure, &state).unwrap();
        assert_eq!(measure.0, written(|b| write_state(b, &state)).len);
    }

    struct Two;
    impl Names for Two {
        fn effect_count(&self) -> u16 {
            3
        }
        fn effect_name(&self, id: u8) -> Option<&str> {
            [Some("Solid"), None, Some("Rainbow")][usize::from(id)]
        }
        fn palette_count(&self) -> u16 {
            1
        }
        fn palette_name(&self, _: u8) -> Option<&str> {
            Some("Default")
        }
    }

    #[test]
    fn effect_gaps_keep_positions() {
        assert_eq!(
            written(|b| write_effect_names(b, &Two)).as_str(),
            r#"["Solid","RSVD","Rainbow"]"#
        );
        assert_eq!(
            written(|b| write_palette_names(b, &Two)).as_str(),
            r#"["Default"]"#
        );
    }
}
