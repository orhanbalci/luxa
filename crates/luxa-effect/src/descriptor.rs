//! Effect descriptors: an effect's name and the controls a UI shows for it.
//!
//! A descriptor is one compact string, in the format existing LED controller
//! UIs already understand:
//!
//! ```text
//! Name@sliders;colours;palette;flags;defaults
//! Rainbow@!,Size;;!
//! ```
//!
//! | Section | Content |
//! |---|---|
//! | sliders | Up to eight entries: speed, intensity, custom 1–3, then checkboxes 1–3. `!` shows the control with its default label, empty hides it, anything else is its label. |
//! | colours | Up to three entries — primary, background, custom — by the same rules. |
//! | palette | `!` shows the palette control, empty hides it, anything else is its label. |
//! | flags | Characters: `0` works on a single LED, `1` one-dimensional, `2` two-dimensional, `v` volume-reactive, `f` frequency-reactive. |
//! | defaults | `key=value` pairs applied when the effect is chosen with defaults, read from the text after the *last* `;`. |
//!
//! A descriptor without `@` has no controls section: its effect shows speed,
//! intensity, every colour and the palette with default labels.
//!
//! Parsing is lazy and allocation-free: every accessor reads the string it is
//! given.

/// How a UI shows one control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Control<'a> {
    /// Not shown.
    Hidden,
    /// Shown with its default label.
    Default,
    /// Shown with this label.
    Label(&'a str),
}

impl<'a> Control<'a> {
    fn from_field(field: &'a str) -> Self {
        match field {
            "" => Self::Hidden,
            "!" => Self::Default,
            label => Self::Label(label),
        }
    }

    /// Whether the control is shown at all.
    pub const fn is_shown(self) -> bool {
        !matches!(self, Self::Hidden)
    }
}

/// What an effect supports, from its flags.
///
/// Flags are matched by character, as UIs match them: a section is not
/// validated, only searched.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Flags {
    /// `0`: works on a single LED.
    pub single_led: bool,
    /// `1`: one-dimensional.
    pub one_dimensional: bool,
    /// `2`: two-dimensional.
    pub two_dimensional: bool,
    /// `v`: reacts to volume.
    pub volume: bool,
    /// `f`: reacts to frequency.
    pub frequency: bool,
}

impl Flags {
    fn parse(field: &str) -> Self {
        Self {
            single_led: field.contains('0'),
            one_dimensional: field.contains('1'),
            two_dimensional: field.contains('2'),
            volume: field.contains('v'),
            frequency: field.contains('f'),
        }
    }

    /// Whether the effect works on a strip: effects that name neither
    /// dimension do, effects that name only two dimensions do not.
    pub const fn supports_1d(self) -> bool {
        self.one_dimensional || !self.two_dimensional
    }
}

/// A parsed view of an effect descriptor string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Descriptor<'a> {
    raw: &'a str,
}

impl<'a> Descriptor<'a> {
    /// A view of `raw`.
    pub const fn new(raw: &'a str) -> Self {
        Self { raw }
    }

    /// The descriptor string itself.
    pub const fn as_str(&self) -> &'a str {
        self.raw
    }

    /// The effect's display name: everything before `@`.
    pub fn name(&self) -> &'a str {
        self.raw.split_once('@').map_or(self.raw, |(name, _)| name)
    }

    /// Slider or checkbox `index`: `0` speed, `1` intensity, `2`–`4` custom
    /// sliders, `5`–`7` checkboxes.
    pub fn slider(&self, index: usize) -> Control<'a> {
        match self.controls() {
            None if index < 2 => Control::Default,
            None => Control::Hidden,
            Some(_) => self.field(0, index),
        }
    }

    /// Colour slot `index`: `0` primary, `1` background, `2` custom.
    pub fn color(&self, index: usize) -> Control<'a> {
        match self.controls() {
            None if index < 3 => Control::Default,
            None => Control::Hidden,
            Some(_) => self.field(1, index),
        }
    }

    /// The palette control.
    pub fn palette(&self) -> Control<'a> {
        match self.controls() {
            None => Control::Default,
            Some(_) => self.section(2).map_or(Control::Hidden, Control::from_field),
        }
    }

    /// The effect's flags; all unset when the descriptor has none.
    pub fn flags(&self) -> Flags {
        self.section(3).map_or(Flags::default(), Flags::parse)
    }

    /// The default value for `key` — `sx`, `ix`, `c1`–`c3`, `o1`–`o3`, `pal`,
    /// `m12`, `si`, `rev`, `mi`, `rY`, `mY` — if the descriptor sets one.
    pub fn default(&self, key: &str) -> Option<i32> {
        self.defaults()
            .find_map(|(k, value)| (k == key).then_some(value))
    }

    /// Every `key=value` default, in order.
    pub fn defaults(&self) -> impl Iterator<Item = (&'a str, i32)> + 'a {
        self.controls()
            .and_then(|controls| controls.rsplit_once(';'))
            .map_or("", |(_, last)| last)
            .split(',')
            .filter_map(|pair| {
                let (key, value) = pair.split_once('=')?;
                Some((key, parse_int(value)))
            })
    }

    fn controls(&self) -> Option<&'a str> {
        self.raw.split_once('@').map(|(_, controls)| controls)
    }

    fn section(&self, index: usize) -> Option<&'a str> {
        self.controls()?.split(';').nth(index)
    }

    fn field(&self, section: usize, index: usize) -> Control<'a> {
        self.section(section)
            .and_then(|s| s.split(',').nth(index))
            .map_or(Control::Hidden, Control::from_field)
    }
}

/// A leading integer, read the way C's `atoi` reads it: optional sign, then
/// digits up to the first non-digit; `0` when there are none.
fn parse_int(text: &str) -> i32 {
    let text = text.trim_start();
    let (negative, digits) = match text.as_bytes().first() {
        Some(b'-') => (true, &text[1..]),
        Some(b'+') => (false, &text[1..]),
        _ => (false, text),
    };
    let magnitude = digits
        .bytes()
        .take_while(u8::is_ascii_digit)
        .fold(0i32, |acc, d| {
            acc.saturating_mul(10).saturating_add(i32::from(d - b'0'))
        });
    if negative { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Control::{Default as Shown, Hidden, Label};

    #[test]
    fn a_descriptor_without_controls_shows_the_basics() {
        let d = Descriptor::new("Solid");
        assert_eq!(d.name(), "Solid");
        assert_eq!(
            [d.slider(0), d.slider(1), d.slider(2)],
            [Shown, Shown, Hidden]
        );
        assert_eq!([d.color(0), d.color(2), d.color(3)], [Shown, Shown, Hidden]);
        assert_eq!(d.palette(), Shown);
        assert_eq!(d.flags(), Flags::default());
        assert!(d.flags().supports_1d());
        assert_eq!(d.defaults().count(), 0);
    }

    #[test]
    fn three_sections() {
        let d = Descriptor::new("Rainbow@!,Size;;!");
        assert_eq!(d.name(), "Rainbow");
        assert_eq!(
            [d.slider(0), d.slider(1), d.slider(2)],
            [Shown, Label("Size"), Hidden]
        );
        assert_eq!(
            d.color(0),
            Hidden,
            "an empty colour section hides every colour"
        );
        assert_eq!(d.palette(), Shown);
        assert_eq!(d.flags(), Flags::default());
        assert_eq!(d.default("sx"), None, "the last section is the palette's");
    }

    #[test]
    fn eight_controls_flags_and_defaults() {
        let d = Descriptor::new(
            "Palette@Shift,Size,Rotation,,,Animate Shift,Animate Rotation,Anamorphic;;!;12;ix=112,c1=0,o1=1,o2=0,o3=1",
        );
        assert_eq!(d.slider(3), Hidden);
        assert_eq!(d.slider(5), Label("Animate Shift"));
        assert_eq!(d.slider(7), Label("Anamorphic"));
        assert_eq!(d.slider(8), Hidden);
        let flags = d.flags();
        assert!(flags.one_dimensional && flags.two_dimensional && flags.supports_1d());
        assert_eq!(d.default("ix"), Some(112));
        assert_eq!(d.default("o2"), Some(0));
        assert_eq!(d.default("sx"), None);
        assert_eq!(d.defaults().count(), 5);
    }

    #[test]
    fn defaults_come_from_after_the_last_separator() {
        // Four sections: the defaults sit where flags usually are.
        let d = Descriptor::new("Flow Stripe@Hue speed,Effect speed;;!;pal=11");
        assert_eq!(d.default("pal"), Some(11));
        assert!(d.flags().one_dimensional, "flags are matched by character");
    }

    #[test]
    fn labelled_colours_and_palette() {
        let d = Descriptor::new(
            "Akemi@Color speed,Dance;Head palette,Arms & Legs,Eyes & Mouth;Face palette;2f;si=0",
        );
        assert_eq!(d.color(1), Label("Arms & Legs"));
        assert_eq!(d.palette(), Label("Face palette"));
        let flags = d.flags();
        assert!(flags.two_dimensional && flags.frequency);
        assert!(!flags.supports_1d(), "two-dimensional only");
        assert_eq!(d.default("si"), Some(0));

        let d = Descriptor::new("Running Dual@!,Wave width;L,!,R;!");
        assert_eq!(
            [d.color(0), d.color(1), d.color(2)],
            [Label("L"), Shown, Label("R")]
        );
    }

    #[test]
    fn empty_flags_and_single_led_volume_effects() {
        let d = Descriptor::new("Twinkle@!,!;!,!;!;;m12=0");
        assert_eq!(d.flags(), Flags::default());
        assert_eq!(d.default("m12"), Some(0));

        let flags = Descriptor::new("Juggles@!,# of balls;!,!;!;01v;m12=0,si=0").flags();
        assert!(flags.single_led && flags.one_dimensional && flags.volume);
        assert!(!flags.frequency);
    }

    #[test]
    fn integers_are_read_like_atoi() {
        assert_eq!(parse_int("128"), 128);
        assert_eq!(parse_int("-3"), -3);
        assert_eq!(parse_int("12abc"), 12);
        assert_eq!(parse_int(""), 0);
        assert_eq!(parse_int("x"), 0);
    }
}
