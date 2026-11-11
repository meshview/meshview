# SPDX-License-Identifier: GPL-3.0-or-later
# Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>

"""Generate the longley-rice test fixtures.

Usage:

    python -I scripts/gen_fixtures.py NTIA_DRIVER tests/fixtures

NTIA_DRIVER is scripts/ntia_driver.cpp compiled with NTIA's ITM v1.4 sources
(see the README). pyitm 0.3 must be importable by the Python running this.
Writes ntia.csv, pyitm.csv and pyitm_meshview.csv into the output directory.
The cases come from a seeded generator, so the same inputs come out every time.
"""

import random
import subprocess
import sys

from pyitm import itm

SEED = 20261111


def r(x, nd):
    """Round so that the CSV holds short decimals."""
    return float(round(x, nd))


def fmt(x):
    return repr(x) if isinstance(x, float) else str(x)


def pick_freq(rng):
    u = rng.random()
    if u < 0.4:
        return r(10 ** rng.uniform(1.30103, 4.30103), 3)  # 20 MHz .. 20 GHz, log uniform
    if u < 0.8:
        return r(rng.uniform(400, 1000), 3)
    return rng.choice([20.0, 40.0, 433.0, 868.0, 869.525, 907.0, 915.0, 2400.0, 10000.0, 20000.0])


def pick_dist(rng):
    u = rng.random()
    if u < 0.6:
        return r(rng.uniform(1, 200), 2)
    if u < 0.9:
        return r(10 ** rng.uniform(0, 3.30103), 2)  # 1 .. 2000 km
    return rng.choice([0.25, 0.5, 1.0, 2000.0, 2500.0])


def pick_height(rng):
    if rng.random() < 0.5:
        return rng.choice(
            [0.5, 1.0, 1.5, 2.0, 3.0, 5.0, 10.0, 15.0, 30.0, 50.0, 100.0, 300.0, 1000.0, 3000.0]
        )
    return r(10 ** rng.uniform(-0.3, 2.5), 1) or 0.5


def pick_pct(rng):
    if rng.random() < 0.4:
        return rng.choice([0.1, 1.0, 5.0, 10.0, 50.0, 90.0, 95.0, 99.0, 99.9])
    return r(rng.uniform(0.5, 99.5), 2)


MDVARS = [0, 1, 2, 3] * 4 + [10, 11, 12, 13, 20, 21, 22, 23, 30, 31, 32, 33]


def valid_case(rng):
    return {
        "kind": rng.choice("TC"),
        "h_tx": pick_height(rng),
        "h_rx": pick_height(rng),
        "tx_site": rng.choice([0, 0, 1, 2]),
        "rx_site": rng.choice([0, 0, 1, 2]),
        "d_km": pick_dist(rng),
        "delta_h": rng.choice([0.0, 10.0, 30.0, 90.0, 200.0, 500.0])
        if rng.random() < 0.5
        else r(rng.uniform(0, 800), 1),
        "climate": rng.randint(1, 7),
        "n_0": rng.choice([250.0, 280.0, 301.0, 330.0, 360.0, 400.0])
        if rng.random() < 0.6
        else r(rng.uniform(250, 400), 1),
        "f_mhz": pick_freq(rng),
        "pol": rng.randint(0, 1),
        "epsilon": rng.choice([4.0, 15.0, 25.0, 81.0])
        if rng.random() < 0.6
        else r(rng.uniform(1.5, 100), 2),
        "sigma": rng.choice([0.001, 0.005, 0.01, 0.03, 5.0])
        if rng.random() < 0.6
        else r(10 ** rng.uniform(-4, 0.7), 5),
        "mdvar": rng.choice(MDVARS),
        "p1": pick_pct(rng),
        "p2": pick_pct(rng),
        "p3": pick_pct(rng),
    }


# One invalid input per case, so that the error does not depend on the order
# of the checks.
BAD = [
    ("h_tx", 0.4),
    ("h_tx", 3001.0),
    ("h_rx", 0.4),
    ("h_rx", 3001.0),
    ("climate", 0),
    ("climate", 8),
    ("n_0", 249.0),
    ("n_0", 401.0),
    ("f_mhz", 19.9),
    ("f_mhz", 20001.0),
    ("pol", 2),
    ("epsilon", 0.5),
    ("sigma", 0.0),
    ("sigma", -1.0),
    ("mdvar", -1),
    ("mdvar", 4),
    ("mdvar", 14),
    ("mdvar", 34),
    ("p1", 0.0),
    ("p1", 100.0),
    ("p2", 0.0),
    ("p2", 100.0),
    ("p3", 0.0),
    ("p3", 100.0),
    ("d_km", 0.0),
    ("d_km", -1.0),
    ("delta_h", -1.0),
    ("tx_site", 3),
    ("rx_site", -1),
    ("epsilon", 1.0),
]

COLS = [
    "kind",
    "h_tx",
    "h_rx",
    "tx_site",
    "rx_site",
    "d_km",
    "delta_h",
    "climate",
    "n_0",
    "f_mhz",
    "pol",
    "epsilon",
    "sigma",
    "mdvar",
    "p1",
    "p2",
    "p3",
]


def grid():
    rng = random.Random(SEED)
    cases = [valid_case(rng) for _ in range(4000)]
    for field, value in BAD:
        for kind in "TC":
            c = valid_case(rng)
            c["kind"] = kind
            c[field] = value
            if field == "epsilon" and value == 1.0:
                c["pol"] = 0  # horizontal: the ground impedance check fails
            cases.append(c)
    return cases


def run_ntia(driver, cases):
    text = "".join(",".join(fmt(c[k]) for k in COLS) + "\n" for c in cases)
    out = subprocess.run([driver], input=text, capture_output=True, text=True, check=True)
    rows = []
    for line in out.stdout.splitlines():
        parts = line.split(",")
        rows.append(parts)
    assert len(rows) == len(cases)
    return rows


def pyitm_call(args):
    try:
        loss, errnum = itm.area(*args)
        return repr(float(loss)), str(errnum)
    except itm.InputError:
        return "InputError", ""
    except Exception as e:  # noqa: BLE001 - the exception class is the result
        return type(e).__name__, ""


def pyitm_args(c):
    """The NTIA case in pyitm's terms: fractions instead of percentages."""
    if c["kind"] == "T":
        pt, pl, pc = c["p1"] / 100, c["p2"] / 100, c["p3"] / 100
    else:  # CR: reliability as time, 50% location, confidence as situation
        pt, pl, pc = c["p2"] / 100, 0.5, c["p1"] / 100
    return [
        c["mdvar"],
        c["delta_h"],
        c["h_tx"],
        c["h_rx"],
        c["d_km"],
        c["tx_site"],
        c["rx_site"],
        c["epsilon"],
        c["sigma"],
        c["n_0"],
        c["f_mhz"],
        c["climate"],
        c["pol"],
        pt,
        pl,
        pc,
    ]


def extra_pyitm_cases():
    """Inputs only pyitm takes, to see its own handling of them."""
    base = [2, 90.0, 5.0, 1.5, 10.0, 0, 0, 15.0, 0.005, 301.0, 907.0, 5, 1, 0.5, 0.5, 0.5]
    out = []
    for idx, value in [
        (2, 0.0),
        (3, 0.0),
        (4, 0.0),
        (4, -5.0),
        (1, -10.0),
        (1, 0.0),
        (10, 19.0),
        (10, 20001.0),
        (12, 2),
        (5, 5),
        (6, -1),
        (0, 7),
        (0, -1),
        (11, 9),
        (13, 0.0),
        (13, 1.0),
        (15, 0.999),
        (8, 0.0),
        (7, 1.0),
        (9, 200.0),
        (2, 4000.0),
        (4, 3000.0),
        (2, 0.2),
    ]:
        args = list(base)
        args[idx] = value
        out.append(args)
    return out


def main():
    driver, out_dir = sys.argv[1], sys.argv[2]
    cases = grid()
    rows = run_ntia(driver, cases)

    with open(f"{out_dir}/ntia.csv", "w") as f:
        f.write("# NTIA ITM v1.4 area mode (ITM_AREA_TLS_Ex for kind T, ITM_AREA_CR_Ex for C),\n")
        f.write("# compiled with g++ -O2 on x86_64 Linux. For C, p1 = confidence, p2 =\n")
        f.write("# reliability and p3 is not used. loss_db and mode are meaningful only when\n")
        f.write("# rtn is 0 or 1; otherwise rtn is the error code. Generated by gen_fixtures.py.\n")
        f.write(",".join(COLS + ["rtn", "warnings", "loss_db", "mode"]) + "\n")
        f.writelines(",".join(parts) + "\n" for parts in rows)

    with open(f"{out_dir}/pyitm.csv", "w") as f:
        f.write("# pyitm 0.3 itm.area(), with the arguments in its order. result is repr() of\n")
        f.write("# the loss, or the name of the exception raised. Cases with radio_climate <= 0\n")
        f.write("# are left out (pyitm's result then depends on its previous call). Generated by\n")
        f.write("# gen_fixtures.py from the same grid as ntia.csv, plus pyitm-only inputs.\n")
        f.write(
            "mod_var,delta_h,tht_m,rht_m,dist_km,t_site,r_site,eps,sgm,eno,frq,climate,pol,"
            "pct_time,pct_loc,pct_conf,result,errnum\n"
        )
        for args in [pyitm_args(c) for c in cases] + extra_pyitm_cases():
            if args[11] <= 0:
                continue
            res, errnum = pyitm_call(args)
            f.write(",".join(fmt(a) for a in args) + f",{res},{errnum}\n")

    with open(f"{out_dir}/pyitm_meshview.csv", "w") as f:
        f.write("# pyitm 0.3 itm.area() as meshview's coverage calls it: ModVar=2, deltaH=90,\n")
        f.write(
            "# siting 0/0, eps 15, sigma 0.005, N 301, climate 5, vertical, pctLoc=pctConf=0.5.\n"
        )
        f.write("# One loss per line (repr), errnum after it, for every combination, in this\n")
        f.write("# order of nesting (outermost first):\n")
        f.write("#   frq_mhz in 433, 868, 869.525, 907, 915, 2400\n")
        f.write("#   (tht_m, rht_m) in (5, 1.5), (10, 2)\n")
        f.write("#   pct_time in 0.5, 0.9\n")
        f.write("#   dist_km = 1 + 0.25 * i for i in 0..=796 (1 to 200 km)\n")
        for frq in MESHVIEW_FREQS:
            for tht, rht in MESHVIEW_HEIGHTS:
                for rel in MESHVIEW_RELS:
                    for i in range(797):
                        d = 1 + 0.25 * i
                        loss, errnum = itm.area(
                            ModVar=2,
                            deltaH=90.0,
                            tht_m=tht,
                            rht_m=rht,
                            dist_km=d,
                            TSiteCriteria=0,
                            RSiteCriteria=0,
                            eps_dielect=15.0,
                            sgm_conductivity=0.005,
                            eno_ns_surfref=301,
                            frq_mhz=frq,
                            radio_climate=5,
                            pol=1,
                            pctTime=rel,
                            pctLoc=0.5,
                            pctConf=0.5,
                        )
                        f.write(f"{loss!r},{errnum}\n")


MESHVIEW_FREQS = [433.0, 868.0, 869.525, 907.0, 915.0, 2400.0]
MESHVIEW_HEIGHTS = [(5.0, 1.5), (10.0, 2.0)]
MESHVIEW_RELS = [0.5, 0.9]

if __name__ == "__main__":
    main()
