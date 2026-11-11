// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The model itself: the area-mode path of NTIA's ITM v1.4, function by
//! function.
//!
//! The functions keep NTIA's names (in snake case), their structure and their
//! order of operations, down to the order of the factors in a product, so that
//! the results agree with the reference to the last bit and not only to a few
//! ulps. Every `pow` stays a call to libm's `pow`, squares included, as in the
//! compiled C++. NTIA's comments citing the equations are kept as well; the
//! references are:
//!
//! - [Algorithm]: G.A. Hufford, "The ITS Irregular Terrain Model, version
//!   1.2.2: the Algorithm", 1995.
//! - [ERL 79-ITS 67]: Longley and Rice, "Prediction of Tropospheric Radio
//!   Transmission Loss Over Irregular Terrain: A Computer Method", 1968.
//! - [TN101]: Rice, Longley, Norton and Barsis, "Transmission Loss Predictions
//!   for Tropospheric Communication Circuits", NBS Technical Note 101, 1967.
//! - [Vogler 1964]: L.E. Vogler, "Calculation of Groundwave Attenuation in the
//!   Far Diffraction Region", 1964.

use crate::complex::Complex;
use crate::{
    AreaParams, Error, IntermediateValues, Loss, Polarization, PropagationMode, SitingCriteria,
    Variability, VariabilityMode, Warnings,
};
use std::f64::consts::PI;
use std::hint::black_box;

const THIRD: f64 = 1.0 / 3.0;
/// Radius of the 4/3 earth, for [Vogler 1964].
const A_0_METER: f64 = 6370e3;
/// Radius used for the effective distance in the variability.
const A_9000_METER: f64 = 9000e3;
const SQRT2: f64 = std::f64::consts::SQRT_2;

/// `pow(x, 2)`, which GCC compiles to a call to libm's `pow` as for any other
/// exponent. LLVM would turn a plain `powf(2.0)` into `x * x`, which is not
/// always the same bits, so the exponent is hidden from it.
fn pow2(x: f64) -> f64 {
    x.powf(black_box(2.0))
}

// NTIA's MIN, MAX and DIM macros. Not f64::min and f64::max, which treat NaN
// differently.
fn min(x: f64, y: f64) -> f64 {
    if x < y { x } else { y }
}

fn max(x: f64, y: f64) -> f64 {
    if x > y { x } else { y }
}

fn dim(x: f64, y: f64) -> f64 {
    if x > y { x - y } else { 0.0 }
}

/// `ITM_AREA_TLS_Ex`, with the percentages already checked to be what TLS
/// takes.
pub(crate) fn area_tls(
    params: &AreaParams,
    time: f64,
    location: f64,
    situation: f64,
) -> Result<Loss, Error> {
    let p = params;
    let mut warnings = Warnings::empty();

    // initial input validation check - some validation occurs later in calculations
    validate_inputs(p, time, location, situation, &mut warnings)?;

    // additional area mode parameter validation checks
    if !(p.distance_km > 0.0 && p.distance_km.is_finite()) {
        return Err(Error::PathDistance);
    }
    if !(p.delta_h_m >= 0.0 && p.delta_h_m.is_finite()) {
        return Err(Error::DeltaH);
    }

    let site_criteria = [p.tx_siting, p.rx_siting];
    let h_meter = [p.tx_height_m, p.rx_height_m];
    let f_mhz = p.frequency_mhz;
    let delta_h = p.delta_h_m;

    let (z_g, gamma_e, n_s) =
        initialize_point_to_point(f_mhz, p.n_0, p.polarization, p.epsilon, p.sigma);
    let (h_e, d_hzn, theta_hzn) = initialize_area(site_criteria, gamma_e, delta_h, h_meter);

    let d_meter = p.distance_km * 1000.0;
    let (a_ref, mode) = longley_rice(
        theta_hzn,
        f_mhz,
        z_g,
        d_hzn,
        h_e,
        gamma_e,
        n_s,
        delta_h,
        h_meter,
        d_meter,
        &mut warnings,
    )?;

    let a_fs = free_space_loss(d_meter, f_mhz);
    let loss_db = a_fs
        + variability(
            time,
            location,
            situation,
            h_e,
            delta_h,
            f_mhz,
            d_meter,
            a_ref,
            p.climate as usize,
            p.variability,
            &mut warnings,
        );

    Ok(Loss {
        loss_db,
        warnings,
        intermediate: IntermediateValues {
            theta_hzn,
            d_hzn_m: d_hzn,
            h_e_m: h_e,
            n_s,
            delta_h_m: delta_h,
            a_ref_db: a_ref,
            a_fs_db: a_fs,
            d_km: p.distance_km,
            mode,
        },
    })
}

/// `ValidateInputs`, for what the enums do not already guarantee. The checks
/// are written so that NaN fails them.
fn validate_inputs(
    p: &AreaParams,
    time: f64,
    location: f64,
    situation: f64,
    warnings: &mut Warnings,
) -> Result<(), Error> {
    let (h_tx, h_rx) = (p.tx_height_m, p.rx_height_m);
    if !(1.0..=1000.0).contains(&h_tx) {
        *warnings |= Warnings::TX_TERMINAL_HEIGHT;
    }
    if !(0.5..=3000.0).contains(&h_tx) {
        return Err(Error::TxTerminalHeight);
    }
    if !(1.0..=1000.0).contains(&h_rx) {
        *warnings |= Warnings::RX_TERMINAL_HEIGHT;
    }
    if !(0.5..=3000.0).contains(&h_rx) {
        return Err(Error::RxTerminalHeight);
    }
    if !(250.0..=400.0).contains(&p.n_0) {
        return Err(Error::Refractivity);
    }
    let f = p.frequency_mhz;
    if !(40.0..=10000.0).contains(&f) {
        *warnings |= Warnings::FREQUENCY;
    }
    if !(20.0..=20000.0).contains(&f) {
        return Err(Error::Frequency);
    }
    if !(p.epsilon >= 1.0 && p.epsilon.is_finite()) {
        return Err(Error::Epsilon);
    }
    if !(p.sigma > 0.0 && p.sigma.is_finite()) {
        return Err(Error::Sigma);
    }
    let open_percentage = |v: f64| v > 0.0 && v < 100.0;
    if !open_percentage(situation) {
        return Err(Error::InvalidSituation);
    }
    if !open_percentage(time) {
        return Err(Error::InvalidTime);
    }
    if !open_percentage(location) {
        return Err(Error::InvalidLocation);
    }
    Ok(())
}

/// `InitializePointToPoint` with the path at sea level, as area mode calls
/// it: returns the complex ground impedance, the curvature of the effective
/// earth and the surface refractivity.
fn initialize_point_to_point(
    f_mhz: f64,
    n_0: f64,
    pol: Polarization,
    epsilon: f64,
    sigma: f64,
) -> (Complex, f64, f64) {
    // gamma_a is the curvature of the actual earth, ~1 / 6370 km
    const GAMMA_A: f64 = 157e-9;

    // With h_sys = 0 there is no scaling of the refractivity [TN101, Eq 4.3].
    let n_s = n_0;

    // gamma_e is the curvature of the effective earth
    let gamma_e = GAMMA_A * (1.0 - 0.04665 * (n_s / 179.3).exp()); // [TN101, Eq 4.4], reworked

    // complex relative permittivity
    let ep_r = Complex::new(epsilon, 18000.0 * sigma / f_mhz);
    // ground impedance (horizontal polarization)
    let mut z_g = Complex::new(ep_r.re - 1.0, ep_r.im).sqrt_glibc();
    if pol == Polarization::Vertical {
        // adjust for vertical polarization
        z_g = z_g.div(ep_r);
    }
    (z_g, gamma_e, n_s)
}

/// `InitializeArea`: the effective heights, horizon distances and horizon
/// angles of the two terminals, estimated from their heights and siting.
fn initialize_area(
    site_criteria: [SitingCriteria; 2],
    gamma_e: f64,
    delta_h_meter: f64,
    h_meter: [f64; 2],
) -> ([f64; 2], [f64; 2], [f64; 2]) {
    let mut h_e = [0.0; 2];
    let mut d_hzn = [0.0; 2];
    let mut theta_hzn = [0.0; 2];
    for i in 0..2 {
        if site_criteria[i] == SitingCriteria::Random {
            h_e[i] = h_meter[i];
        } else {
            let mut b = if site_criteria[i] == SitingCriteria::Careful {
                4.0
            } else {
                9.0
            };
            if h_meter[i] < 5.0 {
                b *= (0.1 * PI * h_meter[i]).sin();
            }
            // [Algorithm, Eqn 3.2]
            h_e[i] = h_meter[i]
                + (1.0 + b) * (-min(20.0, 2.0 * h_meter[i] / max(1e-3, delta_h_meter))).exp();
        }

        let d_ls = (2.0 * h_e[i] / gamma_e).sqrt();

        // [Algorithm, Eqn 3.3]
        const H_3_METER: f64 = 5.0;
        d_hzn[i] = d_ls * (-0.07 * (delta_h_meter / max(h_e[i], H_3_METER)).sqrt()).exp();

        // [Algorithm, Eqn 3.4]
        theta_hzn[i] = (0.65 * delta_h_meter * (d_ls / d_hzn[i] - 1.0) - 2.0 * h_e[i]) / d_ls;
    }
    (h_e, d_hzn, theta_hzn)
}

/// `LongleyRice`: the reference attenuation, the loss relative to free space
/// before any variability, and the mode of propagation it comes from.
#[allow(clippy::too_many_arguments)]
fn longley_rice(
    theta_hzn: [f64; 2],
    f_mhz: f64,
    z_g: Complex,
    d_hzn: [f64; 2],
    h_e: [f64; 2],
    gamma_e: f64,
    n_s: f64,
    delta_h: f64,
    h_meter: [f64; 2],
    d_meter: f64,
    warnings: &mut Warnings,
) -> Result<(f64, PropagationMode), Error> {
    // effective earth radius
    let a_e = 1.0 / gamma_e;

    // Terrestrial smooth earth horizon distance approximation
    let d_hzn_s = [(2.0 * h_e[0] * a_e).sqrt(), (2.0 * h_e[1] * a_e).sqrt()];

    // Maximum line-of-sight distance for smooth earth
    let d_sml = d_hzn_s[0] + d_hzn_s[1];

    // Maximum line-of-sight distance for actual path
    let d_ml = d_hzn[0] + d_hzn[1];

    // Angular distance of line-of-sight region
    let theta_los = -max(theta_hzn[0] + theta_hzn[1], -d_ml / a_e);

    // Check validity of small angle approximation
    if theta_hzn[0].abs() > 200e-3 {
        *warnings |= Warnings::TX_HORIZON_ANGLE;
    }
    if theta_hzn[1].abs() > 200e-3 {
        *warnings |= Warnings::RX_HORIZON_ANGLE;
    }

    // Checks that the actual horizon distance can't be less than 1/10 of the smooth earth horizon distance
    if d_hzn[0] < 0.1 * d_hzn_s[0] {
        *warnings |= Warnings::TX_HORIZON_DISTANCE_1;
    }
    if d_hzn[1] < 0.1 * d_hzn_s[1] {
        *warnings |= Warnings::RX_HORIZON_DISTANCE_1;
    }

    // Checks that the actual horizon distance can't be greater than 3 times the smooth earth horizon distance
    if d_hzn[0] > 3.0 * d_hzn_s[0] {
        *warnings |= Warnings::TX_HORIZON_DISTANCE_2;
    }
    if d_hzn[1] > 3.0 * d_hzn_s[1] {
        *warnings |= Warnings::RX_HORIZON_DISTANCE_2;
    }

    // Check the surface refractivity
    if n_s < 150.0 {
        return Err(Error::SurfaceRefractivitySmall);
    }
    if n_s > 400.0 {
        return Err(Error::SurfaceRefractivityLarge);
    }
    if n_s < 250.0 {
        // 150 <= N_s < 250
        *warnings |= Warnings::SURFACE_REFRACTIVITY;
    }

    // Check effective earth size
    if !(4000000.0..=13333333.0).contains(&a_e) {
        return Err(Error::EffectiveEarth);
    }

    // Check ground impedance
    if z_g.re <= z_g.im.abs() {
        return Err(Error::GroundImpedance);
    }

    // Select two distances far in the diffraction region
    let d_3 = max(d_sml, d_ml + 5.0 * (pow2(a_e) / f_mhz).powf(1.0 / 3.0));
    let d_4 = d_3 + 10.0 * (pow2(a_e) / f_mhz).powf(1.0 / 3.0);

    // Compute the diffraction loss at the two distances
    let diffraction = |d: f64| {
        diffraction_loss(
            d, d_hzn, h_e, z_g, a_e, delta_h, h_meter, theta_los, d_sml, f_mhz,
        )
    };
    let a_3 = diffraction(d_3);
    let a_4 = diffraction(d_4);

    // Compute the slope and intercept of the diffraction line
    let m_d = (a_4 - a_3) / (d_4 - d_3);
    let a_d0 = a_3 - m_d * d_3;

    let d_min = (h_e[0] - h_e[1]).abs() / 200e-3;
    if d_meter < d_min {
        *warnings |= Warnings::PATH_DISTANCE_TOO_SMALL_1;
    }
    if d_meter < 1e3 {
        *warnings |= Warnings::PATH_DISTANCE_TOO_SMALL_2;
    }
    if d_meter > 1000e3 {
        *warnings |= Warnings::PATH_DISTANCE_TOO_BIG_1;
    }
    if d_meter > 2000e3 {
        *warnings |= Warnings::PATH_DISTANCE_TOO_BIG_2;
    }

    let (a_ref, mode);
    // if the path distance is less than the maximum smooth earth line of sight distance...
    if d_meter < d_sml {
        let los = |d: f64| line_of_sight_loss(d, h_e, z_g, delta_h, m_d, a_d0, d_sml, f_mhz);

        // Compute the diffraction loss at the maximum smooth earth line of sight distance
        let a_sml = d_sml * m_d + a_d0;

        // [ERL 79-ITS 67, Eqn 3.16a], in meters instead of km and with MIN() part below
        let mut d_0 = 0.04 * f_mhz * h_e[0] * h_e[1];

        let d_1 = if a_d0 >= 0.0 {
            d_0 = min(d_0, 0.5 * d_ml); // other part of [ERL 79-ITS 67, Eqn 3.16a]
            d_0 + 0.25 * (d_ml - d_0) // [ERL 79-ITS 67, Eqn 3.16d]
        } else {
            max(-a_d0 / m_d, 0.25 * d_ml)
        };

        let a_1 = los(d_1);

        let mut flag = false;

        let mut k_hat_1 = 0.0;
        let mut k_hat_2 = 0.0;

        if d_0 < d_1 {
            let a_0 = los(d_0);
            let q = (d_sml / d_0).ln();

            // [ERL 79-ITS 67, Eqn 3.20]
            k_hat_2 = max(
                0.0,
                ((d_sml - d_0) * (a_1 - a_0) - (d_1 - d_0) * (a_sml - a_0))
                    / ((d_sml - d_0) * (d_1 / d_0).ln() - (d_1 - d_0) * q),
            );

            flag = a_d0 > 0.0 || k_hat_2 > 0.0;

            if flag {
                // [ERL 79-ITS 67, Eqn 3.21]
                k_hat_1 = (a_sml - a_0 - k_hat_2 * q) / (d_sml - d_0);

                if k_hat_1 < 0.0 {
                    k_hat_1 = 0.0;
                    k_hat_2 = dim(a_sml, a_0) / q;

                    if k_hat_2 == 0.0 {
                        k_hat_1 = m_d;
                    }
                }
            }
        }

        if !flag {
            k_hat_1 = dim(a_sml, a_1) / (d_sml - d_1);
            k_hat_2 = 0.0;

            if k_hat_1 == 0.0 {
                k_hat_1 = m_d;
            }
        }

        let a_o = a_sml - k_hat_1 * d_sml - k_hat_2 * d_sml.ln();

        // [ERL 79-ITS 67, Eqn 3.19]
        a_ref = a_o + k_hat_1 * d_meter + k_hat_2 * d_meter.ln();
        mode = PropagationMode::LineOfSight;
    } else {
        // this is a trans-horizon path
        // select to points far into the troposcatter region
        let d_5 = d_ml + 200e3;
        let d_6 = d_ml + 400e3;

        // Compute the troposcatter loss at the two distances
        let mut h0 = -1.0;
        let a_6 = troposcatter_loss(
            d_6, theta_hzn, d_hzn, h_e, a_e, n_s, f_mhz, theta_los, &mut h0,
        );
        let a_5 = troposcatter_loss(
            d_5, theta_hzn, d_hzn, h_e, a_e, n_s, f_mhz, theta_los, &mut h0,
        );

        let (m_s, a_s0, d_x);

        // if we got a reasonable prediction value back...
        if a_5 < 1000.0 {
            // Compute the slope of the troposcatter line
            m_s = (a_6 - a_5) / 200e3;

            // Find the diffraction-troposcatter transition distance
            d_x = max(
                max(
                    d_sml,
                    d_ml + 1.088 * (pow2(a_e) / f_mhz).powf(1.0 / 3.0) * f_mhz.ln(),
                ),
                (a_5 - a_d0 - m_s * d_5) / (m_d - m_s),
            );

            // Compute the intercept of the troposcatter line
            a_s0 = (m_d - m_s) * d_x + a_d0;
        } else {
            // troposcatter gives no real results - so use diffraction line parameters for tropo line
            m_s = m_d;
            a_s0 = a_d0;
            d_x = 10e6;
        }

        // Determine if its diffraction or troposcatter and compute the loss
        if d_meter > d_x {
            a_ref = m_s * d_meter + a_s0;
            mode = PropagationMode::Troposcatter;
        } else {
            a_ref = m_d * d_meter + a_d0;
            mode = PropagationMode::Diffraction;
        }
    }

    // Don't allow a negative loss
    Ok((max(a_ref, 0.0), mode))
}

/// `LineOfSightLoss`: the two-ray loss, blended with the extended diffraction
/// line.
#[allow(clippy::too_many_arguments)]
fn line_of_sight_loss(
    d_meter: f64,
    h_e: [f64; 2],
    z_g: Complex,
    delta_h: f64,
    m_d: f64,
    a_d0: f64,
    d_sml: f64,
    f_mhz: f64,
) -> f64 {
    let delta_h_d = terrain_roughness(d_meter, delta_h);

    let sigma_h_d = sigma_h_function(delta_h_d);

    // wavenumber, k
    let wn = f_mhz / 47.7;

    // [Algorithm, Eqn 4.46]
    let sum = h_e[0] + h_e[1];
    let sin_psi = sum / (pow2(d_meter) + pow2(sum)).sqrt();

    // [Algorithm, Eqn 4.47]
    let mut r_e = Complex::real(sin_psi)
        .sub(z_g)
        .div(Complex::real(sin_psi).add(z_g))
        .scale((-min(10.0, wn * sigma_h_d * sin_psi)).exp());

    // q = Magnitude of R_e', [Algorithm, Eqn 4.48]
    let q = pow2(r_e.re) + pow2(r_e.im);
    if q < 0.25 || q < sin_psi {
        r_e = r_e.scale((sin_psi / q).sqrt());
    }

    // phase difference between rays, [Algorithm, Eqn 4.49]
    let mut delta_phi = wn * 2.0 * h_e[0] * h_e[1] / d_meter;

    // [Algorithm, Eqn 4.50]
    if delta_phi > PI / 2.0 {
        delta_phi = PI - (PI / 2.0) * (PI / 2.0) / delta_phi;
    }

    // Two-ray attenuation
    let rr = Complex::new(delta_phi.cos(), -delta_phi.sin()).add(r_e);
    let a_t = -10.0 * (pow2(rr.re) + pow2(rr.im)).log10();

    // Extended diffraction attenuation
    let a_d = m_d * d_meter + a_d0;

    // weighting factor
    let w = 1.0 / (1.0 + f_mhz * delta_h / max(10e3, d_sml));

    w * a_t + (1.0 - w) * a_d
}

/// `DiffractionLoss`: knife-edge and smooth earth diffraction weighted
/// together, plus the clutter factor.
#[allow(clippy::too_many_arguments)]
fn diffraction_loss(
    d_meter: f64,
    d_hzn: [f64; 2],
    h_e: [f64; 2],
    z_g: Complex,
    a_e: f64,
    delta_h: f64,
    h_meter: [f64; 2],
    theta_los: f64,
    d_sml: f64,
    f_mhz: f64,
) -> f64 {
    let a_k = knife_edge_diffraction(d_meter, f_mhz, a_e, theta_los, d_hzn);
    let a_se = smooth_earth_diffraction(d_meter, f_mhz, a_e, theta_los, d_hzn, h_e, z_g);

    //////////////////
    // Terrain clutter

    // Terrain roughness term, using d_sML__meter, per [ERL 79-ITS 67, page 3-13]
    let delta_h_dsml = terrain_roughness(d_sml, delta_h);
    let sigma_h_d = sigma_h_function(delta_h_dsml);

    // Clutter factor
    // [ERL 79-ITS 67, Eqn 3.38c]
    let a_fo = min(
        15.0,
        5.0 * (1.0 + 1e-5 * h_meter[0] * h_meter[1] * f_mhz * sigma_h_d).log10(),
    );

    //////////////////////////////
    // Combined diffraction losses

    // compute the weighting factor in the following calculations
    let delta_h_d = terrain_roughness(d_meter, delta_h);

    // Area mode: NTIA adds 10 to q here only in point-to-point mode
    // (for low antennas with known path parameters, C ~= 10 [ERL 79-ITS 67, page 3-8]).
    let q = h_meter[0] * h_meter[1];
    let qk = h_e[0] * h_e[1] - q;

    let term1 = (1.0 + qk / q).sqrt(); // square root term in [ERL 79-ITS 67, Eqn 3.23]

    let d_ml = d_hzn[0] + d_hzn[1]; // Maximum line-of-sight distance for actual path
    let q = (term1 + (-theta_los * a_e + d_ml) / d_meter) * min(delta_h_d * f_mhz / 47.7, 6283.2);

    // weighting factor [ERL 17-ITS 67, Eqn 3.23]
    let w = 25.1 / (25.1 + q.sqrt());

    w * a_se + (1.0 - w) * a_k + a_fo
}

/// `FresnelIntegral`: the approximate ideal knife-edge loss A(v, 0), from v².
fn fresnel_integral(v2: f64) -> f64 {
    // Note: v2 is v^2, so 5.76 is actually comparing v to 2.4
    if v2 < 5.76 {
        6.02 + 9.11 * v2.sqrt() - 1.27 * v2 // [TN101v2, Eqn III.24b] and [ERL 79-ITS 67, Eqn 3.27a & 3.27b]
    } else {
        12.953 + 10.0 * v2.log10() // [TN101v2, Eqn III.24c] and [ERL 79-ITS 67, Eqn 3.27a & 3.27b]
    }
}

/// `KnifeEdgeDiffraction`: the loss over the two horizons as knife edges.
fn knife_edge_diffraction(
    d_meter: f64,
    f_mhz: f64,
    a_e: f64,
    theta_los: f64,
    d_hzn: [f64; 2],
) -> f64 {
    let d_ml = d_hzn[0] + d_hzn[1]; // Maximum line-of-sight distance for actual path
    let theta_nlos = d_meter / a_e - theta_los; // Angular distance of diffraction region [Algorithm, Eqn 4.12]
    let d_nlos = d_meter - d_ml; // Diffraction distance, in meters

    // 1 / (4 pi) = 0.0795775
    // [TN101, Eqn I.7]
    let v = |d_hzn: f64| {
        0.0795775 * (f_mhz / 47.7) * pow2(theta_nlos) * d_hzn * d_nlos / (d_nlos + d_hzn)
    };

    fresnel_integral(v(d_hzn[0])) + fresnel_integral(v(d_hzn[1])) // [TN101, Eqn I.1]
}

/// `SmoothEarthDiffraction`: the Vogler three radii method.
fn smooth_earth_diffraction(
    d_meter: f64,
    f_mhz: f64,
    a_e: f64,
    theta_los: f64,
    d_hzn: [f64; 2],
    h_e: [f64; 2],
    z_g: Complex,
) -> f64 {
    let theta_nlos = d_meter / a_e - theta_los; // [Algorithm, Eqn 4.12]
    let d_ml = d_hzn[0] + d_hzn[1]; // Maximum line-of-sight distance for actual path

    // compute 3 radii
    let a_meter = [
        (d_meter - d_ml) / (d_meter / a_e - theta_los),
        // Compute the radius of the effective earth for terminal j using [Vogler 1964, Eqn 3] re-arranged
        0.5 * pow2(d_hzn[0]) / h_e[0],
        0.5 * pow2(d_hzn[1]) / h_e[1],
    ];
    let d_km = [
        (a_meter[0] * theta_nlos) / 1000.0, // angular distance of the "diffraction path"
        d_hzn[0] / 1000.0,
        d_hzn[1] / 1000.0,
    ];

    let mut c_0 = [0.0; 3];
    let mut k = [0.0; 3];
    let mut b_0 = [0.0; 3];
    for i in 0..3 {
        // C_0 is the ratio of the 4/3 earth to effective earth (technically Vogler 1964 ratio is 4/3 to effective earth k value), all raised to the (1/3) power.
        // C_0 = (4 / 3k) ^ (1 / 3) [Vogler 1964, Eqn 2]
        c_0[i] = ((4.0 / 3.0) * A_0_METER / a_meter[i]).powf(THIRD);

        // [Vogler 1964, Eqn 6a / 7a]
        k[i] = 0.017778 * c_0[i] * f_mhz.powf(-THIRD) / z_g.abs();

        // compute B_0 for each radius
        // [Vogler 1964, Fig 4]
        b_0[i] = 1.607 - k[i];
    }

    // compute x__km for each radius [Vogler 1964, Eqn 2]
    let x1 = b_0[1] * pow2(c_0[1]) * f_mhz.powf(THIRD) * d_km[1];
    let x2 = b_0[2] * pow2(c_0[2]) * f_mhz.powf(THIRD) * d_km[2];
    let x0 = b_0[0] * pow2(c_0[0]) * f_mhz.powf(THIRD) * d_km[0] + x1 + x2;

    // compute height gain functions
    let f_x = [height_function(x1, k[1]), height_function(x2, k[2])];

    // compute distance function
    let g_x = 0.05751 * x0 - 10.0 * x0.log10(); // [TN101, Eqn 8.4] & [Volger 1964, Eqn 13]

    g_x - f_x[0] - f_x[1] - 20.0 // [Algorithm, Eqn 4.20] & [Volger 1964]
}

/// `HeightFunction`: the height gain F(x, K) for smooth earth diffraction.
fn height_function(x_km: f64, k: f64) -> f64 {
    if x_km < 200.0 {
        let w = -k.ln();
        if k < 1e-5 || x_km * w.powf(3.0) > 5495.0 {
            let mut result = -117.0;
            if x_km > 1.0 {
                result += 17.372 * x_km.ln();
            }
            result
        } else {
            2.5e-5 * pow2(x_km) / k - 8.686 * w - 15.0
        }
    } else {
        let mut result = 0.05751 * x_km - 4.343 * x_km.ln();
        if x_km < 2000.0 {
            let w = 0.0134 * x_km * (-0.005 * x_km).exp();
            result = (1.0 - w) * result + w * (17.372 * x_km.ln() - 117.0);
        }
        result
    }
}

/// `H0Curve`: the curve fit for H_01(r, j).
fn h0_curve(j: usize, r: f64) -> f64 {
    // values from [Algorithm, 6.13]
    const A: [f64; 5] = [25.0, 80.0, 177.0, 395.0, 705.0];
    const B: [f64; 5] = [24.0, 45.0, 68.0, 80.0, 105.0];
    // related to TN101v2, Eqn III.49, but from [Algorithm, 6.13]
    10.0 * (1.0 + A[j] * (1.0 / r).powf(4.0) + B[j] * pow2(1.0 / r)).log10()
}

/// `H0Function`: the troposcatter frequency gain function H_0() [TN101v1, Ch 9.2].
fn h0_function(r: f64, eta_s: f64) -> f64 {
    let eta_s = min(max(eta_s, 1.0), 5.0); // range 1 <= eta_s <= 5
    let i = eta_s as usize; // integer part of eta_s
    let q = eta_s - i as f64; // decimal part of eta_s
    let mut result = h0_curve(i - 1, r);
    if q != 0.0 {
        // interpolate with next curve, if needed
        result = (1.0 - q) * result + q * h0_curve(i, r);
    }
    result
}

/// `FFunction`: the attenuation function F(θd).
fn f_function(td: f64) -> f64 {
    // constants from [Algorithm, 6.9]
    const A: [f64; 3] = [133.4, 104.6, 71.8];
    const B: [f64; 3] = [0.332e-3, 0.212e-3, 0.157e-3];
    const C: [f64; 3] = [-10.0, -2.5, 5.0];

    // select the set of values to use
    let i = if td <= 10e3 {
        0 // <= 10 km
    } else if td <= 70e3 {
        1 // 10 km to 70 km
    } else {
        2 // > 70 km
    };

    A[i] + B[i] * td + C[i] * td.log10() // [Algorithm, 6.9]
}

/// `TroposcatterLoss`. `h0` carries H_0 from one call to the next, as NTIA's
/// pointer does: the first call at d_6 stores it, the second at d_5 may reuse
/// it.
#[allow(clippy::too_many_arguments)]
fn troposcatter_loss(
    d_meter: f64,
    theta_hzn: [f64; 2],
    d_hzn: [f64; 2],
    h_e: [f64; 2],
    a_e: f64,
    n_s: f64,
    f_mhz: f64,
    theta_los: f64,
    h0: &mut f64,
) -> f64 {
    let mut h_0;

    // wavenumber, k
    let wn = f_mhz / 47.7;

    if *h0 > 15.0 {
        // short-circuit calculations if already greater than 15 dB
        h_0 = *h0;
    } else {
        let mut ad = d_hzn[0] - d_hzn[1];
        let mut rr = h_e[1] / h_e[0];

        if ad < 0.0 {
            // ensure correct frame of reference
            ad = -ad;
            rr = 1.0 / rr;
        }

        let theta = theta_hzn[0] + theta_hzn[1] + d_meter / a_e; // angular distance, in radians

        // [TN101, Eqn 9.4a]
        let r_1 = 2.0 * wn * theta * h_e[0];
        let r_2 = 2.0 * wn * theta * h_e[1];

        if r_1 < 0.2 && r_2 < 0.2 {
            // "If both r_1 and r_2 are less than 0.2 the function A_scat is not defined (or is infinite)" [Algorithm, page 11]
            return 1001.0;
        }

        let mut s = (d_meter - ad) / (d_meter + ad); // asymmetry parameter

        // "In all of this, we truncate the values of s and q at 0.1 and 10" [Algorithm, page 16]
        let q = min(max(0.1, rr / s), 10.0); // TN101, Eqn 9.5
        s = max(0.1, s); // TN101, Eqn 9.5

        let h_0_meter = (d_meter - ad) * (d_meter + ad) * theta * 0.25 / d_meter; // height of cross-over, [Algorithm, 4.66] [TN101v1, 9.3b]

        const Z_0_METER: f64 = 1.7556e3; // Scale height, [Algorithm, 4.67]
        const Z_1_METER: f64 = 8.0e3; // [Algorithm, 4.67]

        // Scattering efficiency factor, eta_s [TN101 Eqn 9.3a]
        let eta_s = (h_0_meter / Z_0_METER)
            * (1.0
                + (0.031 - n_s * 2.32e-3 + pow2(n_s) * 5.67e-6)
                    * (-min(1.7, h_0_meter / Z_1_METER).powf(6.0)).exp());

        let h_00 = (h0_function(r_1, eta_s) + h0_function(r_2, eta_s)) / 2.0; // First term in TN101v1, Eqn 9.5
        let delta_h_0 = min(
            h_00,
            6.0 * (0.6 - max(eta_s, 1.0).log10()) * s.log10() * q.log10(),
        );

        h_0 = h_00 + delta_h_0; // TN101, Eqn 9.5
        h_0 = max(h_0, 0.0); // "If Delta_H_0 would make H_0 negative, use H_0 = 0" [TN101v1, p9.4]

        if eta_s < 1.0 {
            // if <=1, interpolate with the special case of eta_s = 0
            let t = (1.0 + SQRT2 / r_1) * (1.0 + SQRT2 / r_2);
            h_0 = eta_s * h_0
                + (1.0 - eta_s)
                    * 10.0
                    * (pow2(t) * (r_1 + r_2) / (r_1 + r_2 + 2.0 * SQRT2)).log10();
        }

        // "If, at d_5, calculations show that H_0 will exceed 15 dB, they are replaced by the value it has at d_6" [Algorithm, page 12]
        if h_0 > 15.0 && *h0 >= 0.0 {
            h_0 = *h0;
        }
    }

    *h0 = h_0;

    let th = d_meter / a_e - theta_los;

    const D_0_METER: f64 = 40e3; // [Algorithm, 6.8]
    const H_METER: f64 = 47.7; // [Algorithm, 4.63]

    // [Algorithm, 4.63]
    f_function(th * d_meter) + 10.0 * (wn * H_METER * th.powf(4.0)).log10()
        - 0.1 * (n_s - 301.0) * (-th * d_meter / D_0_METER).exp()
        + h_0
}

/// `Curve`: the curve fit of TN101v2 Eqn III.69 & III.70.
fn curve(c1: f64, c2: f64, x1: f64, x2: f64, x3: f64, d_e_meter: f64) -> f64 {
    let t = (d_e_meter - x2) / x3;
    let u = d_e_meter / x1;
    (c1 + c2 / (1.0 + pow2(t))) * pow2(u) / (1.0 + pow2(u))
}

/// `Variability`: the loss for the quantiles asked, relative to free space.
#[allow(clippy::too_many_arguments)]
fn variability(
    time: f64,
    location: f64,
    situation: f64,
    h_e: [f64; 2],
    delta_h: f64,
    f_mhz: f64,
    d_meter: f64,
    a_ref_db: f64,
    climate: usize,
    mdvar: Variability,
    warnings: &mut Warnings,
) -> f64 {
    // Asymptotic values from TN101, Fig 10.13
    // -> approximate to TN101v2 Eqn III.69 & III.70
    // -> to describe the curves for each climate
    const ALL_YEAR: [[f64; 7]; 5] = [
        [-9.67, -0.62, 1.26, -9.21, -0.62, -0.39, 3.15],
        [12.7, 9.19, 15.5, 9.05, 9.19, 2.86, 857.9],
        [
            144.9e3, 228.9e3, 262.6e3, 84.1e3, 228.9e3, 141.7e3, 2222.0e3,
        ],
        [
            190.3e3, 205.2e3, 185.2e3, 101.1e3, 205.2e3, 315.9e3, 164.8e3,
        ],
        [133.8e3, 143.6e3, 99.8e3, 98.6e3, 143.6e3, 167.4e3, 116.3e3],
    ];

    const BSM1: [f64; 7] = [2.13, 2.66, 6.11, 1.98, 2.68, 6.86, 8.51];
    const BSM2: [f64; 7] = [159.5, 7.67, 6.65, 13.11, 7.16, 10.38, 169.8];
    const XSM1: [f64; 7] = [762.2e3, 100.4e3, 138.2e3, 139.1e3, 93.7e3, 187.8e3, 609.8e3];
    const XSM2: [f64; 7] = [
        123.6e3, 172.5e3, 242.2e3, 132.7e3, 186.8e3, 169.6e3, 119.9e3,
    ];
    const XSM3: [f64; 7] = [94.5e3, 136.4e3, 178.6e3, 193.5e3, 133.5e3, 108.9e3, 106.6e3];

    const BSP1: [f64; 7] = [2.11, 6.87, 10.08, 3.68, 4.75, 8.58, 8.43];
    const BSP2: [f64; 7] = [102.3, 15.53, 9.60, 159.3, 8.12, 13.97, 8.19];
    const XSP1: [f64; 7] = [636.9e3, 138.7e3, 165.3e3, 464.4e3, 93.2e3, 216.0e3, 136.2e3];
    const XSP2: [f64; 7] = [134.8e3, 143.7e3, 225.7e3, 93.1e3, 135.9e3, 152.0e3, 188.5e3];
    const XSP3: [f64; 7] = [95.6e3, 98.6e3, 129.7e3, 94.2e3, 113.4e3, 122.7e3, 122.9e3];

    const C_D: [f64; 7] = [1.224, 0.801, 1.380, 1.000, 1.224, 1.518, 1.518]; // [Algorithm, Table 5.1], C_d
    const Z_D: [f64; 7] = [1.282, 2.161, 1.282, 20.0, 1.282, 1.282, 1.282]; // [Algorithm, Table 5.1], z_d

    const BFM1: [f64; 7] = [1.0, 1.0, 1.0, 1.0, 0.92, 1.0, 1.0];
    const BFM2: [f64; 7] = [0.0, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0];
    const BFM3: [f64; 7] = [0.0, 0.0, 0.0, 0.0, 1.77, 0.0, 0.0];
    const BFP1: [f64; 7] = [1.0, 0.93, 1.0, 0.93, 0.93, 1.0, 1.0];
    const BFP2: [f64; 7] = [0.0, 0.31, 0.0, 0.19, 0.31, 0.0, 0.0];
    const BFP3: [f64; 7] = [0.0, 2.00, 0.0, 1.79, 2.00, 0.0, 0.0];

    let mut z_t = inverse_ccdf(time / 100.0);
    let mut z_l = inverse_ccdf(location / 100.0);
    let z_s = inverse_ccdf(situation / 100.0);

    let c = climate - 1; // 0-based indexes

    let wn = f_mhz / 47.7;

    // compute the effective distance
    let d_ex = (2.0 * A_9000_METER * h_e[0]).sqrt()
        + (2.0 * A_9000_METER * h_e[1]).sqrt()
        + (575.7e12 / wn).powf(THIRD); // [Algorithm, Eqn 5.3]

    let d_e = if d_meter < d_ex {
        130e3 * d_meter / d_ex
    } else {
        130e3 + d_meter - d_ex
    };

    //////////////////////////////////
    // situation variability calcs

    // if mdvar >= 20, then "Direct situation variability is to be eliminated as it should when
    //                       considering interference problems.  Note that there may still be a
    //                       small residual situation variability" [Hufford, 1982]
    let sigma_s = if mdvar.exclude_situation {
        0.0
    } else {
        const D_METER: f64 = 100e3; // Scale distance, D = 100 km
        5.0 + 3.0 * (-d_e / D_METER).exp() // [Algorithm, Eqn 5.10]
    };

    //
    //////////////////////////////////

    let v_med = curve(
        ALL_YEAR[0][c],
        ALL_YEAR[1][c],
        ALL_YEAR[2][c],
        ALL_YEAR[3][c],
        ALL_YEAR[4][c],
        d_e,
    );

    match mdvar.mode {
        VariabilityMode::SingleMessage => {
            z_t = z_s;
            z_l = z_s;
        }
        VariabilityMode::Accidental => z_l = z_s,
        VariabilityMode::Mobile => z_l = z_t,
        // else using Broadcast Mode (no additional operations)
        VariabilityMode::Broadcast => {}
    }

    if z_t.abs() > 3.10 || z_l.abs() > 3.10 || z_s.abs() > 3.10 {
        *warnings |= Warnings::EXTREME_VARIABILITIES;
    }

    //////////////////////////////////
    // location variability calcs

    let sigma_l = if mdvar.exclude_location {
        0.0
    } else {
        let delta_h_d = terrain_roughness(d_meter, delta_h);
        10.0 * wn * delta_h_d / (wn * delta_h_d + 13.0) // Context of [Algorithm, Eqn 5.9]
    };
    let y_l = sigma_l * z_l;

    //
    //////////////////////////////////

    //////////////////////////////////
    // time variability calcs

    let q = (0.133 * wn).ln();
    let t = BFM3[c] * q;
    let g_minus = BFM1[c] + BFM2[c] / (pow2(t) + 1.0);
    let t = BFP3[c] * q;
    let g_plus = BFP1[c] + BFP2[c] / (pow2(t) + 1.0);

    let sigma_t_minus = curve(BSM1[c], BSM2[c], XSM1[c], XSM2[c], XSM3[c], d_e) * g_minus;
    let sigma_t_plus = curve(BSP1[c], BSP2[c], XSP1[c], XSP2[c], XSP3[c], d_e) * g_plus;

    let sigma_td = C_D[c] * sigma_t_plus;
    let tgtd = (sigma_t_plus - sigma_td) * Z_D[c];

    let sigma_t = if z_t < 0.0 {
        sigma_t_minus
    } else if z_t <= Z_D[c] {
        sigma_t_plus
    } else {
        sigma_td + tgtd / z_t
    };
    let y_t = sigma_t * z_t;

    //
    /////////////////////////////////

    let y_s_temp = pow2(sigma_s) + pow2(y_t) / (7.8 + pow2(z_s)) + pow2(y_l) / (24.0 + pow2(z_s)); // Part of [Algorithm, Eqn 5.11]

    let (y_r, y_s) = match mdvar.mode {
        VariabilityMode::SingleMessage => {
            (0.0, (pow2(sigma_t) + pow2(sigma_l) + y_s_temp).sqrt() * z_s)
        }
        VariabilityMode::Accidental => (y_t, (pow2(sigma_l) + y_s_temp).sqrt() * z_s),
        VariabilityMode::Mobile => (
            (pow2(sigma_t) + pow2(sigma_l)).sqrt() * z_t,
            y_s_temp.sqrt() * z_s,
        ),
        VariabilityMode::Broadcast => (y_t + y_l, y_s_temp.sqrt() * z_s),
    };

    let result = a_ref_db - v_med - y_r - y_s;

    // [Algorithm, Eqn 52]
    if result < 0.0 {
        result * (29.0 - result) / (29.0 - 10.0 * result)
    } else {
        result
    }
}

/// `InverseComplementaryCumulativeDistributionFunction`: the standard normal
/// deviate exceeded with probability `q`, by Abramowitz & Stegun formula
/// 26.2.23 (error below 4.5e-4).
fn inverse_ccdf(q: f64) -> f64 {
    const C_0: f64 = 2.515516;
    const C_1: f64 = 0.802853;
    const C_2: f64 = 0.010328;
    const D_1: f64 = 1.432788;
    const D_2: f64 = 0.189269;
    const D_3: f64 = 0.001308;

    let x = if q > 0.5 { 1.0 - q } else { q };

    let t_x = (-2.0 * x.ln()).sqrt();

    let zeta_x = ((C_2 * t_x + C_1) * t_x + C_0) / (((D_3 * t_x + D_2) * t_x + D_1) * t_x + 1.0);

    let q_q = t_x - zeta_x;

    if q > 0.5 { -q_q } else { q_q }
}

/// `FreeSpaceLoss`: the basic transmission loss in free space.
fn free_space_loss(d_meter: f64, f_mhz: f64) -> f64 {
    32.45 + 20.0 * f_mhz.log10() + 20.0 * (d_meter / 1000.0).log10()
}

/// `SigmaHFunction`: the RMS deviation of terrain and clutter within the
/// first Fresnel zone in the dominant reflecting plane [ERL 79-ITS 67, Eqn 3.6a].
fn sigma_h_function(delta_h: f64) -> f64 {
    0.78 * delta_h * (-0.5 * delta_h.powf(0.25)).exp()
}

/// `TerrainRoughness`: the terrain irregularity of a path of this length
/// [ERL 79-ITS 67, Eqn 3], with the distance in metres.
fn terrain_roughness(d_meter: f64, delta_h: f64) -> f64 {
    delta_h * (1.0 - 0.8 * (-d_meter / 50e3).exp())
}
