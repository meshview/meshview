// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The little complex arithmetic the model needs: the ground impedance, a
//! reflection coefficient and the two-ray sum in the line-of-sight region.
//!
//! It is written out here rather than taken from a crate because the exact
//! algorithms matter. The square root and the division are each computed the
//! way the reference they are compared with computes them (glibc for NTIA's
//! C++, CPython for pyitm), so that the results can agree to the last bit and
//! not only to a few ulps.

/// A complex number as a pair of doubles. Nothing more is needed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// A real number taken as complex, which is what both C++ and Python do to
    /// a real operand before adding it to or subtracting a complex from it.
    pub const fn real(re: f64) -> Self {
        Self { re, im: 0.0 }
    }

    pub fn add(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }

    pub fn sub(self, other: Self) -> Self {
        Self::new(self.re - other.re, self.im - other.im)
    }

    /// The textbook product, which is what CPython's `_Py_c_prod` computes
    /// (and what it uses for a complex times a float, the float made complex).
    pub fn mul(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }

    /// A complex times a real, as C++ does it: each part scaled, no cross
    /// terms.
    pub fn scale(self, x: f64) -> Self {
        Self::new(self.re * x, self.im * x)
    }

    /// The squared magnitude.
    pub fn norm_sqr(self) -> f64 {
        self.re * self.re + self.im * self.im
    }

    /// The magnitude, through `hypot` as both glibc's `cabs` and CPython's
    /// `abs()` compute it.
    pub fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }

    /// Smith's division, which is what both libgcc's `__divdc3` and CPython's
    /// `_Py_c_quot` do for operands of ordinary size; the operations and their
    /// order are the same in the two, so one implementation serves both.
    ///
    /// A zero divisor gives NaN parts; the caller that has to raise on it
    /// checks first.
    pub fn div(self, other: Self) -> Self {
        let (a, b, c, d) = (self.re, self.im, other.re, other.im);
        if c.abs() >= d.abs() {
            if c == 0.0 {
                return Self::new(f64::NAN, f64::NAN);
            }
            let ratio = d / c;
            let denom = c + d * ratio;
            Self::new((a + b * ratio) / denom, (b - a * ratio) / denom)
        } else if d.abs() >= c.abs() {
            let ratio = c / d;
            let denom = c * ratio + d;
            Self::new((a * ratio + b) / denom, (b * ratio - a) / denom)
        } else {
            Self::new(f64::NAN, f64::NAN)
        }
    }

    /// The principal square root as glibc's `csqrt` computes it, which is what
    /// `std::sqrt(std::complex<double>)` ends up calling on Linux.
    ///
    /// Only finite arguments of ordinary size follow glibc's path; for the
    /// rest (beyond DBL_MAX / 4 or near the subnormals, where glibc rescales)
    /// CPython's algorithm, which also rescales, is used instead.
    pub fn sqrt_glibc(self) -> Self {
        let (x, y) = (self.re, self.im);
        let safe = |v: f64| v.abs() <= f64::MAX / 4.0;
        if !x.is_finite() || !y.is_finite() || !safe(x) || !safe(y) {
            return self.sqrt_cpython();
        }
        if y == 0.0 {
            return if x < 0.0 {
                Self::new(0.0, (-x).sqrt().copysign(y))
            } else {
                Self::new(x.sqrt().abs(), 0.0_f64.copysign(y))
            };
        }
        if x == 0.0 {
            let r = if y.abs() >= 2.0 * f64::MIN_POSITIVE {
                (0.5 * y.abs()).sqrt()
            } else {
                0.5 * (2.0 * y.abs()).sqrt()
            };
            return Self::new(r, r.copysign(y));
        }
        if x.abs() < 2.0 * f64::MIN_POSITIVE && y.abs() < 2.0 * f64::MIN_POSITIVE {
            return self.sqrt_cpython();
        }
        let d = x.hypot(y);
        let (r, s) = if x > 0.0 {
            let r = (0.5 * (d + x)).sqrt();
            (r, 0.5 * (y / r))
        } else {
            let s = (0.5 * (d - x)).sqrt();
            ((0.5 * (y / s)).abs(), s)
        };
        Self::new(r, s.copysign(y))
    }

    /// The principal square root as CPython's `cmath.sqrt` computes it
    /// (`cmath_sqrt_impl`), for finite arguments.
    pub fn sqrt_cpython(self) -> Self {
        // CPython's CM_SCALE_UP and CM_SCALE_DOWN: ldexp(x, 53) and ldexp(x, -27).
        const SCALE_UP: f64 = 9007199254740992.0;
        const SCALE_DOWN: f64 = 1.0 / 134217728.0;
        let (x, y) = (self.re, self.im);
        if x == 0.0 && y == 0.0 {
            return Self::new(0.0, y);
        }
        let mut ax = x.abs();
        let ay = y.abs();
        let s = if ax < f64::MIN_POSITIVE && ay < f64::MIN_POSITIVE {
            ax *= SCALE_UP;
            (ax + ax.hypot(ay * SCALE_UP)).sqrt() * SCALE_DOWN
        } else {
            ax /= 8.0;
            2.0 * (ax + ax.hypot(ay / 8.0)).sqrt()
        };
        let d = ay / (2.0 * s);
        if x >= 0.0 {
            Self::new(s, d.copysign(y))
        } else {
            Self::new(d, s.copysign(y))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_roots_square_back() {
        for z in [
            Complex::new(14.0, 1.2),
            Complex::new(-3.0, 4.0),
            Complex::new(0.0, 2.0),
            Complex::new(5.0, -0.25),
        ] {
            for root in [z.sqrt_glibc(), z.sqrt_cpython()] {
                let back = root.mul(root);
                assert!((back.re - z.re).abs() < 1e-12 && (back.im - z.im).abs() < 1e-12);
                assert!(root.re >= 0.0);
            }
        }
    }

    #[test]
    fn division_inverts_multiplication() {
        let a = Complex::new(3.0, -2.0);
        let b = Complex::new(0.5, 7.0);
        let q = a.mul(b).div(b);
        assert!((q.re - a.re).abs() < 1e-14 && (q.im - a.im).abs() < 1e-14);
        assert!(a.div(Complex::new(0.0, 0.0)).re.is_nan());
    }
}
