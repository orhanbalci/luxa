//! The value grammar: string values for numeric fields.
//!
//! Besides plain numbers, a numeric field accepts a short expression:
//!
//! | Value | Meaning |
//! |---|---|
//! | `"~"`, `"~-"` | one step up or down, wrapping at the ends |
//! | `"~10"`, `"~-10"` | add or subtract, clamped |
//! | `"w~10"` | add, jumping to the other end when already at the limit |
//! | `"~0"` | present, but no change |
//! | `"r"` | a random value |
//! | `"1~5~"`, `"1~5~-"` | step up or down within 1–5, wrapping |
//! | `"4~8r"` | a random value from 4 up to (not including) 8 |
//!
//! The rules are ported from the reference implementation character by
//! character, quirks included — `"1~5"` sets 1, and an unparseable string sets
//! 0 — because clients rely on its exact behaviour.

use luxa_msg::{Bounds, Direction, U8Op};

/// Where an expression's relative operations are confined.
#[derive(Clone, Copy)]
enum Scope {
    /// The field's own range.
    Field,
    /// A range expression whose range has not been read yet.
    Unbounded,
    /// An explicit range.
    Within(Bounds),
}

impl Scope {
    const fn bounds(self) -> Option<Bounds> {
        match self {
            Self::Field => None,
            // A range expression that never names its range works in 0..0.
            Self::Unbounded => Some(Bounds::new(0, 0)),
            Self::Within(bounds) => Some(bounds),
        }
    }
}

/// The operation a string value for a `u8` field describes, or `None` when
/// the value is ignored altogether (empty, or longer than 12 bytes).
pub(crate) fn parse_u8_expr(text: &str) -> Option<U8Op> {
    if text.is_empty() || text.len() > 12 {
        return None;
    }
    // A random pick or a second `~` in a longer string means a range
    // expression, which ignores the field's own range.
    let ranged = text.len() > 3 && (text.contains('r') || text.find('~') != text.rfind('~'));
    let scope = if ranged {
        Scope::Unbounded
    } else {
        Scope::Field
    };
    Some(parse_number(text.as_bytes(), scope))
}

fn parse_number(s: &[u8], scope: Scope) -> U8Op {
    let Some(&first) = s.first() else {
        return U8Op::Keep;
    };
    if first == b'r' {
        return U8Op::Random {
            bounds: scope.bounds(),
        };
    }
    let (wrap, s) = if first == b'w' && s.len() > 1 {
        (true, &s[1..])
    } else {
        (false, s)
    };

    if s[0] == b'~' {
        let delta = atoi(&s[1..]);
        if delta == 0 {
            return match s.get(1) {
                Some(b'0') => U8Op::Keep,
                Some(b'-') => U8Op::Cycle {
                    direction: Direction::Down,
                    bounds: scope.bounds(),
                },
                _ => U8Op::Cycle {
                    direction: Direction::Up,
                    bounds: scope.bounds(),
                },
            };
        }
        return U8Op::Add {
            delta: delta.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
            wrap,
            bounds: scope.bounds(),
        };
    }

    if matches!(scope, Scope::Unbounded) {
        // "A~B<rest>": read the range, then parse the rest within it.
        let low = atoi(s) as u8;
        if let Some(tilde) = s.iter().position(|&c| c == b'~') {
            let rest = &s[tilde + 1..];
            let high = atoi(rest) as u8;
            if high > 0 {
                let skip = 1 + rest[1..].iter().take_while(|c| c.is_ascii_digit()).count();
                return parse_number(&rest[skip..], Scope::Within(Bounds::new(low, high)));
            }
        }
    }

    U8Op::Set(atoi(s) as u8)
}

/// A leading integer, read as C's `atoi` reads it: optional whitespace and sign,
/// then digits up to the first non-digit; `0` when there are none.
pub(crate) fn atoi(s: &[u8]) -> i32 {
    let s = match s.iter().position(|c| !c.is_ascii_whitespace()) {
        Some(start) => &s[start..],
        None => return 0,
    };
    let (negative, digits) = match s.first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    let magnitude = digits
        .iter()
        .take_while(|c| c.is_ascii_digit())
        .fold(0i32, |acc, d| {
            acc.saturating_mul(10).saturating_add(i32::from(d - b'0'))
        });
    if negative { -magnitude } else { magnitude }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Option<U8Op> {
        parse_u8_expr(s)
    }

    const fn up(bounds: Option<Bounds>) -> U8Op {
        U8Op::Cycle {
            direction: Direction::Up,
            bounds,
        }
    }

    const fn down(bounds: Option<Bounds>) -> U8Op {
        U8Op::Cycle {
            direction: Direction::Down,
            bounds,
        }
    }

    const fn add(delta: i16, wrap: bool, bounds: Option<Bounds>) -> U8Op {
        U8Op::Add {
            delta,
            wrap,
            bounds,
        }
    }

    #[test]
    fn empty_and_overlong_values_are_ignored() {
        assert_eq!(parse(""), None);
        assert_eq!(parse("1234567890123"), None, "13 bytes");
        // 12 bytes is still read; the number overflows `atoi`, which saturates
        // at `INT_MAX` like the C library's `strtol`, and truncates to 255.
        assert_eq!(parse("123456789012"), Some(U8Op::Set(255)));
    }

    #[test]
    fn steps() {
        assert_eq!(parse("~"), Some(up(None)));
        assert_eq!(parse("~-"), Some(down(None)));
        assert_eq!(parse("~0"), Some(U8Op::Keep));
        assert_eq!(
            parse("~-0"),
            Some(down(None)),
            "`-0` reads as zero, then as a decrement"
        );
    }

    #[test]
    fn adds() {
        assert_eq!(parse("~10"), Some(add(10, false, None)));
        assert_eq!(parse("~-10"), Some(add(-10, false, None)));
        assert_eq!(parse("w~5"), Some(add(5, true, None)));
        assert_eq!(parse("w~"), Some(up(None)));
        assert_eq!(
            parse("~-99999"),
            Some(add(i16::MIN, false, None)),
            "saturates"
        );
    }

    #[test]
    fn plain_numbers_and_garbage() {
        assert_eq!(parse("42"), Some(U8Op::Set(42)));
        assert_eq!(parse("300"), Some(U8Op::Set(44)), "truncated to a byte");
        assert_eq!(parse("abc"), Some(U8Op::Set(0)));
        assert_eq!(parse("1~5"), Some(U8Op::Set(1)), "too short to be a range");
        assert_eq!(parse("w5"), Some(U8Op::Set(5)));
    }

    #[test]
    fn random() {
        assert_eq!(parse("r"), Some(U8Op::Random { bounds: None }));
        assert_eq!(
            parse("r5"),
            Some(U8Op::Random { bounds: None }),
            "too short to be a range"
        );
        assert_eq!(
            parse("4~8r"),
            Some(U8Op::Random {
                bounds: Some(Bounds::new(4, 8))
            })
        );
        assert_eq!(
            parse("rrrr"),
            Some(U8Op::Random {
                bounds: Some(Bounds::new(0, 0))
            }),
            "a range expression without a range"
        );
    }

    #[test]
    fn ranges() {
        let b = Some(Bounds::new(1, 5));
        assert_eq!(parse("1~5~"), Some(up(b)));
        assert_eq!(parse("1~5~-"), Some(down(b)));
        assert_eq!(parse("1~5~3"), Some(add(3, false, b)));
        assert_eq!(parse("10~20~"), Some(up(Some(Bounds::new(10, 20)))));
        assert_eq!(
            parse("1~0~"),
            Some(U8Op::Set(1)),
            "an empty range is not a range"
        );
        assert_eq!(
            parse("1~5~~"),
            Some(up(b)),
            "the rest after the range is parsed within it"
        );
    }

    #[test]
    fn atoi_matches_c() {
        assert_eq!(atoi(b"  -12x"), -12);
        assert_eq!(atoi(b"+7"), 7);
        assert_eq!(atoi(b""), 0);
        assert_eq!(atoi(b"x1"), 0);
        assert_eq!(atoi(b"99999999999"), i32::MAX);
    }
}
