use crate::astro::Crossing;
use crate::glue;
use crate::plan::{self, Align, Inputs, Landmark, Plan, Sample};
use leptos::html::Div;
use leptos::prelude::*;
use std::sync::Arc;
use wasm_bindgen::prelude::*;

struct Preset {
    name: &'static str,
    lat: f64,
    lon: f64,
    elevation_m: f64,
    tz: &'static str,
}

#[rustfmt::skip]
const PRESETS: &[Preset] = &[
    Preset { name: "Mount Hood", lat: 45.37362, lon: -121.69591, elevation_m: 3429.0, tz: "America/Los_Angeles" },
    Preset { name: "Mount St. Helens", lat: 46.19117, lon: -122.19566, elevation_m: 2549.0, tz: "America/Los_Angeles" },
    Preset { name: "Mount Adams", lat: 46.20245, lon: -121.49069, elevation_m: 3743.0, tz: "America/Los_Angeles" },
    Preset { name: "Mount Rainier", lat: 46.85283, lon: -121.76044, elevation_m: 4392.0, tz: "America/Los_Angeles" },
    Preset { name: "Mount Jefferson", lat: 44.67412, lon: -121.79936, elevation_m: 3199.0, tz: "America/Los_Angeles" },
    Preset { name: "Half Dome", lat: 37.74604, lon: -119.53320, elevation_m: 2694.0, tz: "America/Los_Angeles" },
    Preset { name: "Devils Tower", lat: 44.59048, lon: -104.71527, elevation_m: 1558.0, tz: "America/Denver" },
    Preset { name: "Matterhorn", lat: 45.97637, lon: 7.65861, elevation_m: 4478.0, tz: "Europe/Zurich" },
    Preset { name: "Mount Fuji", lat: 35.36062, lon: 138.72742, elevation_m: 3776.0, tz: "Asia/Tokyo" },
];

impl Preset {
    fn landmark(&self) -> Landmark {
        Landmark {
            name: self.name.into(),
            lat: self.lat,
            lon: self.lon,
            elevation_m: self.elevation_m,
            tz: self.tz.into(),
        }
    }
}

fn phase_name(illum: f64, waxing: bool) -> &'static str {
    match (illum, waxing) {
        (i, _) if i < 0.03 => "New moon",
        (i, _) if i > 0.97 => "Full moon",
        (i, true) if (0.45..=0.55).contains(&i) => "First quarter",
        (i, false) if (0.45..=0.55).contains(&i) => "Last quarter",
        (i, true) if i < 0.5 => "Waxing crescent",
        (_, true) => "Waxing gibbous",
        (i, false) if i < 0.5 => "Waning crescent",
        (_, false) => "Waning gibbous",
    }
}

fn sky(sun_alt: f64) -> &'static str {
    match sun_alt {
        a if a > 6.0 => "Daylight",
        a if a > -0.83 => "Golden hour",
        a if a > -6.0 => "Civil twilight",
        a if a > -12.0 => "Nautical twilight",
        a if a > -18.0 => "Astronomical twilight",
        _ => "Night",
    }
}

fn parse_f64(ev: &leptos::ev::Event) -> Option<f64> {
    event_target_value(ev).trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

fn slug(s: &str) -> String {
    let s: String = s
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

#[component]
pub fn App() -> impl IntoView {
    let landmark = RwSignal::new(PRESETS[0].landmark());
    let (today, now_min) = glue::now_in(PRESETS[0].tz);
    let date = RwSignal::new(today);
    let minutes = RwSignal::new((now_min / 5.0).round() * 5.0);
    let observer_elev = RwSignal::new(500.0_f64);
    let align = RwSignal::new(Align::Center);
    let picking = RwSignal::new(false);
    let query = RwSignal::new(String::new());
    let search_msg = RwSignal::new(None::<String>);
    let zones = StoredValue::new(glue::time_zones());

    let tz = Memo::new(move |_| landmark.with(|l| l.tz.clone()));
    let inputs = Memo::new(move |_| {
        let tz = tz.get();
        let d = date.get();
        Inputs {
            landmark: landmark.get(),
            observer_elev_m: observer_elev.get(),
            align: align.get(),
            day_start: glue::zoned_time(&d, 0.0, &tz),
            day_end: glue::zoned_time(&d, 1440.0, &tz),
            selected: glue::zoned_time(&d, minutes.get(), &tz),
        }
    });
    let computed = Memo::new(move |_| {
        let inp = inputs.get();
        let plan = plan::compute(&inp);
        let tz = inp.landmark.tz.clone();
        let fmt = move |t: f64| glue::format_time(t, &tz);
        let gj = plan::to_geojson(&inp, &plan, &fmt).to_string();
        StoredPlan(Arc::new(plan), gj)
    });
    let fmt = move |t: f64| glue::format_time(t, &tz.get());
    let to_minutes = move |t: f64| {
        let start = inputs.with(|i| i.day_start);
        ((t - start) / 60_000.0).round().clamp(0.0, 1439.0)
    };

    // Map: created on first run, then redrawn whenever the GeoJSON changes.
    let map_el = NodeRef::<Div>::new();
    let on_click: js_sys::Function = Closure::<dyn Fn(f64, f64)>::new(move |lat: f64, lon: f64| {
        if picking.get_untracked() {
            picking.set(false);
            landmark.update(|l| {
                l.name = "Dropped pin".into();
                l.lat = lat;
                l.lon = lon;
            });
        }
    })
    .into_js_value()
    .unchecked_into();
    Effect::new(move |_| {
        let (lat, lon) = landmark.with(|l| (l.lat, l.lon));
        let gj = computed.with(|c| c.1.clone());
        if let Some(el) = map_el.get() {
            glue::update(&el, &gj, lat, lon, &on_click);
        }
    });

    let run_search = move || {
        let q = query.get_untracked();
        if q.trim().is_empty() {
            return;
        }
        search_msg.set(Some("Searching…".into()));
        leptos::task::spawn_local(async move {
            match glue::search(&q).await {
                Ok(Some(hit)) => {
                    search_msg.set(match hit.ele {
                        Some(_) => None,
                        None => Some("No elevation on record — set it below.".into()),
                    });
                    landmark.update(|l| {
                        l.name = hit.name;
                        l.lat = hit.lat;
                        l.lon = hit.lon;
                        if let Some(e) = hit.ele {
                            l.elevation_m = e;
                        }
                    });
                }
                Ok(None) => search_msg.set(Some("No match.".into())),
                Err(e) => search_msg.set(Some(e)),
            }
        });
    };

    let selected = move || computed.with(|c| c.0.selected);
    let preset_value = move || {
        landmark.with(|l| {
            PRESETS
                .iter()
                .position(|p| p.name == l.name && p.lat == l.lat && p.lon == l.lon)
                .map(|i| i.to_string())
                .unwrap_or_else(|| "custom".into())
        })
    };

    view! {
        <main class="layout">
            <section class="panel">
                <header>
                    <h1>"Azimuth"</h1>
                    <p class="tagline">"Where to stand to put the moon on a summit."</p>
                </header>

                <fieldset>
                    <legend>"Landmark"</legend>
                    <label>
                        "Preset"
                        <select
                            prop:value=preset_value
                            on:change=move |ev| {
                                if let Ok(i) = event_target_value(&ev).parse::<usize>() {
                                    landmark.set(PRESETS[i].landmark());
                                }
                            }
                        >
                            {PRESETS
                                .iter()
                                .enumerate()
                                .map(|(i, p)| view! { <option value=i.to_string()>{p.name}</option> })
                                .collect_view()}
                            <option value="custom" disabled=true>"Custom"</option>
                        </select>
                    </label>
                    <form
                        class="search"
                        on:submit=move |ev| {
                            ev.prevent_default();
                            run_search();
                        }
                    >
                        <input
                            type="search"
                            placeholder="Search OpenStreetMap…"
                            prop:value=move || query.get()
                            on:input=move |ev| query.set(event_target_value(&ev))
                        />
                        <button type="submit">"Find"</button>
                    </form>
                    {move || search_msg.get().map(|m| view! { <p class="hint">{m}</p> })}
                    <button
                        type="button"
                        class:active=move || picking.get()
                        on:click=move |_| picking.update(|p| *p = !*p)
                    >
                        {move || if picking.get() { "Click the map…" } else { "Pick on map" }}
                    </button>
                    <label>
                        "Name"
                        <input
                            type="text"
                            prop:value=move || landmark.with(|l| l.name.clone())
                            on:change=move |ev| landmark.update(|l| l.name = event_target_value(&ev))
                        />
                    </label>
                    <div class="row">
                        <label>
                            "Latitude"
                            <input
                                type="number"
                                step="0.00001"
                                min="-90"
                                max="90"
                                prop:value=move || format!("{:.5}", landmark.with(|l| l.lat))
                                on:change=move |ev| {
                                    if let Some(v) = parse_f64(&ev).filter(|v| v.abs() <= 90.0) {
                                        landmark.update(|l| l.lat = v);
                                    }
                                }
                            />
                        </label>
                        <label>
                            "Longitude"
                            <input
                                type="number"
                                step="0.00001"
                                min="-180"
                                max="180"
                                prop:value=move || format!("{:.5}", landmark.with(|l| l.lon))
                                on:change=move |ev| {
                                    if let Some(v) = parse_f64(&ev).filter(|v| v.abs() <= 180.0) {
                                        landmark.update(|l| l.lon = v);
                                    }
                                }
                            />
                        </label>
                    </div>
                    <div class="row">
                        <label>
                            "Summit elevation (m)"
                            <input
                                type="number"
                                step="1"
                                prop:value=move || landmark.with(|l| l.elevation_m).to_string()
                                on:change=move |ev| {
                                    if let Some(v) = parse_f64(&ev) {
                                        landmark.update(|l| l.elevation_m = v);
                                    }
                                }
                            />
                        </label>
                        <label>
                            "Your elevation (m)"
                            <input
                                type="number"
                                step="1"
                                prop:value=move || observer_elev.get().to_string()
                                on:change=move |ev| {
                                    if let Some(v) = parse_f64(&ev) {
                                        observer_elev.set(v);
                                    }
                                }
                            />
                        </label>
                    </div>
                    <label>
                        "Time zone"
                        <input
                            type="text"
                            list="zones"
                            prop:value=move || tz.get()
                            on:change=move |ev| {
                                let v = event_target_value(&ev);
                                if glue::is_valid_zone(&v) {
                                    landmark.update(|l| l.tz = v);
                                }
                            }
                        />
                        <datalist id="zones">
                            {zones
                                .get_value()
                                .into_iter()
                                .map(|z| view! { <option value=z /> })
                                .collect_view()}
                        </datalist>
                    </label>
                </fieldset>

                <fieldset>
                    <legend>"When"</legend>
                    <div class="row">
                        <label>
                            "Date"
                            <input
                                type="date"
                                prop:value=move || date.get()
                                on:change=move |ev| {
                                    let v = event_target_value(&ev);
                                    if !v.is_empty() {
                                        date.set(v);
                                    }
                                }
                            />
                        </label>
                        <button
                            type="button"
                            class="now"
                            on:click=move |_| {
                                let (d, m) = glue::now_in(&tz.get_untracked());
                                date.set(d);
                                minutes.set(m);
                            }
                        >
                            "Now"
                        </button>
                    </div>
                    <label>
                        <span class="time">
                            {move || fmt(inputs.with(|i| i.selected))}
                            " "
                            <small>{move || glue::zone_abbrev(inputs.with(|i| i.selected), &tz.get())}</small>
                        </span>
                        <input
                            type="range"
                            min="0"
                            max="1439"
                            step="5"
                            prop:value=move || minutes.get().to_string()
                            on:input=move |ev| {
                                if let Some(v) = parse_f64(&ev) {
                                    minutes.set(v);
                                }
                            }
                        />
                    </label>
                    <label>
                        "Alignment"
                        <select
                            prop:value=move || match align.get() {
                                Align::Center => "center",
                                Align::Resting => "resting",
                            }
                            on:change=move |ev| {
                                align.set(if event_target_value(&ev) == "resting" {
                                    Align::Resting
                                } else {
                                    Align::Center
                                })
                            }
                        >
                            <option value="center">"Moon centred on summit"</option>
                            <option value="resting">"Moon resting on summit"</option>
                        </select>
                    </label>
                </fieldset>

                <section class="readout">
                    {move || {
                        let s = selected();
                        let m = s.moon;
                        view! {
                            <dl>
                                <dt>"Moon"</dt>
                                <dd>
                                    {format!("az {:.1}° · alt {:.1}°", m.pos.azimuth, m.pos.altitude)}
                                </dd>
                                <dt>"Phase"</dt>
                                <dd>
                                    {format!(
                                        "{} · {:.0}% lit",
                                        phase_name(m.illumination, m.waxing),
                                        m.illumination * 100.0,
                                    )}
                                </dd>
                                <dt>"Stand"</dt>
                                <dd>
                                    {match s.stand {
                                        Some(st) => {
                                            format!(
                                                "{:.1} km from summit · {:.5}, {:.5}",
                                                st.distance_km,
                                                st.lat,
                                                st.lon,
                                            )
                                        }
                                        None if m.pos.altitude < 0.0 => "Moon is below the horizon".into(),
                                        None => "No spot within range".into(),
                                    }}
                                </dd>
                                <dt>"Sky"</dt>
                                <dd>{format!("{} · sun {:.1}°", sky(s.sun_alt), s.sun_alt)}</dd>
                            </dl>
                        }
                    }}
                    <div class="events">
                        {move || {
                            let events = computed.with(|c| c.0.events.clone());
                            if events.is_empty() {
                                return view! { <p class="hint">"No moonrise or moonset this day."</p> }
                                    .into_any();
                            }
                            events
                                .into_iter()
                                .map(|(kind, t)| {
                                    let label = match kind {
                                        Crossing::Rise => "Moonrise",
                                        Crossing::Set => "Moonset",
                                    };
                                    view! {
                                        <button
                                            type="button"
                                            class="event"
                                            on:click=move |_| minutes.set(to_minutes(t))
                                        >
                                            {format!("{label} {}", fmt(t))}
                                        </button>
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }}
                    </div>
                </section>

                <HourTable computed=computed fmt=fmt minutes=minutes to_minutes=to_minutes />

                <button
                    type="button"
                    class="download"
                    on:click=move |_| {
                        let name = format!(
                            "moon-{}-{}.geojson",
                            slug(&landmark.with_untracked(|l| l.name.clone())),
                            date.get_untracked(),
                        );
                        glue::download(&name, &computed.with_untracked(|c| c.1.clone()));
                    }
                >
                    "Download GeoJSON"
                </button>

                <footer>
                    <p>
                        <span class="key path"></span>
                        "Where to stand · "
                        <span class="key rise"></span>
                        "Moonrise · "
                        <span class="key set"></span>
                        "Moonset"
                    </p>
                    <p class="hint">
                        "Works offline once loaded; map tiles you've viewed are cached. Terrain is not modelled — check that nothing blocks the view."
                    </p>
                </footer>
            </section>
            <div class="map" class:picking=move || picking.get() node_ref=map_el></div>
        </main>
    }
}

/// Plan wrapped for memoisation: the GeoJSON string is the change signal.
#[derive(Clone)]
struct StoredPlan(Arc<Plan>, String);

impl PartialEq for StoredPlan {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}

#[component]
fn HourTable(
    computed: Memo<StoredPlan>,
    fmt: impl Fn(f64) -> String + Copy + Send + Sync + 'static,
    minutes: RwSignal<f64>,
    to_minutes: impl Fn(f64) -> f64 + Copy + Send + Sync + 'static,
) -> impl IntoView {
    let rows = move || {
        computed.with(|c| {
            c.0.samples
                .iter()
                .enumerate()
                .filter(|(i, s)| i % 12 == 0 && s.moon.pos.altitude > 0.0)
                .map(|(_, s)| *s)
                .collect::<Vec<Sample>>()
        })
    };
    view! {
        <details class="hours">
            <summary>"Hour by hour"</summary>
            <table>
                <thead>
                    <tr>
                        <th>"Time"</th>
                        <th>"Az"</th>
                        <th>"Alt"</th>
                        <th>"Stand"</th>
                        <th>"Sky"</th>
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        rows()
                            .into_iter()
                            .map(|s| {
                                view! {
                                    <tr on:click=move |_| minutes.set(to_minutes(s.t))>
                                        <td>{fmt(s.t)}</td>
                                        <td>{format!("{:.0}°", s.moon.pos.azimuth)}</td>
                                        <td>{format!("{:.1}°", s.moon.pos.altitude)}</td>
                                        <td>
                                            {s
                                                .stand
                                                .map(|st| format!("{:.0} km", st.distance_km))
                                                .unwrap_or_else(|| "—".into())}
                                        </td>
                                        <td>{sky(s.sun_alt)}</td>
                                    </tr>
                                }
                            })
                            .collect_view()
                    }}
                </tbody>
            </table>
        </details>
    }
}
