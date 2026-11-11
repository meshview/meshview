// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

//! The pyitm reproduction against pyitm 0.3 itself.
//!
//! The fixtures hold what pyitm's `area()` returned, as `repr()` of the
//! float, which parses back to the very same `f64`. The README says how they
//! were made. With the maths library they were made with (glibc on x86_64)
//! every loss is the same to the bit; another one, or another version of it,
//! rounds an `exp` or a `pow` a last bit differently now and then, so what is
//! required is the same to within [`TOLERANCE_DB`], and how many are
//! identical is reported.

use longley_rice::pyitm::{self, Error};

/// How far a loss may be from pyitm's, in dB: a few last bits of an `f64`.
const TOLERANCE_DB: f64 = 1e-10;

fn expected_error(name: &str) -> Error {
    match name {
        "ValueError" => Error::ValueError,
        "ZeroDivisionError" => Error::ZeroDivisionError,
        "OverflowError" => Error::OverflowError,
        other => panic!("unexpected exception {other}"),
    }
}

#[test]
fn matches_pyitm_on_the_grid() {
    let (mut losses, mut errors, mut identical) = (0, 0, 0);
    let mut max_diff: f64 = 0.0;
    for (i, line) in include_str!("fixtures/pyitm.csv").lines().enumerate() {
        if line.starts_with('#') || line.starts_with("mod_var") {
            continue;
        }
        let v: Vec<&str> = line.split(',').collect();
        assert_eq!(v.len(), 18, "line {}", i + 1);
        let f = |i: usize| v[i].parse::<f64>().unwrap();
        let n = |i: usize| v[i].parse::<i32>().unwrap();
        let got = pyitm::area(
            n(0),
            f(1),
            f(2),
            f(3),
            f(4),
            n(5),
            n(6),
            f(7),
            f(8),
            f(9),
            f(10),
            n(11),
            n(12),
            f(13),
            f(14),
            f(15),
        );
        match v[16] {
            "InputError" => {
                assert_eq!(
                    got,
                    Err(Error::InputError { frq_mhz: f(10) }),
                    "line {}",
                    i + 1
                );
                errors += 1;
            }
            name if name.starts_with(|c: char| c.is_ascii_alphabetic()) => {
                assert_eq!(got, Err(expected_error(name)), "line {}", i + 1);
                errors += 1;
            }
            loss => {
                let (db, errnum) = got.unwrap_or_else(|e| panic!("line {}: {e:?}", i + 1));
                let expected = loss.parse::<f64>().unwrap();
                let diff = (db - expected).abs();
                assert!(diff <= TOLERANCE_DB, "line {}: {db} vs {expected}", i + 1);
                max_diff = max_diff.max(diff);
                if db.to_bits() == expected.to_bits() {
                    identical += 1;
                }
                assert_eq!(errnum, n(17), "line {}", i + 1);
                losses += 1;
            }
        }
    }
    println!(
        "{losses} losses ({identical} bit-identical to pyitm, max |diff| {max_diff:e} dB), \
         {errors} exceptions matched"
    );
    assert!(losses > 4000);
}

/// The calls meshview's coverage makes, over the whole distance range it
/// steps through; the order of the loops is the fixture's.
#[test]
fn matches_pyitm_as_meshview_calls_it() {
    let mut expected = include_str!("fixtures/pyitm_meshview.csv")
        .lines()
        .filter(|line| !line.starts_with('#'));
    let (mut count, mut identical) = (0, 0);
    let mut max_diff: f64 = 0.0;
    for frq in [433.0, 868.0, 869.525, 907.0, 915.0, 2400.0] {
        for (tht, rht) in [(5.0, 1.5), (10.0, 2.0)] {
            for rel in [0.5, 0.9] {
                for i in 0..=796 {
                    let d = 1.0 + 0.25 * i as f64;
                    let (db, errnum) = pyitm::area(
                        2, 90.0, tht, rht, d, 0, 0, 15.0, 0.005, 301.0, frq, 5, 1, rel, 0.5, 0.5,
                    )
                    .unwrap();
                    let line = expected.next().expect("fixture too short");
                    let (loss, code) = line.split_once(',').unwrap();
                    let loss = loss.parse::<f64>().unwrap();
                    let diff = (db - loss).abs();
                    assert!(
                        diff <= TOLERANCE_DB,
                        "f {frq} h {tht}/{rht} rel {rel} d {d}: {db} vs {loss}"
                    );
                    max_diff = max_diff.max(diff);
                    if db.to_bits() == loss.to_bits() {
                        identical += 1;
                    }
                    assert_eq!(errnum, code.parse::<i32>().unwrap());
                    count += 1;
                }
            }
        }
    }
    assert!(expected.next().is_none(), "fixture too long");
    println!(
        "{count} meshview calls ({identical} bit-identical to pyitm, max |diff| {max_diff:e} dB)"
    );
}
