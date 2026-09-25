//! Low-precision Sun and Moon positions.
//!
//! Moon: truncated Meeus, *Astronomical Algorithms* ch. 47 (~0.05° in
//! longitude), with topocentric parallax and atmospheric refraction applied to
//! the altitude. Sun: the classic almanac formula (~0.01°). Plenty for
//! planning where to stand; not for occultation timing.

use std::f64::consts::PI;

const DEG: f64 = PI / 180.0;
const EARTH_RADIUS_KM: f64 = 6378.14;
const MOON_RADIUS_KM: f64 = 1737.4;
const AU_KM: f64 = 149_597_870.7;

/// Horizontal position of a body as seen by an observer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Horizontal {
    /// Degrees clockwise from true north, 0..360.
    pub azimuth: f64,
    /// Degrees above the horizon, topocentric and refracted.
    pub altitude: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct MoonState {
    pub pos: Horizontal,
    /// Illuminated fraction of the disk, 0..1.
    pub illumination: f64,
    pub waxing: bool,
    /// Apparent angular diameter in degrees.
    pub diameter: f64,
}

struct Equatorial {
    ra: f64,
    dec: f64,
    dist_km: f64,
    ecl_lon: f64,
}

pub fn julian_day(unix_ms: f64) -> f64 {
    unix_ms / 86_400_000.0 + 2_440_587.5
}

fn norm360(x: f64) -> f64 {
    x.rem_euclid(360.0)
}

fn sin_d(x: f64) -> f64 {
    (x * DEG).sin()
}

fn cos_d(x: f64) -> f64 {
    (x * DEG).cos()
}

fn obliquity(t: f64) -> f64 {
    23.439_291 - 0.013_004_2 * t
}

fn ecliptic_to_equatorial(lon: f64, lat: f64, eps: f64) -> (f64, f64) {
    let ra = (sin_d(lon) * cos_d(eps) - (lat * DEG).tan() * sin_d(eps)).atan2(cos_d(lon)) / DEG;
    let dec = (sin_d(lat) * cos_d(eps) + cos_d(lat) * sin_d(eps) * sin_d(lon)).asin() / DEG;
    (norm360(ra), dec)
}

// Periodic terms from Meeus table 47.A: D, M, M', F, Σl (1e-6°), Σr (1e-3 km).
#[rustfmt::skip]
const LON_DIST: [(i8, i8, i8, i8, f64, f64); 32] = [
    (0, 0, 1, 0, 6288774.0, -20905355.0), (2, 0, -1, 0, 1274027.0, -3699111.0),
    (2, 0, 0, 0, 658314.0, -2955968.0),   (0, 0, 2, 0, 213618.0, -569925.0),
    (0, 1, 0, 0, -185116.0, 48888.0),     (0, 0, 0, 2, -114332.0, -3149.0),
    (2, 0, -2, 0, 58793.0, 246158.0),     (2, -1, -1, 0, 57066.0, -152138.0),
    (2, 0, 1, 0, 53322.0, -170733.0),     (2, -1, 0, 0, 45758.0, -204586.0),
    (0, 1, -1, 0, -40923.0, -129620.0),   (1, 0, 0, 0, -34720.0, 108743.0),
    (0, 1, 1, 0, -30383.0, 104755.0),     (2, 0, 0, -2, 15327.0, 10321.0),
    (0, 0, 1, 2, -12528.0, 0.0),          (0, 0, 1, -2, 10980.0, 79661.0),
    (4, 0, -1, 0, 10675.0, -34782.0),     (0, 0, 3, 0, 10034.0, -23210.0),
    (4, 0, -2, 0, 8548.0, -21636.0),      (2, 1, -1, 0, -7888.0, 24208.0),
    (2, 1, 0, 0, -6766.0, 30824.0),       (1, 0, -1, 0, -5163.0, -8379.0),
    (1, 1, 0, 0, 4987.0, -16675.0),       (2, -1, 1, 0, 4036.0, -12831.0),
    (2, 0, 2, 0, 3994.0, -10445.0),       (4, 0, 0, 0, 3861.0, -11650.0),
    (2, 0, -3, 0, 3665.0, 14403.0),       (0, 1, -2, 0, -2689.0, -7003.0),
    (2, 0, -1, 2, -2602.0, 0.0),          (2, -1, -2, 0, 2390.0, 10056.0),
    (1, 0, 1, 0, -2348.0, 6322.0),        (2, -2, 0, 0, 2236.0, -9884.0),
];

// Meeus table 47.B: D, M, M', F, Σb (1e-6°).
#[rustfmt::skip]
const LAT: [(i8, i8, i8, i8, f64); 20] = [
    (0, 0, 0, 1, 5128122.0), (0, 0, 1, 1, 280602.0),  (0, 0, 1, -1, 277693.0),
    (2, 0, 0, -1, 173237.0), (2, 0, -1, 1, 55413.0),  (2, 0, -1, -1, 46271.0),
    (2, 0, 0, 1, 32573.0),   (0, 0, 2, 1, 17198.0),   (2, 0, 1, -1, 9266.0),
    (0, 0, 2, -1, 8822.0),   (2, -1, 0, -1, 8216.0),  (2, 0, -2, -1, 4324.0),
    (2, 0, 1, 1, 4200.0),    (2, 1, 0, -1, -3359.0),  (2, -1, -1, 1, 2463.0),
    (2, -1, 0, 1, 2211.0),   (2, -1, -1, -1, 2065.0), (0, 1, -1, -1, -1870.0),
    (4, 0, -1, -1, 1828.0),  (0, 1, 0, 1, -1794.0),
];

fn moon_equatorial(jd: f64) -> Equatorial {
    let t = (jd - 2_451_545.0) / 36_525.0;
    let lp = norm360(218.316_447_7 + 481_267.881_234_21 * t);
    let d = norm360(297.850_192_1 + 445_267.111_403_4 * t);
    let m = norm360(357.529_109_2 + 35_999.050_290_9 * t);
    let mp = norm360(134.963_396_4 + 477_198.867_505_5 * t);
    let f = norm360(93.272_095_0 + 483_202.017_523_3 * t);
    let e = 1.0 - 0.002_516 * t;
    let ecc = |mi: i8| match mi.abs() {
        1 => e,
        2 => e * e,
        _ => 1.0,
    };
    let arg = |a: i8, b: i8, c: i8, g: i8| {
        a as f64 * d + b as f64 * m + c as f64 * mp + g as f64 * f
    };

    let (mut sl, mut sr) = (0.0, 0.0);
    for &(a, b, c, g, l, r) in &LON_DIST {
        let x = arg(a, b, c, g);
        sl += l * ecc(b) * sin_d(x);
        sr += r * ecc(b) * cos_d(x);
    }
    let mut sb = 0.0;
    for &(a, b, c, g, v) in &LAT {
        sb += v * ecc(b) * sin_d(arg(a, b, c, g));
    }

    let a1 = 119.75 + 131.849 * t;
    let a2 = 53.09 + 479_264.290 * t;
    let a3 = 313.45 + 481_266.484 * t;
    sl += 3958.0 * sin_d(a1) + 1962.0 * sin_d(lp - f) + 318.0 * sin_d(a2);
    sb += -2235.0 * sin_d(lp) + 382.0 * sin_d(a3) + 175.0 * sin_d(a1 - f)
        + 175.0 * sin_d(a1 + f) + 127.0 * sin_d(lp - mp) - 115.0 * sin_d(lp + mp);

    // Nutation in longitude, dominant term only.
    let omega = 125.044_52 - 1_934.136_261 * t;
    let lon = norm360(lp + sl / 1e6 - 0.004_78 * sin_d(omega));
    let lat = sb / 1e6;
    let (ra, dec) = ecliptic_to_equatorial(lon, lat, obliquity(t));
    Equatorial { ra, dec, dist_km: 385_000.56 + sr / 1000.0, ecl_lon: lon }
}

fn sun_equatorial(jd: f64) -> Equatorial {
    let n = jd - 2_451_545.0;
    let g = norm360(357.529 + 0.985_600_28 * n);
    let q = norm360(280.459 + 0.985_647_36 * n);
    let lon = norm360(q + 1.915 * sin_d(g) + 0.020 * sin_d(2.0 * g));
    let r_au = 1.000_14 - 0.016_71 * cos_d(g) - 0.000_14 * cos_d(2.0 * g);
    let (ra, dec) = ecliptic_to_equatorial(lon, 0.0, obliquity(n / 36_525.0));
    Equatorial { ra, dec, dist_km: r_au * AU_KM, ecl_lon: lon }
}

fn gmst_deg(jd: f64) -> f64 {
    let d = jd - 2_451_545.0;
    let t = d / 36_525.0;
    norm360(280.460_618_37 + 360.985_647_366_29 * d + 0.000_387_933 * t * t)
}

/// Geocentric equatorial -> (azimuth, geometric altitude) for an observer.
fn to_horizontal(eq: &Equatorial, jd: f64, lat: f64, lon: f64) -> (f64, f64) {
    let h = gmst_deg(jd) + lon - eq.ra;
    let alt = (sin_d(lat) * sin_d(eq.dec) + cos_d(lat) * cos_d(eq.dec) * cos_d(h)).asin() / DEG;
    let az = sin_d(h).atan2(cos_d(h) * sin_d(lat) - (eq.dec * DEG).tan() * cos_d(lat)) / DEG;
    (norm360(az + 180.0), alt)
}

/// Bennett's refraction formula, degrees to add to a geometric altitude.
fn refraction(alt: f64) -> f64 {
    if alt < -1.5 {
        return 0.0;
    }
    1.02 / ((alt + 10.3 / (alt + 5.11)) * DEG).tan() / 60.0
}

pub fn moon(unix_ms: f64, lat: f64, lon: f64) -> MoonState {
    let jd = julian_day(unix_ms);
    let m = moon_equatorial(jd);
    let (az, geo_alt) = to_horizontal(&m, jd, lat, lon);
    let parallax = (EARTH_RADIUS_KM / m.dist_km).asin() / DEG;
    let topo_alt = geo_alt - parallax * cos_d(geo_alt);
    let altitude = topo_alt + refraction(topo_alt);

    let s = sun_equatorial(jd);
    let cos_elong = sin_d(s.dec) * sin_d(m.dec) + cos_d(s.dec) * cos_d(m.dec) * cos_d(s.ra - m.ra);
    let elong = cos_elong.clamp(-1.0, 1.0).acos();
    let phase_angle = (s.dist_km * elong.sin()).atan2(m.dist_km - s.dist_km * elong.cos());

    MoonState {
        pos: Horizontal { azimuth: az, altitude },
        illumination: (1.0 + phase_angle.cos()) / 2.0,
        waxing: norm360(m.ecl_lon - s.ecl_lon) < 180.0,
        diameter: 2.0 * (MOON_RADIUS_KM / m.dist_km).asin() / DEG,
    }
}

pub fn sun(unix_ms: f64, lat: f64, lon: f64) -> Horizontal {
    let jd = julian_day(unix_ms);
    let (azimuth, alt) = to_horizontal(&sun_equatorial(jd), jd, lat, lon);
    Horizontal { azimuth, altitude: alt + refraction(alt) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crossing {
    Rise,
    Set,
}

/// Times (unix ms) in `[start, end)` where `altitude(t)` crosses `threshold`.
pub fn crossings(
    start: f64,
    end: f64,
    threshold: f64,
    altitude: impl Fn(f64) -> f64,
) -> Vec<(Crossing, f64)> {
    const STEP: f64 = 10.0 * 60_000.0;
    let mut out = Vec::new();
    let mut t0 = start;
    let mut a0 = altitude(t0) - threshold;
    while t0 < end {
        let t1 = (t0 + STEP).min(end);
        let a1 = altitude(t1) - threshold;
        if (a0 < 0.0) != (a1 < 0.0) {
            let (mut lo, mut hi) = (t0, t1);
            while hi - lo > 1000.0 {
                let mid = (lo + hi) / 2.0;
                if (altitude(mid) - threshold < 0.0) == (a0 < 0.0) {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let kind = if a0 < 0.0 { Crossing::Rise } else { Crossing::Set };
            out.push((kind, (lo + hi) / 2.0));
        }
        t0 = t1;
        a0 = a1;
    }
    out
}

/// Moonrise/set is defined by the upper limb touching the horizon.
pub const MOON_HORIZON: f64 = -0.27;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn meeus_example_47a() {
        // 1992 April 12, 0h TD: λ = 133.162655°, β = -3.229126°, Δ = 368409.7 km.
        let jd = 2_448_724.5;
        let m = moon_equatorial(jd);
        assert!((m.ecl_lon - 133.167).abs() < 0.05, "lon {}", m.ecl_lon);
        assert!((m.dist_km - 368_409.7).abs() < 50.0, "dist {}", m.dist_km);
        // Apparent RA 134.688470°, Dec 13.768368°.
        assert!((m.ra - 134.688).abs() < 0.05, "ra {}", m.ra);
        assert!((m.dec - 13.768).abs() < 0.05, "dec {}", m.dec);
    }

    #[test]
    fn full_moon_is_full() {
        // Full moon 2024-09-18 02:34 UTC.
        let s = moon(1_726_626_840_000.0, 45.37, -121.70);
        assert!(s.illumination > 0.99, "illum {}", s.illumination);
    }

    #[test]
    fn finds_one_rise_and_set_per_day_or_so() {
        let start = 1_726_642_800_000.0; // 2024-09-18 00:00 PDT
        let c = crossings(start, start + 86_400_000.0, MOON_HORIZON, |t| {
            moon(t, 45.37, -121.70).pos.altitude
        });
        assert!(!c.is_empty() && c.len() <= 2, "{c:?}");
    }
}
