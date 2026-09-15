//! Blending a segment's pixels onto what the segments beneath it drew.

use luxa_color::Crgb;
use luxa_msg::BlendMode;

/// `top` blended onto `beneath` by `mode`.
pub(crate) fn blend(mode: BlendMode, top: Crgb, beneath: Crgb) -> Crgb {
    let each = |channel: fn(u8, u8) -> u8| {
        Crgb::new(
            channel(top.r, beneath.r),
            channel(top.g, beneath.g),
            channel(top.b, beneath.b),
        )
    };
    match mode {
        BlendMode::Top => top,
        BlendMode::Bottom => beneath,
        BlendMode::Add => add(top, beneath),
        BlendMode::Stencil => {
            if top.is_black() {
                beneath
            } else {
                top
            }
        }
        BlendMode::Subtract => each(|top, beneath| beneath.saturating_sub(top)),
        BlendMode::Difference => each(u8::abs_diff),
        BlendMode::Average => {
            each(|top, beneath| ((u16::from(top) + u16::from(beneath)) / 2) as u8)
        }
        BlendMode::Multiply => each(multiply),
        BlendMode::Divide => each(|top, beneath| divide(beneath, top)),
        BlendMode::Lighten => each(u8::max),
        BlendMode::Darken => each(u8::min),
        BlendMode::Screen => each(screen),
        BlendMode::Overlay => each(|top, beneath| hard_light(beneath, top)),
        BlendMode::HardLight => each(hard_light),
        BlendMode::SoftLight => each(soft_light),
        BlendMode::Dodge => each(|top, beneath| divide(beneath, 255 - top)),
        BlendMode::Burn => each(|top, beneath| 255 - divide(255 - beneath, top)),
    }
}

/// The channels added; where one passes full, every channel is scaled by the
/// same amount so the brightest is full and the hue holds.
fn add(top: Crgb, beneath: Crgb) -> Crgb {
    let sum = [
        u16::from(top.r) + u16::from(beneath.r),
        u16::from(top.g) + u16::from(beneath.g),
        u16::from(top.b) + u16::from(beneath.b),
    ];
    let peak = sum[0].max(sum[1]).max(sum[2]);
    let [r, g, b] = if peak <= 255 {
        sum.map(|c| c as u8)
    } else {
        sum.map(|c| (u32::from(c) * 255 / u32::from(peak)) as u8)
    };
    Crgb::new(r, g, b)
}

/// `a × b`, with full as one.
fn multiply(a: u8, b: u8) -> u8 {
    (u16::from(a) * u16::from(b) / 255) as u8
}

/// The inverse of multiplying the inverses.
fn screen(a: u8, b: u8) -> u8 {
    255 - multiply(255 - a, 255 - b)
}

/// `value ÷ divisor`, with full as one: full when the divisor is no larger
/// than the value.
fn divide(value: u8, divisor: u8) -> u8 {
    if divisor > value {
        (u16::from(value) * 255 / u16::from(divisor)) as u8
    } else {
        255
    }
}

/// Twice the product where `switch` is dark, the inverse of twice the inverse
/// product where it is light.
fn hard_light(switch: u8, other: u8) -> u8 {
    if switch < 128 {
        multiply(switch, other).saturating_mul(2)
    } else {
        255 - multiply(255 - switch, 255 - other).saturating_mul(2)
    }
}

/// Pegtop's soft light: `(1 − 2·top)·beneath² + 2·top·beneath`, with full as
/// one. Always within a channel's range.
fn soft_light(top: u8, beneath: u8) -> u8 {
    let (top, beneath) = (i32::from(top), i32::from(beneath));
    let value = ((255 - 2 * top) * beneath * beneath + 510 * top * beneath) / (255 * 255);
    value.clamp(0, 255) as u8
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Crgb = Crgb::new(255, 0, 0);
    const BLUE: Crgb = Crgb::new(0, 0, 255);
    const BLACK: Crgb = Crgb::new(0, 0, 0);
    const WHITE: Crgb = Crgb::new(255, 255, 255);
    const GREY: Crgb = Crgb::new(100, 150, 200);

    #[test]
    fn whole_pixel_modes() {
        assert_eq!(blend(BlendMode::Top, RED, BLUE), RED);
        assert_eq!(blend(BlendMode::Bottom, RED, BLUE), BLUE);
        assert_eq!(blend(BlendMode::Stencil, RED, BLUE), RED);
        assert_eq!(
            blend(BlendMode::Stencil, BLACK, BLUE),
            BLUE,
            "black lets it show"
        );
    }

    #[test]
    fn add_keeps_the_hue_where_a_channel_would_pass_full() {
        assert_eq!(blend(BlendMode::Add, RED, BLUE), Crgb::new(255, 0, 255));
        assert_eq!(
            blend(BlendMode::Add, Crgb::new(200, 100, 0), Crgb::new(200, 0, 0)),
            Crgb::new(255, 63, 0)
        );
    }

    #[test]
    fn subtract_and_divide_work_on_what_is_beneath() {
        let beneath = Crgb::new(200, 200, 200);
        assert_eq!(
            blend(BlendMode::Subtract, Crgb::new(50, 250, 0), beneath),
            Crgb::new(150, 0, 200)
        );
        assert_eq!(
            blend(
                BlendMode::Divide,
                Crgb::new(255, 100, 0),
                Crgb::new(128, 200, 7)
            ),
            Crgb::new(128, 255, 255),
            "halved by full, full where the top is no larger, full over black"
        );
    }

    #[test]
    fn black_and_white_tops_that_change_nothing() {
        for beneath in [BLACK, GREY, WHITE, RED] {
            assert_eq!(blend(BlendMode::Multiply, WHITE, beneath), beneath);
            assert_eq!(blend(BlendMode::Screen, BLACK, beneath), beneath);
            assert_eq!(blend(BlendMode::Difference, BLACK, beneath), beneath);
            assert_eq!(blend(BlendMode::Lighten, BLACK, beneath), beneath);
            assert_eq!(blend(BlendMode::Darken, WHITE, beneath), beneath);
            assert_eq!(blend(BlendMode::Dodge, BLACK, beneath), beneath);
            assert_eq!(blend(BlendMode::Burn, WHITE, beneath), beneath);
        }
    }

    #[test]
    fn overlay_switches_on_what_is_beneath_and_hard_light_on_the_top() {
        let (dark, light) = (Crgb::new(50, 50, 50), Crgb::new(200, 200, 200));
        assert_eq!(blend(BlendMode::Overlay, light, dark).r, 78, "multiplied");
        assert_eq!(blend(BlendMode::HardLight, light, dark).r, 167, "screened");
    }

    #[test]
    fn soft_light_stays_in_range_and_leaves_black_beneath_black() {
        for top in 0..=255u8 {
            for beneath in 0..=255u8 {
                let _ = soft_light(top, beneath);
            }
            assert_eq!(soft_light(top, 0), 0);
        }
        assert_eq!(soft_light(0, 255), 255);
        assert_eq!(soft_light(255, 255), 255);
    }
}
