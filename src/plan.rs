//! Turns moon positions into places to stand, and those into GeoJSON.
//!
//! For each moment the moon is up we look *back* along its azimuth from the
//! landmark and solve for the distance at which the landmark's summit sits at
//! the moon's apparent altitude. Standing there puts the moon on the summit.

use crate::astro::{self, Crossing, MoonState};
use crate::terrain::{Ground, TileId};
use serde_json::{Value, json};
use std::collections::BTreeSet;

const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// Standard terrestrial refraction coefficient.
const REFRACTION_K: f64 = 0.13;
/// Beyond this the summit is lost in haze and curvature anyway.
pub const MAX_STAND_KM: f64 = 250.0;
const MOON_RAY_KM: f64 = 60.0;
const SAMPLE_MINUTES: f64 = 5.0;
/// Camera height above the ground.
pub const EYE_HEIGHT_M: f64 = 1.7;
/// Ground assumed where terrain isn't available (offline, not yet loaded).
pub const FALLBACK_GROUND_M: f64 = 500.0;

#[derive(Clone, Debug, PartialEq)]
pub struct Landmark {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    /// Summit elevation, metres above sea level.
    pub elevation_m: f64,
    /// IANA time zone used for the local day and labels.
    pub tz: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    /// Moon's centre directly behind the summit.
    Center,
    /// Moon's lower limb resting on the summit.
    Resting,
}

/// Where the observer's eye is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Observer {
    /// On the ground at each standing spot, looked up from terrain.
    Terrain,
    /// A fixed elevation (metres above sea level), e.g. on a tower.
    Fixed(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub landmark: Landmark,
    pub observer: Observer,
    pub align: Align,
    /// Unix ms of local midnight starting the day.
    pub day_start: f64,
    /// Unix ms of local midnight ending the day.
    pub day_end: f64,
    /// Unix ms of the moment highlighted in the UI.
    pub selected: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Stand {
    pub lat: f64,
    pub lon: f64,
    pub distance_km: f64,
    /// Ground elevation at the spot, when it came from terrain.
    pub ground_m: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct Sample {
    pub t: f64,
    pub moon: MoonState,
    pub sun_alt: f64,
    pub stand: Option<Stand>,
}

pub struct Plan {
    pub samples: Vec<Sample>,
    pub events: Vec<(Crossing, f64)>,
    pub selected: Sample,
    /// Terrain tiles the solution wanted but didn't have yet.
    pub missing: BTreeSet<TileId>,
    /// Spots that fell back to [`FALLBACK_GROUND_M`] for lack of terrain.
    pub assumed: usize,
}

/// Ground lookup used while solving; see [`crate::terrain::Terrain::ground`].
pub type GroundFn<'a> = &'a dyn Fn(f64, f64) -> Ground;

/// Accumulates what the solver learned about terrain coverage.
#[derive(Default)]
struct Needs {
    missing: BTreeSet<TileId>,
    assumed: usize,
}

/// Great-circle destination from a start point, bearing (deg) and distance.
pub fn destination(lat: f64, lon: f64, bearing: f64, dist_m: f64) -> (f64, f64) {
    let (lat1, lon1, brg) = (lat.to_radians(), lon.to_radians(), bearing.to_radians());
    let ang = dist_m / EARTH_RADIUS_M;
    let lat2 = (lat1.sin() * ang.cos() + lat1.cos() * ang.sin() * brg.cos()).asin();
    let lon2 = lon1
        + (brg.sin() * ang.sin() * lat1.cos()).atan2(ang.cos() - lat1.sin() * lat2.sin());
    let lon2 = (lon2.to_degrees() + 540.0).rem_euclid(360.0) - 180.0;
    (lat2.to_degrees(), lon2)
}

/// Distance (m) at which a summit `rise_m` above the eye appears at
/// `angle_deg` elevation, accounting for curvature and refraction:
/// `tan(angle) = rise/d - d(1-k)/2R`.
pub fn stand_distance_m(rise_m: f64, angle_deg: f64) -> Option<f64> {
    if rise_m <= 0.0 || angle_deg >= 89.0 {
        return None;
    }
    let c = (1.0 - REFRACTION_K) / (2.0 * EARTH_RADIUS_M);
    let tan_a = angle_deg.to_radians().tan();
    let d = (-tan_a + (tan_a * tan_a + 4.0 * c * rise_m).sqrt()) / (2.0 * c);
    (d > 0.0 && d <= MAX_STAND_KM * 1000.0).then_some(d)
}

/// Find where to stand, looking back along `azimuth + 180°` from the summit,
/// so that the summit appears at `target` degrees of elevation.
///
/// With terrain, the observer's height depends on where they stand, which
/// depends on their height: iterate (damped) until the spot settles.
fn solve_stand(
    lm: &Landmark,
    observer: Observer,
    azimuth: f64,
    target: f64,
    ground: GroundFn,
    needs: &mut Needs,
) -> Option<Stand> {
    let place = |d: f64, ground_m: Option<f64>| {
        let (lat, lon) = destination(lm.lat, lm.lon, azimuth + 180.0, d);
        Stand { lat, lon, distance_km: d / 1000.0, ground_m }
    };
    let eye = match observer {
        Observer::Fixed(h) => {
            return stand_distance_m(lm.elevation_m - h, target).map(|d| place(d, None));
        }
        Observer::Terrain => FALLBACK_GROUND_M,
    };

    let mut ground_m = eye;
    let mut last: Option<f64> = None;
    for i in 0..10 {
        let d = stand_distance_m(lm.elevation_m - ground_m - EYE_HEIGHT_M, target)?;
        let spot = place(d, None);
        match ground(spot.lat, spot.lon) {
            Ground::Known(g) => {
                if last.is_some_and(|prev| (d - prev).abs() < 25.0) {
                    return Some(Stand { ground_m: Some(g), ..spot });
                }
                last = Some(d);
                // Damp after a few rounds so rugged terrain can't ping-pong.
                ground_m = if i < 3 { g } else { (ground_m + g) / 2.0 };
            }
            Ground::Pending(tile) => {
                needs.missing.insert(tile);
                needs.assumed += 1;
                return Some(spot);
            }
            Ground::Unknown => {
                needs.assumed += 1;
                return Some(spot);
            }
        }
    }
    // Didn't settle within 10 rounds: take the last estimate.
    let d = stand_distance_m(lm.elevation_m - ground_m - EYE_HEIGHT_M, target)?;
    Some(Stand { ground_m: Some(ground_m), ..place(d, None) })
}

fn sample(inp: &Inputs, t: f64, ground: GroundFn, needs: &mut Needs) -> Sample {
    let lm = &inp.landmark;
    let moon = astro::moon(t, lm.lat, lm.lon);
    let sun_alt = astro::sun(t, lm.lat, lm.lon).altitude;
    let target = match inp.align {
        Align::Center => moon.pos.altitude,
        Align::Resting => moon.pos.altitude - moon.diameter / 2.0,
    };
    let stand = (moon.pos.altitude > astro::MOON_HORIZON)
        .then(|| solve_stand(lm, inp.observer, moon.pos.azimuth, target, ground, needs))
        .flatten();
    Sample { t, moon, sun_alt, stand }
}

pub fn compute(inp: &Inputs, ground: GroundFn) -> Plan {
    let mut needs = Needs::default();
    let step = SAMPLE_MINUTES * 60_000.0;
    let n = ((inp.day_end - inp.day_start) / step).ceil() as usize;
    let samples = (0..=n)
        .map(|i| sample(inp, (inp.day_start + i as f64 * step).min(inp.day_end), ground, &mut needs))
        .collect();
    let lm = &inp.landmark;
    let events = astro::crossings(inp.day_start, inp.day_end, astro::MOON_HORIZON, |t| {
        astro::moon(t, lm.lat, lm.lon).pos.altitude
    });
    let selected = sample(inp, inp.selected, ground, &mut needs);
    Plan { samples, events, selected, missing: needs.missing, assumed: needs.assumed }
}

fn pt(lat: f64, lon: f64) -> Value {
    json!([round6(lon), round6(lat)])
}

fn round6(x: f64) -> f64 {
    (x * 1e6).round() / 1e6
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn feature(geometry: Value, properties: Value) -> Value {
    json!({ "type": "Feature", "geometry": geometry, "properties": properties })
}

fn line(coords: Vec<Value>) -> Value {
    json!({ "type": "LineString", "coordinates": coords })
}

fn sample_props(kind: &str, s: &Sample, fmt: &dyn Fn(f64) -> String) -> Value {
    json!({
        "kind": kind,
        "time": fmt(s.t),
        "azimuth": round1(s.moon.pos.azimuth),
        "altitude": round1(s.moon.pos.altitude),
        "distance_km": s.stand.map(|st| round1(st.distance_km)),
        "ground_m": s.stand.and_then(|st| st.ground_m).map(f64::round),
        "illumination": round1(s.moon.illumination * 100.0),
    })
}

/// Build the GeoJSON FeatureCollection. `fmt` formats a unix-ms time for labels.
pub fn to_geojson(inp: &Inputs, plan: &Plan, fmt: &dyn Fn(f64) -> String) -> Value {
    let lm = &inp.landmark;
    let mut features = vec![feature(
        json!({ "type": "Point", "coordinates": pt(lm.lat, lm.lon) }),
        json!({ "kind": "landmark", "name": lm.name, "elevation_m": lm.elevation_m }),
    )];

    // Continuous path of standing spots, split wherever there is no solution.
    let mut segments: Vec<Vec<Value>> = vec![];
    let mut current: Vec<Value> = vec![];
    for s in &plan.samples {
        match s.stand {
            Some(st) => current.push(pt(st.lat, st.lon)),
            None if !current.is_empty() => segments.push(std::mem::take(&mut current)),
            None => {}
        }
    }
    if !current.is_empty() {
        segments.push(current);
    }
    segments.retain(|s| s.len() > 1);
    if !segments.is_empty() {
        features.push(feature(
            json!({ "type": "MultiLineString", "coordinates": segments }),
            json!({ "kind": "path", "name": format!("Where to stand · {}", lm.name) }),
        ));
    }

    // Hourly sight lines from the standing spot to the landmark.
    let hour = 3_600_000.0;
    for s in plan.samples.iter().filter(|s| ((s.t - inp.day_start) % hour).abs() < 1.0) {
        if let Some(st) = s.stand {
            features.push(feature(
                line(vec![pt(st.lat, st.lon), pt(lm.lat, lm.lon)]),
                sample_props("sightline", s, fmt),
            ));
        }
    }

    // Direction of moonrise / moonset as seen from the landmark.
    for &(kind, t) in &plan.events {
        let m = astro::moon(t, lm.lat, lm.lon);
        let (lat, lon) = destination(lm.lat, lm.lon, m.pos.azimuth, MOON_RAY_KM * 1000.0);
        let kind = match kind {
            Crossing::Rise => "moonrise",
            Crossing::Set => "moonset",
        };
        features.push(feature(
            line(vec![pt(lm.lat, lm.lon), pt(lat, lon)]),
            json!({ "kind": kind, "time": fmt(t), "azimuth": round1(m.pos.azimuth) }),
        ));
    }

    // The highlighted moment: observer -> landmark -> on toward the moon.
    let s = &plan.selected;
    let az = s.moon.pos.azimuth;
    let (ray_lat, ray_lon) = destination(lm.lat, lm.lon, az, MOON_RAY_KM * 1000.0);
    let start = match s.stand {
        Some(st) => pt(st.lat, st.lon),
        None => {
            let (la, lo) = destination(lm.lat, lm.lon, az + 180.0, MOON_RAY_KM * 1000.0);
            pt(la, lo)
        }
    };
    features.push(feature(
        line(vec![start, pt(lm.lat, lm.lon), pt(ray_lat, ray_lon)]),
        sample_props("selected", s, fmt),
    ));
    if let Some(st) = s.stand {
        features.push(feature(
            json!({ "type": "Point", "coordinates": pt(st.lat, st.lon) }),
            sample_props("observer", s, fmt),
        ));
    }

    json!({ "type": "FeatureCollection", "features": features })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn destination_one_degree_north() {
        let (lat, lon) = destination(45.0, -121.0, 0.0, 111_195.0);
        assert!((lat - 46.0).abs() < 0.001 && (lon + 121.0).abs() < 1e-9);
    }

    #[test]
    fn stand_distance_matches_flat_earth_when_close() {
        // 1000 m rise at 45°: ~1 km away; curvature negligible.
        let d = stand_distance_m(1000.0, 45.0).unwrap();
        assert!((d - 1000.0).abs() < 1.0, "{d}");
    }

    fn hood() -> Landmark {
        Landmark {
            name: "Mount Hood".into(),
            lat: 45.37362,
            lon: -121.69591,
            elevation_m: 3429.0,
            tz: "America/Los_Angeles".into(),
        }
    }

    #[test]
    fn terrain_on_flat_ground_matches_fixed_elevation() {
        let flat = |_: f64, _: f64| Ground::Known(800.0);
        let mut needs = Needs::default();
        let t = solve_stand(&hood(), Observer::Terrain, 100.0, 5.0, &flat, &mut needs).unwrap();
        let f = solve_stand(&hood(), Observer::Fixed(800.0 + EYE_HEIGHT_M), 100.0, 5.0, &flat, &mut needs)
            .unwrap();
        assert!((t.distance_km - f.distance_km).abs() < 0.05, "{} vs {}", t.distance_km, f.distance_km);
        assert_eq!(t.ground_m, Some(800.0));
        assert!(needs.missing.is_empty() && needs.assumed == 0);
    }

    #[test]
    fn terrain_settles_on_sloping_ground() {
        // Ground rises 10 m per km west of the summit (spots are to the west).
        let slope = |_: f64, lon: f64| Ground::Known(((-121.69591 - lon) * 78.0 * 10.0).max(0.0) + 200.0);
        let mut needs = Needs::default();
        let s = solve_stand(&hood(), Observer::Terrain, 90.0, 6.0, &slope, &mut needs).unwrap();
        let g = s.ground_m.unwrap();
        // Self-consistent: the distance solved for that ground height is where we are.
        let d = stand_distance_m(3429.0 - g - EYE_HEIGHT_M, 6.0).unwrap() / 1000.0;
        assert!((d - s.distance_km).abs() < 0.2, "{d} vs {}", s.distance_km);
    }

    #[test]
    fn terrain_reports_missing_tiles() {
        let tile = TileId { z: 10, x: 1, y: 2 };
        let pending = |_: f64, _: f64| Ground::Pending(tile);
        let mut needs = Needs::default();
        assert!(solve_stand(&hood(), Observer::Terrain, 90.0, 6.0, &pending, &mut needs).is_some());
        assert!(needs.missing.contains(&tile) && needs.assumed == 1);
    }

    #[test]
    fn stand_distance_includes_curvature() {
        // Hood from Portland: ~3300 m rise at ~2°. Flat earth says 94.5 km;
        // curvature pulls you closer.
        let d = stand_distance_m(3300.0, 2.0).unwrap();
        assert!(d < 94_500.0 && d > 70_000.0, "{d}");
        assert!(stand_distance_m(3300.0, -1.0).is_none());
        assert!(stand_distance_m(-10.0, 5.0).is_none());
    }
}
