# longley-rice

The ITS Irregular Terrain Model (ITM), better known as the Longley-Rice
model, in **area prediction mode**, in Rust, with no dependencies.

ITM predicts the median and the variability of the basic transmission loss of
a radio path between 20 MHz and 20 GHz over irregular terrain. Area mode needs
no terrain profile: the terrain is described by a single irregularity figure,
Δh, and the horizons are estimated from the antenna heights and how carefully
the sites were chosen. That makes it the mode for coverage estimates, where
all that is known is a distance.

This crate is a translation of the area-mode code of NTIA's reference
implementation, [ITM version 1.4](https://github.com/NTIA/itm), and gives the
same results as that C++: bit for bit on the cases it was checked against (see
[Verification](#verification)). Point-to-point mode is not included.

## Usage

```rust
use longley_rice::{
    AreaParams, Climate, Polarization, SitingCriteria, Variability, VariabilityMode, area_cr,
};

let params = AreaParams {
    tx_height_m: 10.0,
    rx_height_m: 2.0,
    tx_siting: SitingCriteria::Random,
    rx_siting: SitingCriteria::Random,
    distance_km: 50.0,
    delta_h_m: 90.0, // hills
    climate: Climate::ContinentalTemperate,
    n_0: 301.0,
    frequency_mhz: 868.0,
    polarization: Polarization::Vertical,
    epsilon: 15.0, // average ground
    sigma: 0.005,
    variability: Variability::new(VariabilityMode::Mobile),
};

// 50% confidence, 90% reliability.
let loss = area_cr(&params, 50.0, 90.0).expect("valid inputs");
println!("{:.1} dB (warnings: {})", loss.loss_db, loss.warnings);
```

The two entry points are NTIA's:

- `area_tls(&params, time, location, situation)`: the quantiles as time,
  location and situation percentages (`ITM_AREA_TLS_Ex`).
- `area_cr(&params, confidence, reliability)`: as confidence and reliability
  (`ITM_AREA_CR_Ex`), for the mobile and accidental modes of variability.

Both return a `Loss` with the basic transmission loss in dB, NTIA's warning
flags and NTIA's intermediate values (horizons, effective heights, reference
attenuation, free space loss, propagation mode), or an `Error` that mirrors
NTIA's error codes (`Error::code` gives the number). The percentages are
strictly between 0 and 100, as in NTIA's interface. Climate, polarization,
siting and mode of variability are enums, with `TryFrom<i32>` for NTIA's codes.

## Two modes: NTIA and pyitm

`area_tls` and `area_cr` are ITM as NTIA publishes it, and what to use.

`longley_rice::pyitm::area` is something else: a reproduction of the `area()`
function of the Python package [pyitm](https://github.com/tmd224/pyitm) 0.3,
with its exact argument list and meaning (probabilities 0.01 to 0.99 rather
than percentages, integer codes, an error only for a frequency outside 20 to
20000 MHz) and its results, bit for bit. It exists for programs moving off
pyitm that need their numbers not to change, and for measuring how far pyitm
is from ITM. Where pyitm would raise, it returns which Python exception.

```rust
use longley_rice::pyitm;

// itm.area(ModVar=2, deltaH=90.0, tht_m=5.0, rht_m=1.5, dist_km=10.0,
//          TSiteCriteria=0, RSiteCriteria=0, eps_dielect=15.0,
//          sgm_conductivity=0.005, eno_ns_surfref=301, frq_mhz=907.0,
//          radio_climate=5, pol=1, pctTime=0.5, pctLoc=0.5, pctConf=0.5)
match pyitm::area(2, 90.0, 5.0, 1.5, 10.0, 0, 0, 15.0, 0.005, 301.0, 907.0, 5, 1, 0.5, 0.5, 0.5) {
    Ok((loss_db, _errnum)) => println!("{loss_db:.1} dB"),
    Err(pyitm::Error::InputError { .. }) => {} // what pyitm callers usually catch
    Err(other) => panic!("pyitm would have raised: {other}"),
}
```

pyitm is a translation of the older ITM 1.2.2, and it has four bugs of its
own that this mode keeps:

1. In the line-of-sight loss, the phase difference above 1.57 rad is set to
   `3.14 - 2.4649` instead of `3.14 - 2.4649 / q`.
2. The time variability above the median (`sgtp`) is always zero: its factor
   is assigned to `avar.qp` while `avar.gp`, which is used, stays 0.
3. The diffraction-troposcatter transition distance uses `lrprop.xae`, never
   assigned, so 0, where the value computed goes to a local variable.
4. Situation variability is dropped when location variability is (mode of
   variability 10 to 13): `vs0` is tested on `w1` instead of `ws`.

pyitm keeps state between calls in function attributes; every `area()` call
sets all of it afresh except for a radio climate of 0 or less, where it uses
whatever coefficients the previous call left. That case cannot be reproduced
by a function without that history, and returns
`pyitm::Error::UnsupportedClimate`.

### How far pyitm is from ITM

Measured over the call meshview's coverage makes (mobile mode, Δh 90 m, random
siting, ε 15, σ 0.005 S/m, N₀ 301, continental temperate, vertical, 50%
location and confidence), for 433, 868, 869.525, 907, 915 and 2400 MHz,
antenna heights from 1/1 m to 100/10 m, reliabilities from 10% to 99% and
distances from 1 to 200 km every 250 m (172 152 cases):

- Reliability of 50% or more: pyitm's loss is typically 0.001 dB from ITM's
  (99% of cases within 0.01 dB). The largest difference, 0.31 dB, comes from
  bug 1, which needs a large two-ray phase difference, so high antennas and
  a high frequency: of the heights tried, it only shows with 100/10 m at
  2.4 GHz, from 7.5 to 54 km. What is left is the difference between ITM
  1.2.2 and 1.4, below 0.012 dB.
- Reliability below 50%: bug 2 drops the time variability, and pyitm's loss is
  higher than ITM's by 3.5 dB on average and up to 6.6 dB at 10% reliability.
- Bugs 3 and 4 do not change anything on this family of calls.

The coverage radius meshview derives (the last distance at which a 20, 27 or
30 dBm transmitter is received above -120 dBm) is the same with pyitm and with
ITM in 71 of 72 combinations of frequency, heights and 50% or 90% reliability,
and 250 m shorter with ITM in the other.

Over the wider grid of the NTIA fixture, all modes of variability and all
inputs included, pyitm is 3.8 dB from ITM on average and up to 63 dB away.

With the four bugs fixed, pyitm is ITM 1.2.2, and it differs from ITM 1.4 by
at most 0.11 dB on that grid, by NTIA's rewriting of formulas between the two
versions (constants re-derived, natural logarithms becoming decimal ones, the
inverse normal's constant, the ground impedance's `18000 σ / f`, the smooth
earth diffraction recast in Vogler's terms...). Patching those changes into
the fixed pyitm as well brings it to within 3.5e-13 dB of NTIA's C++ on every
case, which shows that the four bugs and the version change account for all
of the difference.

## Verification

The tests compare this crate with the two programs it follows, from fixtures
in `tests/fixtures`:

- `ntia.csv`: 4060 area-mode cases run through NTIA's C++ (`ITM_AREA_TLS_Ex`
  and `ITM_AREA_CR_Ex`), compiled with g++ 16 at `-O2` on x86_64 Linux, with
  losses printed to 17 significant digits. Frequencies from 20 MHz to 20 GHz
  (denser from 400 to 1000 MHz), distances from 0.25 to 2500 km (denser from 1
  to 200 km), heights from 0.5 to 3000 m, Δh from 0 to 800 m, every climate,
  polarization, siting criterion and mode of variability (including +10 and
  +20), TLS and CR percentages from 0.1 to 99.9, and 58 cases with one invalid
  input each. On x86_64 Linux all 4002 losses are bit-identical to NTIA's
  (max difference 0 dB), with the same warning flags and propagation mode, and
  all 58 errors have NTIA's code. The test accepts up to 1e-10 dB, for other
  platforms' maths libraries.
- NTIA's own five area-mode cases (`area.csv` in NTIA's repository), to their
  stated 0.1 dB.
- `pyitm.csv`: the same grid in pyitm's terms, plus inputs only pyitm takes,
  run through pyitm 0.3's `area()` on CPython 3.13: 4064 losses, all
  bit-identical, with the same `errnum`, and 17 exceptions, all matched.
- `pyitm_meshview.csv`: 19 128 calls made the way meshview's coverage makes
  them (six frequencies, two pairs of heights, 50% and 90% reliability, 1 to
  200 km every 250 m), all bit-identical to pyitm.

  As with NTIA's, the pyitm tests accept up to 1e-10 dB, for other platforms'
  maths libraries, and report how many results are bit-identical.

Bit-identical results come from doing what the references do: the same
operations in the same order, every `pow` (squares included) a call to libm's
`pow` as in the compiled C++ and in Python's `math.pow`, glibc's complex square
root and libgcc's complex division for NTIA, and CPython's complex square
root, logarithm and division for pyitm. They depend on the platform's libm
being the same as the reference's; elsewhere expect differences of the order
of 1e-13 dB.

### Regenerating the fixtures

The generator and the driver are in `scripts/`. NTIA's sources include their
headers through Windows paths, so build them from a copy with the includes
fixed:

```sh
git clone https://github.com/NTIA/itm ntia-itm   # commit 183ad95 for v1.4
mkdir ntia-build && cp ntia-itm/src/*.cpp ntia-itm/include/*.h ntia-build/
sed -i 's#"\.\.\\include\\\(.*\)"#"\1"#' ntia-build/*.cpp
sed -i 's#extern "C" __declspec(dllexport)#extern "C"#' ntia-build/itm.h
cd ntia-build
g++ -O2 -std=c++17 -I. ../scripts/ntia_driver.cpp itm_area.cpp \
    InitializePointToPoint.cpp InitializeArea.cpp LongleyRice.cpp \
    LineOfSightLoss.cpp DiffractionLoss.cpp FresnelIntegral.cpp \
    KnifeEdgeDiffraction.cpp SmoothEarthDiffraction.cpp H0Function.cpp \
    TroposcatterLoss.cpp Variability.cpp \
    InverseComplementaryCumulativeDistributionFunction.cpp FreeSpaceLoss.cpp \
    ValidateInputs.cpp SigmaHFunction.cpp TerrainRoughness.cpp -o ntia_driver
cd ..
# With pyitm 0.3 installed in this Python:
python -I scripts/gen_fixtures.py ntia-build/ntia_driver tests/fixtures
```

The cases come from a seeded generator, so this writes the same three files
again.

## Licence

Copyright (C) 2026 Óscar García Amor.

Free software under the GNU General Public License, version 3 or later; see
[LICENSE][license]. The model is derived from NTIA's ITM, which is public
domain in the United States and released worldwide for derivative works; the
pyitm module reproduces pyitm, under the MIT licence. [NOTICE][notice] has
both, with the acknowledgment and the statement of changes NTIA asks for.

[license]: https://github.com/ogarcia/meshview-rs/blob/master/longley-rice/LICENSE
[notice]: https://github.com/ogarcia/meshview-rs/blob/master/longley-rice/NOTICE
