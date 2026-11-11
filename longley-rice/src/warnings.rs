// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! What NTIA flags about a prediction that was made but should be used with
//! care.

use std::fmt;
use std::ops::{BitAnd, BitOr, BitOrAssign};

/// A set of warning flags, with the same bits as NTIA's `Warnings.h` so that
/// [`Warnings::bits`] can be compared with what the reference returns.
///
/// A warning does not stop the prediction: the loss is still computed, but
/// some input or intermediate value is near or past the range the model was
/// built for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Warnings(u32);

impl Warnings {
    /// The transmitter height is outside 1 to 1000 m.
    pub const TX_TERMINAL_HEIGHT: Self = Self(0x0001);
    /// The receiver height is outside 1 to 1000 m.
    pub const RX_TERMINAL_HEIGHT: Self = Self(0x0002);
    /// The frequency is outside 40 MHz to 10 GHz.
    pub const FREQUENCY: Self = Self(0x0004);
    /// The path is longer than 1000 km.
    pub const PATH_DISTANCE_TOO_BIG_1: Self = Self(0x0008);
    /// The path is longer than 2000 km.
    pub const PATH_DISTANCE_TOO_BIG_2: Self = Self(0x0010);
    /// The path is shorter than the difference of the effective heights
    /// divided by 0.2 (a steep path).
    pub const PATH_DISTANCE_TOO_SMALL_1: Self = Self(0x0020);
    /// The path is shorter than 1 km.
    pub const PATH_DISTANCE_TOO_SMALL_2: Self = Self(0x0040);
    /// The transmitter's horizon angle is beyond 0.2 rad, where the small angle
    /// approximations start to fail.
    pub const TX_HORIZON_ANGLE: Self = Self(0x0080);
    /// The receiver's horizon angle is beyond 0.2 rad.
    pub const RX_HORIZON_ANGLE: Self = Self(0x0100);
    /// The transmitter's horizon is closer than a tenth of its smooth earth
    /// horizon.
    pub const TX_HORIZON_DISTANCE_1: Self = Self(0x0200);
    /// The receiver's horizon is closer than a tenth of its smooth earth
    /// horizon.
    pub const RX_HORIZON_DISTANCE_1: Self = Self(0x0400);
    /// The transmitter's horizon is further than three times its smooth earth
    /// horizon.
    pub const TX_HORIZON_DISTANCE_2: Self = Self(0x0800);
    /// The receiver's horizon is further than three times its smooth earth
    /// horizon.
    pub const RX_HORIZON_DISTANCE_2: Self = Self(0x1000);
    /// A time, location or situation quantile is far in the tail of its
    /// distribution (beyond 3.1 standard deviations).
    pub const EXTREME_VARIABILITIES: Self = Self(0x2000);
    /// The surface refractivity is below 250 N-units.
    pub const SURFACE_REFRACTIVITY: Self = Self(0x4000);

    const NAMES: [(Self, &'static str); 15] = [
        (Self::TX_TERMINAL_HEIGHT, "TX_TERMINAL_HEIGHT"),
        (Self::RX_TERMINAL_HEIGHT, "RX_TERMINAL_HEIGHT"),
        (Self::FREQUENCY, "FREQUENCY"),
        (Self::PATH_DISTANCE_TOO_BIG_1, "PATH_DISTANCE_TOO_BIG_1"),
        (Self::PATH_DISTANCE_TOO_BIG_2, "PATH_DISTANCE_TOO_BIG_2"),
        (Self::PATH_DISTANCE_TOO_SMALL_1, "PATH_DISTANCE_TOO_SMALL_1"),
        (Self::PATH_DISTANCE_TOO_SMALL_2, "PATH_DISTANCE_TOO_SMALL_2"),
        (Self::TX_HORIZON_ANGLE, "TX_HORIZON_ANGLE"),
        (Self::RX_HORIZON_ANGLE, "RX_HORIZON_ANGLE"),
        (Self::TX_HORIZON_DISTANCE_1, "TX_HORIZON_DISTANCE_1"),
        (Self::RX_HORIZON_DISTANCE_1, "RX_HORIZON_DISTANCE_1"),
        (Self::TX_HORIZON_DISTANCE_2, "TX_HORIZON_DISTANCE_2"),
        (Self::RX_HORIZON_DISTANCE_2, "RX_HORIZON_DISTANCE_2"),
        (Self::EXTREME_VARIABILITIES, "EXTREME_VARIABILITIES"),
        (Self::SURFACE_REFRACTIVITY, "SURFACE_REFRACTIVITY"),
    ];

    /// No warnings.
    pub const fn empty() -> Self {
        Self(0)
    }

    /// The flags as NTIA's `long *warnings` holds them.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// The flags from NTIA's bits. Bits that no flag uses are dropped.
    pub const fn from_bits_truncate(bits: u32) -> Self {
        Self(bits & 0x7fff)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Whether every flag in `other` is set.
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The names of the flags that are set, as in `Warnings.h` without the
    /// `WARN__` prefix.
    pub fn names(self) -> impl Iterator<Item = &'static str> {
        Self::NAMES
            .into_iter()
            .filter(move |(flag, _)| self.contains(*flag))
            .map(|(_, name)| name)
    }
}

impl BitOr for Warnings {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for Warnings {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl BitAnd for Warnings {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

/// The names of the flags set, separated by ` | `, or `none`.
impl fmt::Display for Warnings {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("none");
        }
        for (i, name) in self.names().enumerate() {
            if i > 0 {
                f.write_str(" | ")?;
            }
            f.write_str(name)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_combine_and_print() {
        let w = Warnings::FREQUENCY | Warnings::EXTREME_VARIABILITIES;
        assert_eq!(w.bits(), 0x2004);
        assert!(w.contains(Warnings::FREQUENCY));
        assert!(!w.contains(Warnings::TX_TERMINAL_HEIGHT));
        assert_eq!(w.to_string(), "FREQUENCY | EXTREME_VARIABILITIES");
        assert_eq!(Warnings::empty().to_string(), "none");
        assert_eq!(Warnings::from_bits_truncate(0xffff_ffff).bits(), 0x7fff);
    }
}
