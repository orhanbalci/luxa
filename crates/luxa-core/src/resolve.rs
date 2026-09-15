//! Resolving relative values against the current state.
//!
//! Commands *describe* a change — "one step up", "toggle", "something random
//! between 4 and 8". Only here, with the current value and the field's range in
//! hand, does that become a number.

use luxa_msg::{BoolOp, Direction, U8Op};

/// The engine's source of randomness: a small seedable xorshift generator.
///
/// Owned by the engine and seeded once by the runtime, so every random choice
/// is reproducible from the seed in tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rng(u32);

impl Rng {
    pub(crate) const fn new(seed: u32) -> Self {
        // Xorshift never leaves zero, so zero is not a usable seed.
        Self(if seed == 0 { 0x9E37_79B9 } else { seed })
    }

    pub(crate) fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// A value in `min..end`, end exclusive; `min` when the range is empty.
    pub(crate) fn below(&mut self, min: u8, end: u16) -> u8 {
        let min16 = u16::from(min);
        if end <= min16 {
            return min;
        }
        let span = u32::from(end - min16);
        (min16 + (self.next_u32() % span) as u16) as u8
    }
}

/// The range a `u8` field accepts when an operation names none.
///
/// `max` is a `u16` because some fields use a count as their upper bound — an
/// effect field's is the number of effect ids, which can be 256.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Range {
    pub(crate) min: u8,
    pub(crate) max: u16,
}

impl Range {
    /// `0..=255`, the range of most fields.
    pub(crate) const FULL: Self = Self { min: 0, max: 255 };

    /// `0..=max`.
    pub(crate) const fn up_to(max: u16) -> Self {
        Self { min: 0, max }
    }
}

/// The value `op` produces from `current`.
pub(crate) fn resolve_u8(op: U8Op, current: u8, field: Range, rng: &mut Rng) -> u8 {
    let bounds = match op {
        U8Op::Cycle { bounds, .. } | U8Op::Add { bounds, .. } | U8Op::Random { bounds } => bounds,
        U8Op::Set(value) => return value,
        U8Op::Keep => return current,
    };
    let (min, max) = match bounds {
        Some(b) => (i32::from(b.min), i32::from(b.max)),
        None => (i32::from(field.min), i32::from(field.max)),
    };
    let cur = i32::from(current);

    let value = match op {
        U8Op::Cycle {
            direction: Direction::Up,
            ..
        } => {
            if cur + 1 > max {
                min
            } else {
                (cur + 1).max(min)
            }
        }
        U8Op::Cycle {
            direction: Direction::Down,
            ..
        } => {
            if cur - 1 < min {
                max
            } else {
                (cur - 1).min(max)
            }
        }
        U8Op::Add { delta, wrap, .. } => {
            let delta = i32::from(delta);
            if wrap && cur == max && delta > 0 {
                min
            } else if wrap && cur == min && delta < 0 {
                max
            } else {
                // Upper then lower, not `clamp`: inverted bounds from a
                // request must not panic, and settle on the lower bound.
                let mut out = cur + delta;
                if out > max {
                    out = max;
                }
                if out < min {
                    out = min;
                }
                out
            }
        }
        U8Op::Random { .. } => {
            // An upper bound of zero means "the whole byte range".
            let end = if max == 0 { 255 } else { max };
            return rng.below(min.clamp(0, 255) as u8, end.clamp(0, 256) as u16);
        }
        U8Op::Set(_) | U8Op::Keep => unreachable!("returned above"),
    };
    value.clamp(0, 255) as u8
}

/// The value `op` produces from `current`.
pub(crate) const fn resolve_bool(op: BoolOp, current: bool) -> bool {
    match op {
        BoolOp::Set(value) => value,
        BoolOp::Toggle => !current,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use luxa_msg::Bounds;

    fn run(op: U8Op, current: u8) -> u8 {
        resolve_u8(op, current, Range::FULL, &mut Rng::new(1))
    }

    const UP: U8Op = U8Op::Cycle {
        direction: Direction::Up,
        bounds: None,
    };
    const DOWN: U8Op = U8Op::Cycle {
        direction: Direction::Down,
        bounds: None,
    };

    const fn add(delta: i16, wrap: bool) -> U8Op {
        U8Op::Add {
            delta,
            wrap,
            bounds: None,
        }
    }

    #[test]
    fn set_and_keep() {
        assert_eq!(run(U8Op::Set(9), 200), 9);
        assert_eq!(run(U8Op::Keep, 200), 200);
    }

    #[test]
    fn step_up_and_down_wrap_at_the_ends() {
        // "~" and "~-"
        assert_eq!(run(UP, 5), 6);
        assert_eq!(run(UP, 255), 0);
        assert_eq!(run(DOWN, 5), 4);
        assert_eq!(run(DOWN, 0), 255);
    }

    #[test]
    fn add_clamps() {
        // "~10" and "~-10"
        assert_eq!(run(add(10, false), 250), 255);
        assert_eq!(run(add(-10, false), 3), 0);
        assert_eq!(run(add(10, false), 100), 110);
    }

    #[test]
    fn add_with_wrap_jumps_only_from_the_limit() {
        // "w~10" and "w~-10"
        assert_eq!(
            run(add(10, true), 255),
            0,
            "at the top, jumps to the bottom"
        );
        assert_eq!(run(add(10, true), 250), 255, "short of the top, clamps");
        assert_eq!(
            run(add(-10, true), 0),
            255,
            "at the bottom, jumps to the top"
        );
        assert_eq!(run(add(-10, true), 5), 0, "short of the bottom, clamps");
    }

    #[test]
    fn cycling_within_bounds() {
        // "1~5~" and "1~5~-"
        let b = Some(Bounds::new(1, 5));
        let up = U8Op::Cycle {
            direction: Direction::Up,
            bounds: b,
        };
        let down = U8Op::Cycle {
            direction: Direction::Down,
            bounds: b,
        };
        assert_eq!(run(up, 3), 4);
        assert_eq!(run(up, 5), 1, "wraps past the top of the bounds");
        assert_eq!(
            run(up, 0),
            1,
            "a value below the bounds is raised into them"
        );
        assert_eq!(
            run(up, 9),
            1,
            "a value above the bounds wraps to the bottom"
        );
        assert_eq!(run(down, 1), 5);
        assert_eq!(
            run(down, 9),
            5,
            "a value above the bounds is lowered into them"
        );
    }

    #[test]
    fn random_stays_in_range_with_an_exclusive_top() {
        let mut rng = Rng::new(7);
        let bounded = U8Op::Random {
            bounds: Some(Bounds::new(4, 8)),
        };
        for _ in 0..500 {
            let v = resolve_u8(bounded, 0, Range::FULL, &mut rng);
            assert!((4..8).contains(&v), "{v}");
        }
        // An effect range of 0..count never yields `count` itself.
        let fx = U8Op::Random { bounds: None };
        for _ in 0..500 {
            assert!(resolve_u8(fx, 0, Range::up_to(3), &mut rng) < 3);
        }
    }

    #[test]
    fn random_is_reproducible_from_the_seed() {
        let op = U8Op::Random { bounds: None };
        let draw = |seed| {
            let mut rng = Rng::new(seed);
            [0; 8].map(|_| resolve_u8(op, 0, Range::FULL, &mut rng))
        };
        assert_eq!(draw(42), draw(42));
        assert_ne!(draw(42), draw(43));
    }

    #[test]
    fn inverted_bounds_do_not_panic() {
        let b = Some(Bounds::new(9, 2));
        for op in [
            U8Op::Add {
                delta: 3,
                wrap: false,
                bounds: b,
            },
            U8Op::Cycle {
                direction: Direction::Up,
                bounds: b,
            },
            U8Op::Random { bounds: b },
        ] {
            let _ = run(op, 5);
        }
    }

    #[test]
    fn zero_is_never_a_stuck_seed() {
        let mut rng = Rng::new(0);
        assert_ne!(rng.next_u32(), rng.next_u32());
    }

    #[test]
    fn bools() {
        assert!(resolve_bool(BoolOp::Set(true), false));
        assert!(!resolve_bool(BoolOp::Toggle, true));
        assert!(resolve_bool(BoolOp::Toggle, false));
    }
}
