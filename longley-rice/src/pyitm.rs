// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>
//
// This module reproduces the behaviour of pyitm 0.3, Copyright (c) 2019
// tmd224, released under the MIT licence; see the NOTICE file.

//! A faithful reproduction of pyitm 0.3's `itm.area()`, bugs included.
//!
//! pyitm is a Python translation of ITM 1.2.2, older than the version
//! [`area_tls`](crate::area_tls) follows, and its translation has bugs that
//! move its results away from ITM's:
//!
//! - in the line-of-sight loss, the phase difference above 1.57 rad becomes
//!   the constant `3.14 - 2.4649` instead of `3.14 - 2.4649 / q`;
//! - in the variability, the time variability above the median is multiplied
//!   by a factor that is never assigned and stays 0, so it is always zero;
//! - in the propagation, the diffraction-troposcatter transition distance uses
//!   a variable that is never assigned and stays 0;
//! - in the variability too, situation variability is dropped when location
//!   variability is (mode of variability 10 to 13), where ITM drops it only
//!   for 20 to 23.
//!
//! The function here is not ITM: it is for programs that used pyitm and need
//! their numbers not to move when they stop using it, and for measuring how
//! far pyitm is from ITM. For predictions, use [`area_tls`](crate::area_tls)
//! or [`area_cr`](crate::area_cr).
//!
//! Every operation is done as CPython 3 does it, in the same order, with the
//! same library calls (libm's `pow`, `log`, `exp`... the ones Python's `math`
//! module calls) and CPython's own complex square root, logarithm and
//! division, so on the same platform the result is the same `f64`, bit for
//! bit. Where pyitm would raise an exception, an [`Error`] says which.

// pyitm's constants are kept as pyitm writes them, 3.14 and 1.4142 among them.
#![allow(clippy::approx_constant)]

use crate::complex::Complex;
use std::fmt;
use std::hint::black_box;

/// Why pyitm's `area()` would not have returned a result.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Error {
    /// pyitm's `InputError`, which `area()` raises only for a frequency
    /// outside 20 to 20000 MHz. Callers of pyitm usually catch this one.
    InputError { frq_mhz: f64 },
    /// Python's `ValueError` from the `math` module ("math domain error"),
    /// such as a logarithm of zero or a square root of a negative number.
    ValueError,
    /// Python's `ZeroDivisionError`.
    ZeroDivisionError,
    /// Python's `OverflowError`, from a `math` function whose result is too
    /// large.
    OverflowError,
    /// A radio climate of zero or less. pyitm then skips setting the climate's
    /// coefficients and uses whatever the previous call left behind, so the
    /// result depends on what was computed before and cannot be reproduced by
    /// a function without that history.
    UnsupportedClimate(i32),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InputError { frq_mhz } => write!(
                f,
                "frq_mhz={frq_mhz}MHz: ITM model is only valid for frequencies 20MHz - 20GHz"
            ),
            Self::ValueError => f.write_str("math domain error"),
            Self::ZeroDivisionError => f.write_str("division by zero"),
            Self::OverflowError => f.write_str("math range error"),
            Self::UnsupportedClimate(climate) => write!(
                f,
                "radio_climate={climate}: pyitm's result depends on the previous call's state"
            ),
        }
    }
}

impl std::error::Error for Error {}

type Result<T> = std::result::Result<T, Error>;

/// pyitm's `area()`, with its arguments in its order and with its meaning.
///
/// Returns the loss in dB and pyitm's `errnum`: 0 when all is well, else the
/// highest of 1 (some parameter is nearly out of range), 2 (a default was
/// substituted for an impossible parameter), 3 (a combination of parameters
/// is out of range) and 4 (some parameter is out of range, the result is
/// probably invalid).
///
/// The arguments are those of the Python function:
///
/// - `mod_var`: the mode of variability, 0 (single message), 1 (individual),
///   2 (mobile) or 3 (broadcast), plus 10 and 20 as in ITM (with the bug
///   described in the [module documentation](self)).
/// - `delta_h`: the terrain irregularity parameter, in metres.
/// - `tht_m`, `rht_m`: the antenna heights, in metres.
/// - `dist_km`: the distance, in kilometres.
/// - `t_site_criteria`, `r_site_criteria`: the siting, 0 (random), 1
///   (careful) or 2 (very careful); anything else above 0 counts as 2 and
///   anything below as 0.
/// - `eps_dielect`, `sgm_conductivity`: the ground's relative permittivity
///   and conductivity, in S/m.
/// - `eno_ns_surfref`: the surface refractivity, in N-units.
/// - `frq_mhz`: the frequency, in MHz.
/// - `radio_climate`: the climate, 1 to 7 as in ITM; above 7 counts as 5.
/// - `pol`: the polarization, 0 for horizontal and anything else vertical.
/// - `pct_time`, `pct_loc`, `pct_conf`: the time, location and confidence
///   quantiles as fractions, 0.01 to 0.99 in pyitm's documentation but not
///   checked.
///
/// Python ints convert to these `f64` exactly, and pyitm converts the two
/// siting criteria with `int()`, which is why they are integers here. The
/// only check pyitm makes is the frequency's; out-of-range values otherwise
/// go into the computation and come out as a result, an `errnum` or a Python
/// exception, and the same happens here.
#[allow(clippy::too_many_arguments)]
pub fn area(
    mod_var: i32,
    delta_h: f64,
    tht_m: f64,
    rht_m: f64,
    dist_km: f64,
    t_site_criteria: i32,
    r_site_criteria: i32,
    eps_dielect: f64,
    sgm_conductivity: f64,
    eno_ns_surfref: f64,
    frq_mhz: f64,
    radio_climate: i32,
    pol: i32,
    pct_time: f64,
    pct_loc: f64,
    pct_conf: f64,
) -> Result<(f64, i32)> {
    let mut s = State::default();
    let kst = [t_site_criteria, r_site_criteria];
    let zt = qerfi(pct_time)?;
    let zl = qerfi(pct_loc)?;
    let zc = qerfi(pct_conf)?;
    s.dh = delta_h;
    s.hg = [tht_m, rht_m];
    s.klim = radio_climate;
    s.ens = eno_ns_surfref;
    s.kwx = 0;

    // argument checking
    if frq_mhz < 20.0 || frq_mhz > 20000.0 {
        return Err(Error::InputError { frq_mhz });
    }
    // Where pyitm would carry state over from the previous call (see the
    // variant's documentation).
    if radio_climate <= 0 {
        return Err(Error::UnsupportedClimate(radio_climate));
    }

    s.qlrps(
        frq_mhz,
        0.0,
        eno_ns_surfref,
        pol,
        eps_dielect,
        sgm_conductivity,
    )?;
    s.qlra(kst, radio_climate, mod_var)?;
    if s.lvar < 1 {
        s.lvar = 1;
    }

    s.lrprop(dist_km * 1000.0)?;

    let fs = 32.45 + 20.0 * py::log10(frq_mhz)? + 20.0 * py::log10(s.dist / 1000.0)?;
    let xlb = fs + s.avar(zt, zl, zc)?;
    Ok((xlb, s.kwx))
}

/// Python's semantics for the operations pyitm uses: the `math` functions
/// raise instead of returning NaN or infinity for finite arguments, and float
/// division by zero raises.
mod py {
    use super::{Error, Result, black_box};
    use crate::complex::Complex;

    pub fn div(a: f64, b: f64) -> Result<f64> {
        if b == 0.0 {
            Err(Error::ZeroDivisionError)
        } else {
            Ok(a / b)
        }
    }

    pub fn log(x: f64) -> Result<f64> {
        if x > 0.0 || x.is_nan() {
            Ok(x.ln())
        } else {
            Err(Error::ValueError)
        }
    }

    pub fn log10(x: f64) -> Result<f64> {
        if x > 0.0 || x.is_nan() {
            Ok(x.log10())
        } else {
            Err(Error::ValueError)
        }
    }

    pub fn sqrt(x: f64) -> Result<f64> {
        if x < 0.0 {
            Err(Error::ValueError)
        } else {
            Ok(x.sqrt())
        }
    }

    pub fn exp(x: f64) -> Result<f64> {
        let r = x.exp();
        if r.is_infinite() && x.is_finite() {
            Err(Error::OverflowError)
        } else {
            Ok(r)
        }
    }

    pub fn sin(x: f64) -> Result<f64> {
        if x.is_infinite() {
            Err(Error::ValueError)
        } else {
            Ok(x.sin())
        }
    }

    pub fn cos(x: f64) -> Result<f64> {
        if x.is_infinite() {
            Err(Error::ValueError)
        } else {
            Ok(x.cos())
        }
    }

    /// `math.pow`. The exponent goes through `black_box` so that the call
    /// stays a call to libm's `pow`, as in Python: left alone, LLVM turns
    /// `pow(x, 2.0)` into `x * x`, which is not always the same bits.
    pub fn pow(x: f64, y: f64) -> Result<f64> {
        let r = x.powf(black_box(y));
        if x.is_finite() && y.is_finite() && !r.is_finite() {
            if r.is_nan() || x == 0.0 {
                return Err(Error::ValueError);
            }
            return Err(Error::OverflowError);
        }
        Ok(r)
    }

    /// `int()` of a float, which raises for NaN and infinity. Values beyond
    /// i64 saturate, which is enough for the comparisons it is used in.
    pub fn int(x: f64) -> Result<i64> {
        if x.is_nan() {
            Err(Error::ValueError)
        } else if x.is_infinite() {
            Err(Error::OverflowError)
        } else {
            Ok(x as i64)
        }
    }

    /// Python's `max(a, b)`: the first unless the second is greater.
    pub fn max(a: f64, b: f64) -> f64 {
        if b > a { b } else { a }
    }

    /// Python's `min(a, b)`: the first unless the second is smaller.
    pub fn min(a: f64, b: f64) -> f64 {
        if b < a { b } else { a }
    }

    /// A Python float times `1j`: the float made complex, then the complex
    /// product.
    pub fn imag(x: f64) -> Complex {
        Complex::real(x).mul(Complex::new(0.0, 1.0))
    }

    /// Complex division, which raises on a zero divisor.
    pub fn cdiv(a: Complex, b: Complex) -> Result<Complex> {
        if b.re == 0.0 && b.im == 0.0 {
            Err(Error::ZeroDivisionError)
        } else {
            Ok(a.div(b))
        }
    }

    /// `abs()` of a complex, which raises if the result overflows.
    pub fn cabs(z: Complex) -> Result<f64> {
        let r = z.abs();
        if r.is_infinite() && z.re.is_finite() && z.im.is_finite() {
            Err(Error::OverflowError)
        } else {
            Ok(r)
        }
    }

    /// The real part of `cmath.log(x)` for a real `x`, as CPython's `c_log`
    /// computes it: near 1 it goes through `log1p` rather than `log`, which
    /// is not always the same bits.
    pub fn clog_real(x: f64) -> Result<f64> {
        const LARGE: f64 = f64::MAX / 4.0;
        let ax = x.abs();
        if x.is_nan() {
            return Ok(x);
        }
        if ax.is_infinite() {
            return Ok(f64::INFINITY);
        }
        if ax > LARGE {
            Ok((ax / 2.0).ln() + std::f64::consts::LN_2)
        } else if ax < f64::MIN_POSITIVE {
            if ax > 0.0 {
                // ldexp(ax, DBL_MANT_DIG), then the scale taken back out.
                Ok((ax * 9007199254740992.0).ln() - 53.0 * std::f64::consts::LN_2)
            } else {
                Err(Error::ValueError)
            }
        } else if (0.71..=1.73).contains(&ax) {
            Ok(((ax - 1.0) * (ax + 1.0) + 0.0 * 0.0).ln_1p() / 2.0)
        } else {
            Ok(ax.ln())
        }
    }
}

/// `x - y` when x is greater, else 0: Fortran's DIM, as pyitm's `util` has it.
fn fortran_dim(x: f64, y: f64) -> f64 {
    if x > y { x - y } else { 0.0 }
}

const THIRD: f64 = 1.0 / 3.0;

/// Everything pyitm keeps between its functions: its `prop`, `propa` and
/// `propv` objects, and the function attributes that stand in for ITM's C++
/// statics. pyitm creates the objects afresh in each `area()` call and sets
/// all the statics before reading them (for a radio climate above 0), except
/// `lrprop.xae` and `avar.gp`, which nothing ever sets: they stay at the 0.0
/// they start as, and so they do here.
#[derive(Default)]
struct State {
    // prop
    aref: f64,
    dist: f64,
    hg: [f64; 2],
    wn: f64,
    dh: f64,
    ens: f64,
    gme: f64,
    zgnd: Impedance,
    he: [f64; 2],
    dl: [f64; 2],
    the: [f64; 2],
    kwx: i32,
    mdp: i32,
    // propv
    sgc: f64,
    lvar: i32,
    mdvar: i32,
    klim: i32,
    // propa
    dlsa: f64,
    dx: f64,
    ael: f64,
    ak1: f64,
    ak2: f64,
    aed: f64,
    emd: f64,
    aes: f64,
    ems: f64,
    dls: [f64; 2],
    dla: f64,
    tha: f64,
    // lrprop statics
    wlos: bool,
    wscat: bool,
    dmin: f64,
    /// Never assigned in pyitm (the value goes to a local of the same name).
    xae: f64,
    // alos static
    wls: f64,
    // adiff statics
    wd1: f64,
    xd1: f64,
    afo: f64,
    qk: f64,
    aht: f64,
    xht: f64,
    // ascat statics
    ad: f64,
    rr: f64,
    etq: f64,
    h0s: f64,
    // avar statics
    kdv: i32,
    dexa: f64,
    de: f64,
    vmd: f64,
    sgl: f64,
    sgtm: f64,
    sgtp: f64,
    sgtd: f64,
    tgtd: f64,
    gm: f64,
    /// Never assigned in pyitm (the value goes to `avar.qp`).
    gp: f64,
    cv: [f64; 5],
    csm: [f64; 5],
    csp: [f64; 5],
    csd1: f64,
    zd: f64,
    cfm: [f64; 3],
    cfp: [f64; 3],
    ws: bool,
    w1: bool,
    vs0: f64,
    sgt: f64,
}

/// The ground impedance as pyitm keeps it, as two floats.
#[derive(Clone, Copy, Default)]
struct Impedance {
    re: f64,
    im: f64,
}

impl Impedance {
    /// `prop.zgndreal + prop.zgndimag*1j`, as pyitm rebuilds it each time.
    fn get(self) -> Complex {
        Complex::real(self.re).add(py::imag(self.im))
    }
}

impl State {
    fn qlrps(
        &mut self,
        fmhz: f64,
        zsys: f64,
        en0: f64,
        ipol: i32,
        eps: f64,
        sgm: f64,
    ) -> Result<()> {
        let gma = 157E-9;
        self.wn = py::div(fmhz, 47.7)?;
        self.ens = en0;
        if zsys != 0.0 {
            self.ens *= py::exp(py::div(-zsys, 9460.0)?)?;
        }
        self.gme = gma * (1.0 - 0.04665 * py::exp(py::div(self.ens, 179.3)?)?);
        let zq = Complex::real(eps).add(py::imag(376.62 * py::div(sgm, self.wn)?));
        let mut zgnd = zq.sub(Complex::real(1.0)).sqrt_cpython();
        if ipol != 0 {
            zgnd = py::cdiv(zgnd, zq)?;
        }
        self.zgnd = Impedance {
            re: zgnd.re,
            im: zgnd.im,
        };
        Ok(())
    }

    #[allow(clippy::needless_range_loop)]
    fn qlra(&mut self, kst: [i32; 2], klimx: i32, mdvarx: i32) -> Result<()> {
        for j in 0..2 {
            if kst[j] <= 0 {
                self.he[j] = self.hg[j];
            } else {
                let mut q = 4.0;
                if kst[j] != 1 {
                    q = 9.0;
                }
                if self.hg[j] < 5.0 {
                    q *= py::sin(0.3141593 * self.hg[j])?;
                }
                self.he[j] = self.hg[j]
                    + (1.0 + q)
                        * py::exp(-py::min(
                            20.0,
                            py::div(2.0 * self.hg[j], py::max(1E-3, self.dh))?,
                        ))?;
            }
            let q = py::sqrt(py::div(2.0 * self.he[j], self.gme)?)?;
            self.dl[j] =
                q * py::exp(-0.07 * py::sqrt(py::div(self.dh, py::max(self.he[j], 5.0))?)?)?;
            self.the[j] = py::div(
                0.65 * self.dh * (py::div(q, self.dl[j])? - 1.0) - 2.0 * self.he[j],
                q,
            )?;
        }
        self.mdp = 1;
        self.lvar = self.lvar.max(3);
        if mdvarx >= 0 {
            self.mdvar = mdvarx;
            self.lvar = self.lvar.max(4);
        }
        if klimx > 0 {
            self.klim = klimx;
            self.lvar = 5;
        }
        Ok(())
    }

    fn lrprop(&mut self, d: f64) -> Result<()> {
        let zgnd = self.zgnd.get();
        if self.mdp != 0 {
            for j in 0..2 {
                self.dls[j] = py::sqrt(py::div(2.0 * self.he[j], self.gme)?)?;
            }
            self.dlsa = self.dls[0] + self.dls[1];
            self.dla = self.dl[0] + self.dl[1];
            self.tha = py::max(self.the[0] + self.the[1], -self.dla * self.gme);
            self.wlos = false;
            self.wscat = false;

            if self.wn < 0.838 || self.wn > 210.0 {
                self.kwx = self.kwx.max(1);
            }
            for j in 0..2 {
                if self.hg[j] < 1.0 || self.hg[j] > 1000.0 {
                    self.kwx = self.kwx.max(1);
                }
            }
            for j in 0..2 {
                if self.the[j].abs() > 200e-3
                    || self.dl[j] < 0.1 * self.dls[j]
                    || self.dl[j] > 3.0 * self.dls[j]
                {
                    self.kwx = self.kwx.max(3);
                }
            }
            if self.ens < 250.0
                || self.ens > 400.0
                || self.gme < 75E-9
                || self.gme > 250E-9
                || zgnd.re <= zgnd.im.abs()
                || self.wn < 0.419
                || self.wn > 420.0
            {
                self.kwx = 4;
            }
            for j in 0..2 {
                if self.hg[j] < 0.5 || self.hg[j] > 3000.0 {
                    self.kwx = 4;
                }
            }

            self.dmin = py::div((self.he[0] - self.he[1]).abs(), 200E-3)?;
            self.adiff(0.0)?;
            // pyitm assigns this to a local, so the `xae` used below stays 0.
            let xae = py::pow(self.wn * py::pow(self.gme, 2.0)?, -THIRD)?;
            let d3 = py::max(self.dlsa, 1.3787 * xae + self.dla);
            let d4 = d3 + 2.7574 * xae;
            let a3 = self.adiff(d3)?;
            let a4 = self.adiff(d4)?;
            self.emd = py::div(a4 - a3, d4 - d3)?;
            self.aed = a3 - self.emd * d3;
        }

        if self.mdp >= 0 {
            self.mdp = 0;
            self.dist = d;
        }

        if self.dist > 0.0 {
            if self.dist > 1000E3 {
                self.kwx = self.kwx.max(1);
            }
            if self.dist < self.dmin {
                self.kwx = self.kwx.max(3);
            }
            if self.dist < 1E3 || self.dist > 2000E3 {
                self.kwx = 4;
            }
        }

        if self.dist < self.dlsa && !self.wlos {
            self.alos(0.0)?;
            let d2 = self.dlsa;
            let a2 = self.aed + d2 * self.emd;
            let mut d0 = 1.908 * self.wn * self.he[0] * self.he[1];
            let d1 = if self.aed >= 0.0 {
                d0 = py::min(d0, 0.5 * self.dla);
                d0 + 0.25 * (self.dla - d0)
            } else {
                py::max(py::div(-self.aed, self.emd)?, 0.25 * self.dla)
            };
            let a1 = self.alos(d1)?;
            let mut wq = false;
            if d0 < d1 {
                let a0 = self.alos(d0)?;
                let q = py::log(py::div(d2, d0)?)?;
                let numerator = (d2 - d0) * (a1 - a0) - (d1 - d0) * (a2 - a0);
                let denominator = (d2 - d0) * py::log(py::div(d1, d0)?)? - (d1 - d0) * q;
                self.ak2 = py::max(0.0, py::div(numerator, denominator)?);
                wq = self.aed >= 0.0 || self.ak2 > 0.0;
                if wq {
                    self.ak1 = py::div(a2 - a0 - self.ak2 * q, d2 - d0)?;
                    if self.ak1 < 0.0 {
                        self.ak1 = 0.0;
                        self.ak2 = py::div(fortran_dim(a2, a0), q)?;
                        if self.ak2 == 0.0 {
                            self.ak1 = self.emd;
                        }
                    }
                }
            }
            if !wq {
                self.ak1 = py::div(fortran_dim(a2, a1), d2 - d1)?;
                self.ak2 = 0.0;
                if self.ak1 == 0.0 {
                    self.ak1 = self.emd;
                }
            }
            self.ael = a2 - self.ak1 * d2 - self.ak2 * py::log(d2)?;
            self.wlos = true;
            if self.dist > 0.0 {
                self.aref = self.ael + self.ak1 * self.dist + self.ak2 * py::log(self.dist)?;
            }
        }

        if self.dist <= 0.0 || self.dist >= self.dlsa {
            if !self.wscat {
                self.ascat(0.0)?;
                let d5 = self.dla + 200E3;
                let d6 = d5 + 200E3;
                let a6 = self.ascat(d6)?;
                let a5 = self.ascat(d5)?;
                if a5 < 1000.0 {
                    self.ems = py::div(a6 - a5, 200E3)?;
                    let transition = self.dla + 0.3 * self.xae * py::log(47.7 * self.wn)?;
                    let crossing = py::div(a5 - self.aed - self.ems * d5, self.emd - self.ems)?;
                    self.dx = py::max(self.dlsa, py::max(transition, crossing));
                    self.aes = (self.emd - self.ems) * self.dx + self.aed;
                } else {
                    self.ems = self.emd;
                    self.aes = self.aed;
                    self.dx = 10.0E6;
                }
                self.wscat = true;
            }
            if self.dist > self.dx {
                self.aref = self.aes + self.ems * self.dist;
            } else {
                self.aref = self.aed + self.emd * self.dist;
            }
        }

        self.aref = py::max(self.aref, 0.0);
        Ok(())
    }

    fn alos(&mut self, d: f64) -> Result<f64> {
        let zgnd = self.zgnd.get();
        if d == 0.0 {
            self.wls = py::div(
                0.021,
                0.021 + py::div(self.wn * self.dh, py::max(10E3, self.dlsa))?,
            )?;
            return Ok(0.0);
        }
        let mut q = (1.0 - 0.8 * py::exp(py::div(-d, 50E3)?)?) * self.dh;
        let s = 0.78 * q * py::exp(-py::pow(py::div(q, 16.0)?, 0.25)?)?;
        q = self.he[0] + self.he[1];
        let sps = py::div(q, py::sqrt((d * d) + (q * q))?)?;
        let sps_c = Complex::real(sps);
        let mut r = py::cdiv(sps_c.sub(zgnd), sps_c.add(zgnd))?
            .mul(Complex::real(py::exp(-py::min(10.0, self.wn * s * sps))?));
        q = r.norm_sqr();
        if q < 0.25 || q < sps {
            r = r.mul(Complex::real(py::sqrt(py::div(sps, q)?)?));
        }
        let alosv = (self.emd * d) + self.aed;
        q = py::div(self.wn * self.he[0] * self.he[1] * 2.0, d)?;
        if q > 1.57 {
            // pyitm's bug: ITM has 3.14 - 2.4649 / q.
            q = 3.14 - 2.4649;
        }
        let cos = py::cos(q)?;
        let sin = py::sin(q)?;
        let sum = Complex::real(cos).sub(py::imag(sin)).add(r);
        let log = Complex::new(
            py::clog_real(sum.norm_sqr())?,
            0.0_f64.atan2(sum.norm_sqr()),
        );
        let alosv_c = Complex::real(-4.343)
            .mul(log)
            .sub(Complex::real(alosv))
            .mul(Complex::real(self.wls))
            .add(Complex::real(alosv));
        Ok(alosv_c.re)
    }

    fn adiff(&mut self, d: f64) -> Result<f64> {
        let zgnd = self.zgnd.get();
        if d == 0.0 {
            let mut q = self.hg[0] * self.hg[1];
            self.qk = self.he[0] * self.he[1] - q;
            if self.mdp < 0 {
                q += 10.0;
            }
            self.wd1 = py::sqrt(1.0 + py::div(self.qk, q)?)?;
            self.xd1 = self.dla + py::div(self.tha, self.gme)?;
            q = (1.0 - 0.8 * py::exp(py::div(-self.dlsa, 50E3)?)?) * self.dh;
            q *= 0.78 * py::exp(-py::pow(py::div(q, 16.0)?, 0.25)?)?;
            self.afo = py::min(
                15.0,
                2.171 * py::log(1.0 + 4.77E-4 * self.hg[0] * self.hg[1] * self.wn * q)?,
            );
            self.qk = py::div(1.0, py::cabs(zgnd)?)?;
            self.aht = 20.0;
            self.xht = 0.0;
            for j in 0..2 {
                let a = py::div(0.5 * py::pow(self.dl[j], 2.0)?, self.he[j])?;
                let wa = py::pow(a * self.wn, THIRD)?;
                let pk = py::div(self.qk, wa)?;
                let q = py::div((1.607 - pk) * 151.0 * wa * self.dl[j], a)?;
                self.xht += q;
                self.aht += fht(q, pk)?;
            }
            return Ok(0.0);
        }
        let th = self.tha + d * self.gme;
        let ds = d - self.dla;
        let mut q = 0.0795775 * self.wn * ds * py::pow(th, 2.0)?;
        let mut adiffv = aknfe(py::div(q * self.dl[0], ds + self.dl[0])?)?
            + aknfe(py::div(q * self.dl[1], ds + self.dl[1])?)?;
        let a = py::div(ds, th)?;
        let wa = py::pow(a * self.wn, THIRD)?;
        let pk = py::div(self.qk, wa)?;
        q = (1.607 - pk) * 151.0 * wa * th + self.xht;
        let ar = 0.05751 * q - 4.343 * py::log(q)? - self.aht;
        q = (self.wd1 + py::div(self.xd1, d)?)
            * py::min(
                (1.0 - 0.8 * py::exp(py::div(-d, 50e3)?)?) * self.dh * self.wn,
                6283.2,
            );
        let wd = py::div(25.1, 25.1 + py::sqrt(q)?)?;
        adiffv = ar * wd + (1.0 - wd) * adiffv + self.afo;
        Ok(adiffv)
    }

    fn ascat(&mut self, d: f64) -> Result<f64> {
        if d == 0.0 {
            self.ad = self.dl[0] - self.dl[1];
            self.rr = py::div(self.he[1], self.he[0])?;
            if self.ad < 0.0 {
                self.ad = -self.ad;
                self.rr = py::div(1.0, self.rr)?;
            }
            self.etq = (5.67E-6 * self.ens - 2.32E-3) * self.ens + 0.031;
            self.h0s = -15.0;
            return Ok(0.0);
        }
        let mut h0;
        if self.h0s > 15.0 {
            h0 = self.h0s;
        } else {
            let th = self.the[0] + self.the[1] + d * self.gme;
            let mut r2 = 2.0 * self.wn * th;
            let r1 = r2 * self.he[0];
            r2 *= self.he[1];
            if r1 < 0.2 && r2 < 0.2 {
                return Ok(1001.0); // early return
            }
            let mut ss = py::div(d - self.ad, d + self.ad)?;
            let mut q = py::div(self.rr, ss)?;
            ss = py::max(0.1, ss);
            q = py::min(py::max(0.1, q), 10.0);
            let z0 = py::div((d - self.ad) * (d + self.ad) * th * 0.25, d)?;
            let et = py::div(
                (self.etq * py::exp(-py::pow(py::min(1.7, py::div(z0, 8.0E3)?), 6.0)?)? + 1.0) * z0,
                1.7556e3,
            )?;
            let ett = py::max(et, 1.0);
            h0 = (h0f(r1, ett)? + h0f(r2, ett)?) * 0.5;
            h0 += py::min(
                h0,
                (1.38 - py::log(ett)?) * py::log(ss)? * py::log(q)? * 0.49,
            );
            h0 = fortran_dim(h0, 0.0);
            if et < 1.0 {
                let base = (1.0 + py::div(1.4142, r1)?) * (1.0 + py::div(1.4142, r2)?);
                let arg = py::div(py::pow(base, 2.0)? * (r1 + r2), r1 + r2 + 2.8284)?;
                h0 = et * h0 + (1.0 - et) * 4.343 * py::log(arg)?;
            }
            if h0 > 15.0 && self.h0s >= 0.0 {
                h0 = self.h0s;
            }
        }
        self.h0s = h0;
        let th = self.tha + d * self.gme;
        Ok(
            ahd(th * d)? + 4.343 * py::log(47.7 * self.wn * py::pow(th, 4.0)?)?
                - 0.1 * (self.ens - 301.0) * py::exp(py::div(-th * d, 40e3)?)?
                + h0,
        )
    }

    fn avar(&mut self, zzt: f64, zzl: f64, zzc: f64) -> Result<f64> {
        const BV1: [f64; 7] = [-9.67, -0.62, 1.26, -9.21, -0.62, -0.39, 3.15];
        const BV2: [f64; 7] = [12.7, 9.19, 15.5, 9.05, 9.19, 2.86, 857.9];
        const XV1: [f64; 7] = [
            144.9e3, 228.9e3, 262.6e3, 84.1e3, 228.9e3, 141.7e3, 2222.0e3,
        ];
        const XV2: [f64; 7] = [
            190.3e3, 205.2e3, 185.2e3, 101.1e3, 205.2e3, 315.9e3, 164.8e3,
        ];
        const XV3: [f64; 7] = [133.8e3, 143.6e3, 99.8e3, 98.6e3, 143.6e3, 167.4e3, 116.3e3];
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
        const BSD1: [f64; 7] = [1.224, 0.801, 1.380, 1.000, 1.224, 1.518, 1.518];
        const BZD1: [f64; 7] = [1.282, 2.161, 1.282, 20.0, 1.282, 1.282, 1.282];
        const BFM1: [f64; 7] = [1.0, 1.0, 1.0, 1.0, 0.92, 1.0, 1.0];
        const BFM2: [f64; 7] = [0.0, 0.0, 0.0, 0.0, 0.25, 0.0, 0.0];
        const BFM3: [f64; 7] = [0.0, 0.0, 0.0, 0.0, 1.77, 0.0, 0.0];
        const BFP1: [f64; 7] = [1.0, 0.93, 1.0, 0.93, 0.93, 1.0, 1.0];
        const BFP2: [f64; 7] = [0.0, 0.31, 0.0, 0.19, 0.31, 0.0, 0.0];
        const BFP3: [f64; 7] = [0.0, 2.00, 0.0, 1.79, 2.00, 0.0, 0.0];

        let rt = 7.8;
        let rl = 24.0;

        if self.lvar > 0 {
            // pyitm's switch, entered at its default case: `area()` always
            // reaches here with lvar 5 for the climates it is called with.
            let mut k = (self.klim - 1) as usize;
            if self.klim <= 0 || self.klim > 7 {
                self.klim = 5;
                k = 4;
                self.kwx = self.kwx.max(2);
            }
            self.cv = [BV1[k], BV2[k], XV1[k], XV2[k], XV3[k]];
            self.csm = [BSM1[k], BSM2[k], XSM1[k], XSM2[k], XSM3[k]];
            self.csp = [BSP1[k], BSP2[k], XSP1[k], XSP2[k], XSP3[k]];
            self.csd1 = BSD1[k];
            self.zd = BZD1[k];
            self.cfm = [BFM1[k], BFM2[k], BFM3[k]];
            self.cfp = [BFP1[k], BFP2[k], BFP3[k]];

            // case 4
            self.kdv = self.mdvar;
            self.ws = self.kdv >= 20;
            if self.ws {
                self.kdv -= 20;
            }
            self.w1 = self.kdv >= 10;
            if self.w1 {
                self.kdv -= 10;
            }
            if self.kdv < 0 || self.kdv > 3 {
                self.kdv = 0;
                self.kwx = self.kwx.max(2);
            }

            // case 3
            let q = py::log(0.133 * self.wn)?;
            self.gm = self.cfm[0] + py::div(self.cfm[1], py::pow(self.cfm[2] * q, 2.0)? + 1.0)?;
            // pyitm's bug: this goes to `avar.qp`, and `avar.gp` stays 0.
            let _qp = self.cfp[0] + py::div(self.cfp[1], py::pow(self.cfp[2] * q, 2.0)? + 1.0)?;

            // case 2
            self.dexa = py::sqrt(18e6 * self.he[0])?
                + py::sqrt(18e6 * self.he[1])?
                + py::pow(py::div(575.7e12, self.wn)?, THIRD)?;

            // case 1
            if self.dist < self.dexa {
                self.de = py::div(130E3 * self.dist, self.dexa)?;
            } else {
                self.de = 130E3 + self.dist - self.dexa;
            }

            let [c1, c2, x1, x2, x3] = self.cv;
            self.vmd = curve(c1, c2, x1, x2, x3, self.de)?;
            let [c1, c2, x1, x2, x3] = self.csm;
            self.sgtm = curve(c1, c2, x1, x2, x3, self.de)? * self.gm;
            let [c1, c2, x1, x2, x3] = self.csp;
            self.sgtp = curve(c1, c2, x1, x2, x3, self.de)? * self.gp;
            self.sgtd = self.sgtp * self.csd1;
            self.tgtd = (self.sgtp - self.sgtd) * self.zd;
            if self.w1 {
                self.sgl = 0.0;
            } else {
                let q = (1.0 - 0.8 * py::exp(py::div(-self.dist, 50E3)?)?) * self.dh * self.wn;
                self.sgl = py::div(10.0 * q, q + 13.0)?;
            }
            // pyitm's bug: ITM tests `ws` here.
            if self.w1 {
                self.vs0 = 0.0;
            } else {
                self.vs0 = py::pow(5.0 + 3.0 * py::exp(py::div(-self.de, 100E3)?)?, 2.0)?;
            }
            self.lvar = 0;
        }

        let mut zt = zzt;
        let mut zl = zzl;
        let zc = zzc;
        match self.kdv {
            0 => {
                zt = zc;
                zl = zc;
            }
            1 => zl = zc,
            2 => zl = zt,
            _ => {}
        }

        if zt.abs() > 3.1 || zl.abs() > 3.1 || zc.abs() > 3.1 {
            self.kwx = self.kwx.max(1);
        }

        self.sgt = if zt < 0.0 {
            self.sgtm
        } else if zt <= self.zd {
            self.sgtp
        } else {
            self.sgtd + py::div(self.tgtd, zt)?
        };

        let vs = self.vs0
            + py::div(py::pow(self.sgt * zt, 2.0)?, rt + zc * zc)?
            + py::div(py::pow(self.sgl * zl, 2.0)?, rl + zc * zc)?;

        let (sgt, sgl) = (self.sgt, self.sgl);
        let yr;
        match self.kdv {
            0 => {
                yr = 0.0;
                self.sgc = py::sqrt(sgt * sgt + sgl * sgl + vs)?;
            }
            1 => {
                yr = sgt * zt;
                self.sgc = py::sqrt(sgl * sgl + vs)?;
            }
            2 => {
                yr = py::sqrt(sgt * sgt + sgl * sgl)? * zt;
                self.sgc = py::sqrt(vs)?;
            }
            _ => {
                yr = sgt * zt + sgl * zl;
                self.sgc = py::sqrt(vs)?;
            }
        }

        let mut avarv = self.aref - self.vmd - yr - self.sgc * zc;
        if avarv < 0.0 {
            avarv = py::div(avarv * (29.0 - avarv), 29.0 - 10.0 * avarv)?;
        }
        Ok(avarv)
    }
}

fn curve(c1: f64, c2: f64, x1: f64, x2: f64, x3: f64, de: f64) -> Result<f64> {
    let first = c1 + py::div(c2, 1.0 + py::pow(py::div(de - x2, x3)?, 2.0)?)?;
    let numerator = first * py::pow(py::div(de, x1)?, 2.0)?;
    py::div(numerator, 1.0 + py::pow(py::div(de, x1)?, 2.0)?)
}

fn qerfi(q: f64) -> Result<f64> {
    let c0 = 2.515516698;
    let c1 = 0.802853;
    let c2 = 0.010328;
    let d1 = 1.432788;
    let d2 = 0.189269;
    let d3 = 0.001308;
    let x = 0.5 - q;
    let mut t = py::max(0.5 - x.abs(), 0.000001);
    t = py::sqrt(-2.0 * py::log(t)?)?;
    let mut v = t - py::div((c2 * t + c1) * t + c0, ((d3 * t + d2) * t + d1) * t + 1.0)?;
    if x < 0.0 {
        v = -v;
    }
    Ok(v)
}

fn aknfe(v2: f64) -> Result<f64> {
    if v2 < 5.76 {
        Ok(6.02 + 9.11 * py::sqrt(v2)? - 1.27 * v2)
    } else {
        Ok(12.953 + 4.343 * py::log(v2)?)
    }
}

fn fht(x: f64, pk: f64) -> Result<f64> {
    let mut fhtv;
    if x < 200.0 {
        let w = -py::log(pk)?;
        if pk < 1e-5 || x * py::pow(w, 3.0)? > 5495.0 {
            fhtv = -117.0;
            if x > 1.0 {
                fhtv += 17.372 * py::log(x)?;
            }
        } else {
            fhtv = py::div(2.5e-5 * x * x, pk)? - 8.686 * w - 15.0;
        }
    } else {
        fhtv = 0.05751 * x - 4.343 * py::log(x)?;
        if x < 2000.0 {
            let w = 0.0134 * x * py::exp(-0.005 * x)?;
            fhtv = (1.0 - w) * fhtv + w * (17.372 * py::log(x)? - 117.0);
        }
    }
    Ok(fhtv)
}

fn h0f(r: f64, et: f64) -> Result<f64> {
    const A: [f64; 5] = [25.0, 80.0, 177.0, 395.0, 705.0];
    const B: [f64; 5] = [24.0, 45.0, 68.0, 80.0, 105.0];
    let mut it = py::int(et)?;
    let q;
    if it <= 0 {
        it = 1;
        q = 0.0;
    } else if it >= 5 {
        it = 5;
        q = 0.0;
    } else {
        q = et - it as f64;
    }
    let i = it as usize;
    let x = py::pow(py::div(1.0, r)?, 2.0)?;
    let mut h0fv = 4.343 * py::log((A[i - 1] * x + B[i - 1]) * x + 1.0)?;
    if q != 0.0 {
        h0fv = (1.0 - q) * h0fv + q * 4.343 * py::log((A[i] * x + B[i]) * x + 1.0)?;
    }
    Ok(h0fv)
}

fn ahd(td: f64) -> Result<f64> {
    const A: [f64; 3] = [133.4, 104.6, 71.8];
    const B: [f64; 3] = [0.332e-3, 0.212e-3, 0.157e-3];
    const C: [f64; 3] = [-4.343, -1.086, 2.171];
    let i = if td <= 10E3 {
        0
    } else if td <= 70E3 {
        1
    } else {
        2
    };
    Ok(A[i] + B[i] * td + C[i] * py::log(td)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_what_pyitm_rejects() {
        let call = |f: f64, climate: i32| {
            area(
                2, 90.0, 10.0, 2.0, 10.0, 0, 0, 15.0, 0.005, 301.0, f, climate, 1, 0.5, 0.5, 0.5,
            )
        };
        assert_eq!(call(19.9, 5), Err(Error::InputError { frq_mhz: 19.9 }));
        assert_eq!(
            call(20000.1, 5),
            Err(Error::InputError { frq_mhz: 20000.1 })
        );
        assert_eq!(call(868.0, 0), Err(Error::UnsupportedClimate(0)));
        assert!(call(868.0, 5).is_ok());
        // A zero height ends in a division by zero in Python.
        let zero_height = area(
            2, 90.0, 0.0, 2.0, 10.0, 0, 0, 15.0, 0.005, 301.0, 868.0, 5, 1, 0.5, 0.5, 0.5,
        );
        assert!(zero_height.is_err());
    }
}
