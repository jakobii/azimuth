//! Bindings to `public/glue.js` (Leaflet, Intl time zones, downloads, search).

use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = azimuthGlue)]
    pub fn update(
        el: &web_sys::HtmlElement,
        geojson: &str,
        lat: f64,
        lon: f64,
        on_click: &js_sys::Function,
    );

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = zonedTime)]
    pub fn zoned_time(date: &str, minutes: f64, tz: &str) -> f64;

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = formatTime)]
    pub fn format_time(ms: f64, tz: &str) -> String;

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = zoneAbbrev)]
    pub fn zone_abbrev(ms: f64, tz: &str) -> String;

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = nowIn)]
    fn now_in_raw(tz: &str) -> js_sys::Array;

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = timeZones)]
    fn time_zones_raw() -> js_sys::Array;

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = isValidZone)]
    pub fn is_valid_zone(tz: &str) -> bool;

    #[wasm_bindgen(js_namespace = azimuthGlue)]
    pub fn download(filename: &str, text: &str);

    #[wasm_bindgen(js_namespace = azimuthGlue, js_name = search, catch)]
    fn search_raw(q: &str) -> Result<js_sys::Promise, JsValue>;
}

/// Today's date (YYYY-MM-DD) and minutes since midnight in `tz`.
pub fn now_in(tz: &str) -> (String, f64) {
    let a = now_in_raw(tz);
    (a.get(0).as_string().unwrap_or_default(), a.get(1).as_f64().unwrap_or(0.0))
}

pub fn time_zones() -> Vec<String> {
    time_zones_raw().iter().filter_map(|v| v.as_string()).collect()
}

pub struct SearchHit {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
    pub ele: Option<f64>,
}

pub async fn search(q: &str) -> Result<Option<SearchHit>, String> {
    let promise = search_raw(q).map_err(|e| format!("{e:?}"))?;
    let v = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| "Search failed — are you offline?".to_string())?;
    let Some(s) = v.as_string() else { return Ok(None) };
    let j: serde_json::Value = serde_json::from_str(&s).map_err(|e| e.to_string())?;
    Ok(Some(SearchHit {
        name: j["name"].as_str().unwrap_or("Search result").to_string(),
        lat: j["lat"].as_f64().ok_or("bad lat")?,
        lon: j["lon"].as_f64().ok_or("bad lon")?,
        ele: j["ele"].as_f64(),
    }))
}
