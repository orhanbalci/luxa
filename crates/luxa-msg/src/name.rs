//! Fixed-capacity names.

use core::fmt;

/// A short UTF-8 name held inline, with capacity `N` bytes.
///
/// A deliberately tiny type rather than a third-party string: names appear in
/// the public state, and a dependency's string type would tie every user of
/// this crate to that dependency's major version.
///
/// Longer input is truncated to fit, at a character boundary so the result is
/// always valid UTF-8. `N` may be at most 255.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Name<const N: usize> {
    /// Bytes past `len` are always zero, so derived equality and hashing only
    /// ever see the name itself.
    bytes: [u8; N],
    len: u8,
}

impl<const N: usize> Name<N> {
    /// The empty name.
    pub const EMPTY: Self = Self {
        bytes: [0; N],
        len: 0,
    };

    /// A name from `name`, truncated to at most `N` bytes.
    pub fn new(name: &str) -> Self {
        const {
            assert!(
                N <= u8::MAX as usize,
                "Name capacity is limited to 255 bytes"
            )
        };

        let mut end = name.len().min(N);
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0; N];
        bytes[..end].copy_from_slice(&name.as_bytes()[..end]);
        Self {
            bytes,
            len: end as u8,
        }
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        // Always built from a `&str` cut at a char boundary, so this cannot
        // fail; the fallback exists only to avoid `unsafe`.
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }

    /// Length in bytes.
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Whether the name is empty.
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Capacity in bytes.
    pub const fn capacity(&self) -> usize {
        N
    }
}

impl<const N: usize> Default for Name<N> {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl<const N: usize> AsRef<str> for Name<N> {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl<const N: usize> fmt::Debug for Name<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl<const N: usize> fmt::Display for Name<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_a_name_that_fits() {
        let n = Name::<16>::new("Desk left");
        assert_eq!(n.as_str(), "Desk left");
        assert_eq!(n.len(), 9);
        assert_eq!(n.capacity(), 16);
    }

    #[test]
    fn truncates_at_a_char_boundary() {
        // 'é' is two bytes; a 2-byte cut would split it.
        assert_eq!(Name::<2>::new("héllo").as_str(), "h");
        assert_eq!(Name::<3>::new("héllo").as_str(), "hé");
    }

    #[test]
    fn empty_and_default_agree() {
        assert!(Name::<8>::default().is_empty());
        assert_eq!(Name::<8>::new(""), Name::<8>::EMPTY);
    }

    #[test]
    fn equality_ignores_how_the_name_was_built() {
        assert_eq!(Name::<4>::new("abcdef"), Name::<4>::new("abcd"));
    }
}
