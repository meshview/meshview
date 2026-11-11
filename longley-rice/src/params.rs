// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The inputs of an area prediction.
//!
//! NTIA's interface takes every choice as an `int` and checks it at run time.
//! Here each is an enum, so an invalid one cannot be passed at all; the
//! `TryFrom<i32>` conversions take NTIA's codes for callers that have them as
//! numbers, and refuse the invalid ones with the error NTIA would return.

use crate::Error;

/// The parameters of an area prediction, everything but the percentages that
/// say which quantile of the loss is wanted.
///
/// Area mode knows nothing of the actual terrain between the terminals: it
/// stands for it with a single irregularity figure, `delta_h_m`, and estimates
/// the horizons from the antenna heights and how carefully the sites were
/// chosen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AreaParams {
    /// Structural height of the transmitter's antenna above the ground, in
    /// metres. Valid from 0.5 to 3000; outside 1 to 1000 it is flagged.
    pub tx_height_m: f64,
    /// Structural height of the receiver's antenna above the ground, in
    /// metres, with the same range as the transmitter's.
    pub rx_height_m: f64,
    /// How the transmitter's site was chosen.
    pub tx_siting: SitingCriteria,
    /// How the receiver's site was chosen.
    pub rx_siting: SitingCriteria,
    /// Distance between the terminals, in kilometres. Any positive distance is
    /// accepted; outside 1 to 1000 km it is flagged.
    pub distance_km: f64,
    /// Terrain irregularity parameter Δh, in metres: the interdecile range of
    /// the terrain heights along the path. Typical values are 0 for water or
    /// very flat land, 30 for plains, 90 for hills, 200 for mountains and 500
    /// for rugged mountains.
    pub delta_h_m: f64,
    /// The radio climate.
    pub climate: Climate,
    /// Surface refractivity N_0, in N-units, from 250 to 400. 301 is the usual
    /// value for a continental temperate climate.
    pub n_0: f64,
    /// Carrier frequency, in MHz, from 20 to 20000; outside 40 to 10000 it is
    /// flagged.
    pub frequency_mhz: f64,
    /// Polarization of both antennas.
    pub polarization: Polarization,
    /// Relative permittivity of the ground, at least 1. 15 for average ground,
    /// 80 for sea water.
    pub epsilon: f64,
    /// Conductivity of the ground, in S/m, positive. 0.005 for average ground,
    /// 5 for sea water.
    pub sigma: f64,
    /// Which kinds of variability the quantiles describe.
    pub variability: Variability,
}

/// How carefully a terminal's site was chosen, which in area mode raises its
/// effective height over the structural one on irregular terrain.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SitingCriteria {
    /// Placed without regard to the terrain.
    #[default]
    Random = 0,
    /// Placed with some care, on a local high spot.
    Careful = 1,
    /// Placed with great care, on the best spot around.
    VeryCareful = 2,
}

/// The radio climate, which sets the curves for the long-term variability.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Climate {
    Equatorial = 1,
    ContinentalSubtropical = 2,
    MaritimeSubtropical = 3,
    Desert = 4,
    ContinentalTemperate = 5,
    MaritimeTemperateOverLand = 6,
    MaritimeTemperateOverSea = 7,
}

/// Antenna polarization, assumed to be the same at both ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Polarization {
    Horizontal = 0,
    Vertical = 1,
}

/// NTIA's mode of variability ("mdvar"): how the time, location and situation
/// quantiles combine.
///
/// It is a mode plus two adjustments, which NTIA encodes as one number: the
/// mode, plus 10 when location variability is left out, plus 20 when direct
/// situation variability is left out. [`Variability::code`] and the
/// `TryFrom<i32>` conversion go between the two.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Variability {
    pub mode: VariabilityMode,
    /// Leave out location variability, as for a terminal that stays at one
    /// point whose loss is what is being estimated (NTIA's +10).
    pub exclude_location: bool,
    /// Leave out direct situation variability, as when studying interference
    /// (NTIA's +20). A small residual remains.
    pub exclude_situation: bool,
}

/// How the quantiles combine, the base of [`Variability`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VariabilityMode {
    /// Single message mode: time, location and situation are lumped together,
    /// and only the situation (confidence) quantile counts.
    SingleMessage = 0,
    /// Accidental (individual) mode: time and location together, plus
    /// situation.
    Accidental = 1,
    /// Mobile mode: time and location together as reliability, plus
    /// situation as confidence. The usual choice for coverage of a mobile
    /// service.
    Mobile = 2,
    /// Broadcast mode: time, location and situation each on their own.
    Broadcast = 3,
}

impl Variability {
    /// The mode, with both kinds of variability kept.
    pub const fn new(mode: VariabilityMode) -> Self {
        Self {
            mode,
            exclude_location: false,
            exclude_situation: false,
        }
    }

    /// NTIA's numeric mdvar.
    pub const fn code(self) -> i32 {
        self.mode as i32
            + if self.exclude_location { 10 } else { 0 }
            + if self.exclude_situation { 20 } else { 0 }
    }
}

impl From<VariabilityMode> for Variability {
    fn from(mode: VariabilityMode) -> Self {
        Self::new(mode)
    }
}

impl TryFrom<i32> for Variability {
    type Error = Error;

    /// NTIA's mdvar: 0 to 3, 10 to 13, 20 to 23 or 30 to 33.
    fn try_from(code: i32) -> Result<Self, Error> {
        if !(0..=33).contains(&code) {
            return Err(Error::Mdvar);
        }
        let mode = match code % 10 {
            0 => VariabilityMode::SingleMessage,
            1 => VariabilityMode::Accidental,
            2 => VariabilityMode::Mobile,
            3 => VariabilityMode::Broadcast,
            _ => return Err(Error::Mdvar),
        };
        Ok(Self {
            mode,
            exclude_location: code % 20 >= 10,
            exclude_situation: code >= 20,
        })
    }
}

impl TryFrom<i32> for SitingCriteria {
    type Error = Error;

    /// NTIA's code, 0 to 2. An invalid one is reported as the transmitter's,
    /// since the conversion does not know which terminal it is for.
    fn try_from(code: i32) -> Result<Self, Error> {
        match code {
            0 => Ok(Self::Random),
            1 => Ok(Self::Careful),
            2 => Ok(Self::VeryCareful),
            _ => Err(Error::TxSitingCriteria),
        }
    }
}

impl TryFrom<i32> for Climate {
    type Error = Error;

    /// NTIA's code, 1 to 7.
    fn try_from(code: i32) -> Result<Self, Error> {
        match code {
            1 => Ok(Self::Equatorial),
            2 => Ok(Self::ContinentalSubtropical),
            3 => Ok(Self::MaritimeSubtropical),
            4 => Ok(Self::Desert),
            5 => Ok(Self::ContinentalTemperate),
            6 => Ok(Self::MaritimeTemperateOverLand),
            7 => Ok(Self::MaritimeTemperateOverSea),
            _ => Err(Error::InvalidRadioClimate),
        }
    }
}

impl TryFrom<i32> for Polarization {
    type Error = Error;

    /// NTIA's code, 0 for horizontal and 1 for vertical.
    fn try_from(code: i32) -> Result<Self, Error> {
        match code {
            0 => Ok(Self::Horizontal),
            1 => Ok(Self::Vertical),
            _ => Err(Error::Polarization),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mdvar_codes_round_trip() {
        for code in (0..=3).chain(10..=13).chain(20..=23).chain(30..=33) {
            assert_eq!(Variability::try_from(code).unwrap().code(), code);
        }
        for code in [-1, 4, 9, 14, 19, 24, 29, 34, 100] {
            assert_eq!(Variability::try_from(code), Err(Error::Mdvar));
        }
    }
}
