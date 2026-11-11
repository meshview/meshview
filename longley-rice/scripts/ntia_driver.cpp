// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Óscar García Amor <ogarcia@connectical.com>
//
// Runs NTIA ITM v1.4 area-mode cases for the longley-rice test fixtures; see
// gen_fixtures.py and the README. Build it against NTIA's sources with their
// include directory on the include path (-I).
//
// Reads area-mode cases from stdin, one per line:
//   kind,h_tx,h_rx,tx_site,rx_site,d_km,delta_h,climate,N_0,f_mhz,pol,eps,sigma,mdvar,p1,p2,p3
// kind T: p1,p2,p3 = time,location,situation (ITM_AREA_TLS_Ex)
// kind C: p1,p2 = confidence,reliability (ITM_AREA_CR_Ex), p3 ignored
// Writes the line back followed by ,rtn,warnings,A_db,mode with %.17g.
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include "itm.h"

int main() {
    char line[4096];
    while (fgets(line, sizeof line, stdin)) {
        line[strcspn(line, "\r\n")] = 0;
        if (!line[0]) continue;
        char kind = line[0];
        char *p = line + 2;
        double v[16];
        for (int i = 0; i < 16; i++) { v[i] = strtod(p, &p); if (*p == ',') p++; }
        IntermediateValues iv{};
        double A = 0.0 / 0.0;
        long w = 0;
        int rtn;
        if (kind == 'T')
            rtn = ITM_AREA_TLS_Ex(v[0], v[1], (int)v[2], (int)v[3], v[4], v[5], (int)v[6], v[7], v[8], (int)v[9], v[10], v[11], (int)v[12], v[13], v[14], v[15], &A, &w, &iv);
        else
            rtn = ITM_AREA_CR_Ex(v[0], v[1], (int)v[2], (int)v[3], v[4], v[5], (int)v[6], v[7], v[8], (int)v[9], v[10], v[11], (int)v[12], v[13], v[14], &A, &w, &iv);
        if (rtn > 1) { A = 0.0 / 0.0; w = 0; iv.mode = 0; }
        printf("%s,%d,%ld,%.17g,%d\n", line, rtn, w, A, iv.mode);
    }
}
