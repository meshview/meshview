// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The NTIA model against NTIA's own C++.
//!
//! `fixtures/ntia.csv` holds a few thousand area-mode cases run through NTIA's
//! reference ITM v1.4, compiled with g++, with the results printed to 17
//! significant digits: enough to tell every bit. The README says how it was
//! made.

use longley_rice::{
    AreaParams, Climate, Error, Polarization, PropagationMode, SitingCriteria, Variability,
    Warnings, area_cr, area_tls,
};

/// What the reference gave for a case: an error code, or a loss with its
/// warnings and propagation mode.
enum Expected {
    Error(i32),
    Loss(f64, u32, i32),
}

struct Case {
    line: usize,
    kind: char,
    h_tx: f64,
    h_rx: f64,
    tx_site: i32,
    rx_site: i32,
    d_km: f64,
    delta_h: f64,
    climate: i32,
    n_0: f64,
    f_mhz: f64,
    pol: i32,
    epsilon: f64,
    sigma: f64,
    mdvar: i32,
    p: [f64; 3],
    expected: Expected,
}

fn cases() -> Vec<Case> {
    include_str!("fixtures/ntia.csv")
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.starts_with('#') && !line.starts_with("kind"))
        .map(|(i, line)| {
            let v: Vec<&str> = line.split(',').collect();
            assert_eq!(v.len(), 21, "line {}", i + 1);
            let f = |i: usize| v[i].parse::<f64>().unwrap();
            let n = |i: usize| v[i].parse::<i32>().unwrap();
            let rtn = n(17);
            Case {
                line: i + 1,
                kind: v[0].chars().next().unwrap(),
                h_tx: f(1),
                h_rx: f(2),
                tx_site: n(3),
                rx_site: n(4),
                d_km: f(5),
                delta_h: f(6),
                climate: n(7),
                n_0: f(8),
                f_mhz: f(9),
                pol: n(10),
                epsilon: f(11),
                sigma: f(12),
                mdvar: n(13),
                p: [f(14), f(15), f(16)],
                expected: if rtn > 1 {
                    Expected::Error(rtn)
                } else {
                    Expected::Loss(f(19), v[18].parse().unwrap(), n(20))
                },
            }
        })
        .collect()
}

/// Runs a case the way NTIA's integer interface would take it: codes that do
/// not convert are the error the reference returns for them.
fn run(c: &Case) -> Result<longley_rice::Loss, Error> {
    let params = AreaParams {
        tx_height_m: c.h_tx,
        rx_height_m: c.h_rx,
        tx_siting: SitingCriteria::try_from(c.tx_site)?,
        rx_siting: SitingCriteria::try_from(c.rx_site).map_err(|_| Error::RxSitingCriteria)?,
        distance_km: c.d_km,
        delta_h_m: c.delta_h,
        climate: Climate::try_from(c.climate)?,
        n_0: c.n_0,
        frequency_mhz: c.f_mhz,
        polarization: Polarization::try_from(c.pol)?,
        epsilon: c.epsilon,
        sigma: c.sigma,
        variability: Variability::try_from(c.mdvar)?,
    };
    match c.kind {
        'T' => area_tls(&params, c.p[0], c.p[1], c.p[2]),
        _ => area_cr(&params, c.p[0], c.p[1]),
    }
}

fn mode_code(mode: PropagationMode) -> i32 {
    mode as i32
}

#[test]
fn matches_ntia_reference() {
    let cases = cases();
    assert!(cases.len() > 4000);
    let (mut max_diff, mut worst, mut identical, mut losses, mut errors) = (0.0_f64, 0, 0, 0, 0);
    for c in &cases {
        match (run(c), &c.expected) {
            (Err(error), Expected::Error(code)) => {
                assert_eq!(error.code(), *code, "line {}", c.line);
                errors += 1;
            }
            (Ok(loss), Expected::Loss(db, warnings, mode)) => {
                assert_eq!(loss.warnings.bits(), *warnings, "line {}", c.line);
                assert_eq!(mode_code(loss.intermediate.mode), *mode, "line {}", c.line);
                let diff = (loss.loss_db - db).abs();
                if diff > max_diff {
                    max_diff = diff;
                    worst = c.line;
                }
                if loss.loss_db.to_bits() == db.to_bits() {
                    identical += 1;
                }
                losses += 1;
            }
            (got, _) => panic!("line {}: got {got:?}", c.line),
        }
    }
    println!(
        "{losses} losses ({identical} bit-identical, max |diff| {max_diff:e} dB at line {worst}), \
         {errors} errors"
    );
    assert!(
        max_diff <= 1e-10,
        "max difference {max_diff:e} dB at line {worst}"
    );
}

/// NTIA's own area-mode test cases (`area.csv` in NTIA's repository), whose
/// expected losses are given to 0.1 dB.
#[test]
fn ntia_area_csv() {
    // h_tx, h_rx, delta_h, mdvar, d_km, tx_site, rx_site, epsilon, sigma, N_0, f_mhz, pol,
    // climate, time, location, situation, A_db
    const CASES: [[f64; 17]; 5] = [
        [
            10.0, 1.0, 0.0, 0.0, 16.0, 0.0, 0.0, 15.0, 0.008, 301.0, 230.0, 0.0, 5.0, 87.0, 50.0,
            50.0, 152.5,
        ],
        [
            3.0, 1.5, 10.0, 1.0, 10.0, 1.0, 0.0, 15.0, 0.008, 301.0, 450.0, 0.0, 5.0, 40.0, 28.0,
            25.0, 133.0,
        ],
        [
            15.0, 3.0, 5.0, 2.0, 100.0, 2.0, 1.0, 15.0, 0.008, 301.0, 980.0, 1.0, 4.0, 92.0, 53.0,
            97.0, 224.1,
        ],
        [
            3.0, 5.0, 20.0, 3.0, 75.0, 0.0, 1.0, 15.0, 0.008, 301.0, 3100.0, 1.0, 2.0, 50.0, 80.0,
            43.0, 205.1,
        ],
        [
            1.5, 10.0, 45.0, 0.0, 25.0, 1.0, 2.0, 15.0, 0.008, 301.0, 8900.0, 1.0, 1.0, 70.0, 99.0,
            26.0, 156.0,
        ],
    ];
    for c in CASES {
        let params = AreaParams {
            tx_height_m: c[0],
            rx_height_m: c[1],
            delta_h_m: c[2],
            variability: Variability::try_from(c[3] as i32).unwrap(),
            distance_km: c[4],
            tx_siting: SitingCriteria::try_from(c[5] as i32).unwrap(),
            rx_siting: SitingCriteria::try_from(c[6] as i32).unwrap(),
            epsilon: c[7],
            sigma: c[8],
            n_0: c[9],
            frequency_mhz: c[10],
            polarization: Polarization::try_from(c[11] as i32).unwrap(),
            climate: Climate::try_from(c[12] as i32).unwrap(),
        };
        let loss = area_tls(&params, c[13], c[14], c[15]).unwrap();
        assert!(
            (loss.loss_db - c[16]).abs() <= 0.05,
            "{} vs {}",
            loss.loss_db,
            c[16]
        );
        assert_eq!(loss.warnings, Warnings::empty());
    }
}

#[test]
fn cr_maps_errors_to_its_own_percentages() {
    let params = AreaParams {
        tx_height_m: 5.0,
        rx_height_m: 1.5,
        tx_siting: SitingCriteria::Random,
        rx_siting: SitingCriteria::Random,
        distance_km: 10.0,
        delta_h_m: 90.0,
        climate: Climate::ContinentalTemperate,
        n_0: 301.0,
        frequency_mhz: 907.0,
        polarization: Polarization::Vertical,
        epsilon: 15.0,
        sigma: 0.005,
        variability: Variability::try_from(2).unwrap(),
    };
    assert_eq!(area_cr(&params, 0.0, 50.0), Err(Error::InvalidConfidence));
    assert_eq!(
        area_cr(&params, 50.0, 100.0),
        Err(Error::InvalidReliability)
    );
    assert_eq!(
        area_tls(&params, f64::NAN, 50.0, 50.0),
        Err(Error::InvalidTime)
    );
    let nan_distance = AreaParams {
        distance_km: f64::NAN,
        ..params
    };
    assert_eq!(area_cr(&nan_distance, 50.0, 50.0), Err(Error::PathDistance));
    // CR is TLS with the location at 50%.
    assert_eq!(
        area_cr(&params, 70.0, 90.0),
        area_tls(&params, 90.0, 50.0, 70.0)
    );
}
