// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! Why a prediction could not be made.

use std::fmt;

/// An input the model refuses, one variant per area-mode error code in NTIA's
/// `Errors.h`, with the same meaning and the same number behind [`Error::code`].
///
/// Some cannot come out of [`area_tls`](crate::area_tls) or
/// [`area_cr`](crate::area_cr), because the typed parameters make the input
/// impossible to begin with: those are what the `TryFrom<i32>` conversions of
/// the parameter enums return for a code that is not valid. The internally
/// computed refractivity and effective earth checks cannot fail either once
/// the refractivity has passed its own range check, since area mode computes
/// them from it alone; they are kept so that every NTIA code has a variant.
///
/// Where NTIA only compares with the ends of a range, a value that is not a
/// number at all (NaN, and infinity for the inputs that have no upper bound)
/// is refused here with the same error, where NTIA's checks would let it
/// through to produce a meaningless loss.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// The transmitter's structural height is outside 0.5 to 3000 m.
    TxTerminalHeight,
    /// The receiver's structural height is outside 0.5 to 3000 m.
    RxTerminalHeight,
    /// Not one of the seven radio climates.
    InvalidRadioClimate,
    /// The time percentage is not strictly between 0 and 100.
    InvalidTime,
    /// The location percentage is not strictly between 0 and 100.
    InvalidLocation,
    /// The situation percentage is not strictly between 0 and 100.
    InvalidSituation,
    /// The confidence percentage is not strictly between 0 and 100.
    InvalidConfidence,
    /// The reliability percentage is not strictly between 0 and 100.
    InvalidReliability,
    /// The surface refractivity N_0 is outside 250 to 400 N-units.
    Refractivity,
    /// The frequency is outside 20 MHz to 20 GHz.
    Frequency,
    /// Not horizontal or vertical polarization.
    Polarization,
    /// The relative permittivity is below 1.
    Epsilon,
    /// The conductivity is not positive.
    Sigma,
    /// The ground impedance has an imaginary part at least as large as its real
    /// part, which the model's approximations do not cover.
    GroundImpedance,
    /// Not a valid mode of variability.
    Mdvar,
    /// The effective earth radius computed from the refractivity is outside
    /// 4000 to 13333 km.
    EffectiveEarth,
    /// The path distance is not positive.
    PathDistance,
    /// The terrain irregularity parameter is negative.
    DeltaH,
    /// Not a valid siting criterion for the transmitter.
    TxSitingCriteria,
    /// Not a valid siting criterion for the receiver.
    RxSitingCriteria,
    /// The surface refractivity computed internally is below 150 N-units.
    SurfaceRefractivitySmall,
    /// The surface refractivity computed internally is above 400 N-units.
    SurfaceRefractivityLarge,
}

impl Error {
    /// NTIA's numeric code for the error, as `ITM_AREA_TLS` and `ITM_AREA_CR`
    /// return it.
    pub fn code(self) -> i32 {
        match self {
            Self::TxTerminalHeight => 1000,
            Self::RxTerminalHeight => 1001,
            Self::InvalidRadioClimate => 1002,
            Self::InvalidTime => 1003,
            Self::InvalidLocation => 1004,
            Self::InvalidSituation => 1005,
            Self::InvalidConfidence => 1006,
            Self::InvalidReliability => 1007,
            Self::Refractivity => 1008,
            Self::Frequency => 1009,
            Self::Polarization => 1010,
            Self::Epsilon => 1011,
            Self::Sigma => 1012,
            Self::GroundImpedance => 1013,
            Self::Mdvar => 1014,
            Self::EffectiveEarth => 1016,
            Self::PathDistance => 1017,
            Self::DeltaH => 1018,
            Self::TxSitingCriteria => 1019,
            Self::RxSitingCriteria => 1020,
            Self::SurfaceRefractivitySmall => 1021,
            Self::SurfaceRefractivityLarge => 1022,
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TxTerminalHeight => "transmitter height is out of range (0.5 to 3000 m)",
            Self::RxTerminalHeight => "receiver height is out of range (0.5 to 3000 m)",
            Self::InvalidRadioClimate => "invalid radio climate",
            Self::InvalidTime => "time percentage is out of range (0 to 100, exclusive)",
            Self::InvalidLocation => "location percentage is out of range (0 to 100, exclusive)",
            Self::InvalidSituation => "situation percentage is out of range (0 to 100, exclusive)",
            Self::InvalidConfidence => {
                "confidence percentage is out of range (0 to 100, exclusive)"
            }
            Self::InvalidReliability => {
                "reliability percentage is out of range (0 to 100, exclusive)"
            }
            Self::Refractivity => "surface refractivity is out of range (250 to 400 N-units)",
            Self::Frequency => "frequency is out of range (20 to 20000 MHz)",
            Self::Polarization => "invalid polarization",
            Self::Epsilon => "relative permittivity is out of range (at least 1)",
            Self::Sigma => "conductivity is out of range (must be positive)",
            Self::GroundImpedance => {
                "the imaginary part of the ground impedance is larger than its real part"
            }
            Self::Mdvar => "invalid mode of variability",
            Self::EffectiveEarth => "the computed effective earth radius is out of range",
            Self::PathDistance => "path distance is out of range (must be positive)",
            Self::DeltaH => "terrain irregularity parameter is out of range (must not be negative)",
            Self::TxSitingCriteria => "invalid transmitter siting criteria",
            Self::RxSitingCriteria => "invalid receiver siting criteria",
            Self::SurfaceRefractivitySmall => "the computed surface refractivity is too small",
            Self::SurfaceRefractivityLarge => "the computed surface refractivity is too large",
        })
    }
}

impl std::error::Error for Error {}
