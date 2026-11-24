"""Numerical checks of the derivations in docs/optimisation (appendix C).

    python3 docs/optimisation/example/derivations.py

The curvature of a line shifted sideways from a seed line. The seed has curvature k0(s); the
shifted line is r(s) = r0(s) + d(s) N(s), with N to the left. The formula under test:

    k = [ a (k0 a + d'') + d' (k0' d + 2 k0 d') ] / (a^2 + d'^2)^(3/2),   a = 1 - k0 d.

The check builds a seed whose curvature varies, shifts it by a d(s) that varies, and compares the
formula with the curvature read from the shifted points themselves by finite differences.
"""
import math


def curvature_check(length=1200.0, h=0.05):
    k0 = lambda s: 1.0 / 400.0 + math.sin(s / 150.0) / 2000.0
    dk0 = lambda s: math.cos(s / 150.0) / (2000.0 * 150.0)
    d = lambda s: 20.0 * math.sin(s / 120.0)
    d1 = lambda s: 20.0 / 120.0 * math.cos(s / 120.0)
    d2 = lambda s: -20.0 / 120.0**2 * math.sin(s / 120.0)
    # The seed, by the midpoint rule on steps of h: heading theta' = k0, then x' = cos, y' = sin.
    n = int(round(length / h))
    th, x, y = 0.0, 0.0, 0.0
    pts = []
    for i in range(n + 1):
        s = i * h
        pts.append((x - d(s) * math.sin(th), y + d(s) * math.cos(th)))
        th_mid = th + 0.5 * h * k0(s + 0.25 * h)
        x += h * math.cos(th_mid)
        y += h * math.sin(th_mid)
        th += h * k0(s + 0.5 * h)
    worst, k_lo, k_hi = 0.0, float("inf"), -float("inf")
    m = 40  # finite differences over 2 m, so rounding does not swamp the second difference
    for i in range(m, n + 1 - m, 200):
        s = i * h
        (xa, ya), (xb, yb), (xc, yc) = pts[i - m], pts[i], pts[i + m]
        hh = m * h
        x1, y1 = (xc - xa) / (2 * hh), (yc - ya) / (2 * hh)
        x2, y2 = (xc - 2 * xb + xa) / hh**2, (yc - 2 * yb + ya) / hh**2
        numeric = (x1 * y2 - y1 * x2) / (x1 * x1 + y1 * y1) ** 1.5
        a = 1.0 - k0(s) * d(s)
        formula = (a * (k0(s) * a + d2(s)) + d1(s) * (dk0(s) * d(s) + 2.0 * k0(s) * d1(s))) / (a * a + d1(s) ** 2) ** 1.5
        worst = max(worst, abs(numeric - formula))
        k_lo, k_hi = min(k_lo, formula), max(k_hi, formula)
    return {
        "worst_difference_per_m": worst,
        "k_min_per_m": k_lo,
        "k_max_per_m": k_hi,
        "tightest_radius_m": 1.0 / max(abs(k_lo), abs(k_hi)),
        "seed_tightest_radius_m": 1.0 / (1.0 / 400.0 + 1.0 / 2000.0),
        "worst_relative": worst / max(abs(k_lo), abs(k_hi)),
    }


if __name__ == "__main__":
    for k, v in curvature_check().items():
        print(f"{k}: {v:.6g}")
