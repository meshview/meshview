// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The coverage a node is predicted to have, from the ITS Irregular Terrain
//! Model in area mode: the received power at points around it, every 5° and
//! every step out to a radius, as the Python meshview's `radio/coverage.py`
//! samples it.
//!
//! The model is NTIA's, through the `longley-rice` crate. The Python version
//! used pyitm, which differs from it by bugs of its own; at the reliabilities
//! the pages ask for the predicted reach comes out the same, or a step shorter.

use longley_rice::{
    AreaParams, Climate, Polarization, SitingCriteria, Variability, VariabilityMode,
};

/// The radio climate assumed: continental temperate.
const CLIMATE: Climate = Climate::ContinentalTemperate;
/// Average ground: its conductivity, in S/m, and relative permittivity.
const GROUND_SIGMA: f64 = 0.005;
const GROUND_EPSILON: f64 = 15.0;
/// Hilly terrain's irregularity, in metres.
const DELTA_H_M: f64 = 90.0;
/// Surface refractivity, in N-units.
const N_0: f64 = 301.0;
const EARTH_RADIUS_KM: f64 = 6371.0;
const BEARING_STEP_DEG: usize = 5;

/// What a prediction is asked for with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radio {
    pub lat: f64,
    pub lon: f64,
    pub freq_mhz: f64,
    pub tx_dbm: f64,
    pub tx_height_m: f64,
    pub rx_height_m: f64,
    pub radius_km: f64,
    pub step_km: f64,
    /// The fraction of time and locations the signal is to reach, 0 to 1.
    pub reliability: f64,
}

/// The power received at `distance_km`, in dBm, or `None` where the model has
/// no answer for the inputs.
fn received_dbm(radio: &Radio, distance_km: f64) -> Option<f64> {
    let params = AreaParams {
        tx_height_m: radio.tx_height_m,
        rx_height_m: radio.rx_height_m,
        tx_siting: SitingCriteria::Random,
        rx_siting: SitingCriteria::Random,
        distance_km,
        delta_h_m: DELTA_H_M,
        climate: CLIMATE,
        n_0: N_0,
        frequency_mhz: radio.freq_mhz,
        polarization: Polarization::Vertical,
        epsilon: GROUND_EPSILON,
        sigma: GROUND_SIGMA,
        variability: Variability {
            mode: VariabilityMode::Mobile,
            exclude_location: false,
            exclude_situation: false,
        },
    };
    let loss_db = longley_rice::area_cr(&params, 50.0, radio.reliability * 100.0)
        .ok()?
        .loss_db;
    Some(radio.tx_dbm - loss_db)
}

/// The point `distance_km` from (`lat`, `lon`) towards `bearing_deg`, on a
/// sphere.
pub fn destination_point(lat: f64, lon: f64, bearing_deg: f64, distance_km: f64) -> (f64, f64) {
    let lat1 = lat.to_radians();
    let lon1 = lon.to_radians();
    let bearing = bearing_deg.to_radians();
    let d = distance_km / EARTH_RADIUS_KM;

    let lat2 = (lat1.sin() * d.cos() + lat1.cos() * d.sin() * bearing.cos()).asin();
    let lon2 =
        lon1 + (bearing.sin() * d.sin() * lat1.cos()).atan2(d.cos() - lat1.sin() * lat2.sin());
    (lat2.to_degrees(), lon2.to_degrees())
}

/// The distances sampled, as the Python version adds them up, step after step
/// from the step or 1 km, whichever is further.
fn distances(radio: &Radio) -> impl Iterator<Item = f64> + '_ {
    let finite = radio.radius_km.is_finite() && radio.step_km.is_finite() && radio.step_km > 0.0;
    std::iter::successors(finite.then(|| radio.step_km.max(1.0)), move |d| {
        Some(d + radio.step_km)
    })
    .take_while(move |d| *d <= radio.radius_km)
}

/// The received power at every point sampled: by distance, then bearing.
pub fn heatmap(radio: &Radio) -> Vec<(f64, f64, f64)> {
    let mut points = Vec::new();
    for distance in distances(radio) {
        for bearing in (0..360).step_by(BEARING_STEP_DEG) {
            let (lat, lon) = destination_point(radio.lat, radio.lon, bearing as f64, distance);
            if let Some(dbm) = received_dbm(radio, distance) {
                points.push((lat, lon, dbm));
            }
        }
    }
    points
}

/// For each bearing, the furthest point sampled where the power still reaches
/// `threshold_dbm`; bearings where it reaches nowhere are left out.
pub fn perimeter(radio: &Radio, threshold_dbm: f64) -> Vec<(f64, f64)> {
    let mut perimeter = Vec::new();
    for bearing in (0..360).step_by(BEARING_STEP_DEG) {
        let mut last = None;
        for distance in distances(radio) {
            if received_dbm(radio, distance).is_some_and(|dbm| dbm >= threshold_dbm) {
                last = Some(destination_point(
                    radio.lat,
                    radio.lon,
                    bearing as f64,
                    distance,
                ));
            }
        }
        perimeter.extend(last);
    }
    perimeter
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radio() -> Radio {
        Radio {
            lat: 42.6,
            lon: -8.4,
            freq_mhz: 868.0,
            tx_dbm: 20.0,
            tx_height_m: 5.0,
            rx_height_m: 1.5,
            radius_km: 40.0,
            step_km: 0.25,
            reliability: 0.5,
        }
    }

    #[test]
    fn the_outline_has_a_point_a_bearing() {
        assert_eq!(perimeter(&radio(), -120.0).len(), 72);
    }

    #[test]
    fn distances_add_up_from_a_kilometre() {
        let mut radio = radio();
        radio.radius_km = 2.0;
        assert_eq!(
            distances(&radio).collect::<Vec<_>>(),
            [1.0, 1.25, 1.5, 1.75, 2.0]
        );
        radio.step_km = 0.0;
        assert_eq!(distances(&radio).count(), 0);
    }

    #[test]
    fn a_point_due_north_is_north() {
        let (lat, lon) = destination_point(
            0.0,
            0.0,
            0.0,
            EARTH_RADIUS_KM * std::f64::consts::PI / 180.0,
        );
        assert!((lat - 1.0).abs() < 1e-12 && lon.abs() < 1e-12);
    }
}
