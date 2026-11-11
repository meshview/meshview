// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The ITS Irregular Terrain Model (ITM), better known as the Longley-Rice
//! model, in area prediction mode.
//!
//! ITM predicts the median and the variability of the basic transmission loss
//! of a radio path between 20 MHz and 20 GHz over irregular terrain. Area mode
//! is the variant that needs no terrain profile: the terrain is described by a
//! single irregularity figure, Δh, and the horizons are estimated from the
//! antenna heights and how carefully the sites were chosen. That makes it the
//! mode for coverage estimates, where only a distance is known.
//!
//! This is a translation of the area-mode code of NTIA's reference
//! implementation, ITM version 1.4 (<https://github.com/NTIA/itm>), and it
//! gives the same results as that C++ to within floating point noise; see the
//! crate's README for how that was checked. Point-to-point mode is not
//! included.
//!
//! NTIA's two area-mode entry points are here as [`area_tls`], where the
//! quantiles are given as time, location and situation percentages, and
//! [`area_cr`], where they are given as confidence and reliability.
//!
//! The [`pyitm`] module is something else: a reproduction, bugs included, of
//! the `area()` function of the Python package pyitm 0.3, for programs moving
//! off it that need their results to stay the same. Use it only for that.
//!
//! # Example
//!
//! The loss over 50 km of hilly ground at 868 MHz, between antennas 10 and 2 m
//! high, exceeded in no more than 10% of the time and locations (90%
//! reliability) with 50% confidence:
//!
//! ```
//! use longley_rice::{
//!     AreaParams, Climate, Polarization, SitingCriteria, Variability, VariabilityMode, area_cr,
//! };
//!
//! let params = AreaParams {
//!     tx_height_m: 10.0,
//!     rx_height_m: 2.0,
//!     tx_siting: SitingCriteria::Random,
//!     rx_siting: SitingCriteria::Random,
//!     distance_km: 50.0,
//!     delta_h_m: 90.0,
//!     climate: Climate::ContinentalTemperate,
//!     n_0: 301.0,
//!     frequency_mhz: 868.0,
//!     polarization: Polarization::Vertical,
//!     epsilon: 15.0,
//!     sigma: 0.005,
//!     variability: Variability::new(VariabilityMode::Mobile),
//! };
//! let loss = area_cr(&params, 50.0, 90.0)?;
//! assert!(loss.loss_db > loss.intermediate.a_fs_db);
//! assert!(loss.warnings.is_empty());
//! # Ok::<(), longley_rice::Error>(())
//! ```

mod complex;
mod error;
mod itm;
mod params;
pub mod pyitm;
mod warnings;

pub use error::Error;
pub use params::{AreaParams, Climate, Polarization, SitingCriteria, Variability, VariabilityMode};
pub use warnings::Warnings;

/// The outcome of a prediction.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loss {
    /// The basic transmission loss, in dB, for the quantiles asked.
    pub loss_db: f64,
    /// What should make the result be used with care, if anything.
    pub warnings: Warnings,
    /// Values computed on the way, as NTIA's `IntermediateValues` has them.
    pub intermediate: IntermediateValues,
}

/// The values NTIA's `_Ex` entry points hand back besides the loss.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntermediateValues {
    /// Horizon elevation angles of the transmitter and receiver, in radians.
    pub theta_hzn: [f64; 2],
    /// Horizon distances of the transmitter and receiver, in metres.
    pub d_hzn_m: [f64; 2],
    /// Effective heights of the transmitter and receiver, in metres.
    pub h_e_m: [f64; 2],
    /// Surface refractivity, in N-units (N_0 itself in area mode).
    pub n_s: f64,
    /// The terrain irregularity parameter, in metres.
    pub delta_h_m: f64,
    /// The reference attenuation, the median loss relative to free space before
    /// the variability, in dB.
    pub a_ref_db: f64,
    /// The free space basic transmission loss, in dB.
    pub a_fs_db: f64,
    /// The path distance, in kilometres.
    pub d_km: f64,
    /// Which propagation mode the reference attenuation comes from.
    pub mode: PropagationMode,
}

/// The region of the path the reference attenuation was computed for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropagationMode {
    /// Within the smooth earth line-of-sight distance.
    LineOfSight = 1,
    /// Beyond the horizons, where diffraction dominates.
    Diffraction = 2,
    /// Far beyond the horizons, where tropospheric scatter dominates.
    Troposcatter = 3,
}

/// The loss with the variability given as time, location and situation
/// percentages, each strictly between 0 and 100: NTIA's `ITM_AREA_TLS_Ex`.
///
/// Which of the three count depends on the mode of variability in `params`:
/// single message mode uses only the situation, accidental mode time and
/// situation, mobile mode time (as time and location together) and situation,
/// and broadcast mode all three. The ones that do not count are still checked.
pub fn area_tls(
    params: &AreaParams,
    time: f64,
    location: f64,
    situation: f64,
) -> Result<Loss, Error> {
    itm::area_tls(params, time, location, situation)
}

/// The loss with the variability given as confidence and reliability
/// percentages, each strictly between 0 and 100: NTIA's `ITM_AREA_CR_Ex`.
///
/// This is [`area_tls`] with the reliability as the time, 50 as the location
/// and the confidence as the situation, and it is meant for the mobile and
/// accidental modes of variability, where those are what the two quantiles
/// mean.
pub fn area_cr(params: &AreaParams, confidence: f64, reliability: f64) -> Result<Loss, Error> {
    // convert TLS error codes for time and situation into CR error codes for confidence and reliability
    itm::area_tls(params, reliability, 50.0, confidence).map_err(|error| match error {
        Error::InvalidTime => Error::InvalidReliability,
        Error::InvalidSituation => Error::InvalidConfidence,
        other => other,
    })
}
