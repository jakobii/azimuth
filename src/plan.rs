//! Turns moon positions into places to stand, and those into GeoJSON.
//!
//! For each moment the moon is up we look *back* along its azimuth from the
//! landmark and solve for the distance at which the landmark's summit sits at
//! the moon's apparent altitude. Standing there puts the moon on the summit.

use crate::astro::{self, Crossing, MoonState};
use serde_json::{Value, json};

const EARTH_RADIUS_M: f64 = 6_371_000.0;
/// Standard terrestrial refraction coefficient.
const REFRACTION_K: f64 = 0.13;
/// Beyond this the summit is lost in haze and curvature anyway.
pub const MAX_STAND_KM: f64 = 250.0;
const MOON_RAY_KM: f64 = 60.0;
const SAMPLE_MINUTES: f64 = 5.0;

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

#[derive(Clone, Debug, PartialEq)]
pub struct Inputs {
    pub landmark: Landmark,
    /// Observer eye elevation, metres above sea level.
    pub observer_elev_m: f64,
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

fn sample(inp: &Inputs, t: f64) -> Sample {
    let lm = &inp.landmark;
    let moon = astro::moon(t, lm.lat, lm.lon);
    let sun_alt = astro::sun(t, lm.lat, lm.lon).altitude;
    let target = match inp.align {
        Align::Center => moon.pos.altitude,
        Align::Resting => moon.pos.altitude - moon.diameter / 2.0,
    };
    let stand = (moon.pos.altitude > astro::MOON_HORIZON)
        .then(|| stand_distance_m(lm.elevation_m - inp.observer_elev_m, target))
        .flatten()
        .map(|d| {
            let (lat, lon) = destination(lm.lat, lm.lon, moon.pos.azimuth + 180.0, d);
            Stand { lat, lon, distance_km: d / 1000.0 }
        });
    Sample { t, moon, sun_alt, stand }
}

pub fn compute(inp: &Inputs) -> Plan {
    let step = SAMPLE_MINUTES * 60_000.0;
    let n = ((inp.day_end - inp.day_start) / step).ceil() as usize;
    let samples = (0..=n)
        .map(|i| sample(inp, (inp.day_start + i as f64 * step).min(inp.day_end)))
        .collect();
    let lm = &inp.landmark;
    let events = astro::crossings(inp.day_start, inp.day_end, astro::MOON_HORIZON, |t| {
        astro::moon(t, lm.lat, lm.lon).pos.altitude
    });
    Plan { samples, events, selected: sample(inp, inp.selected) }
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
