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

/// Shift a YYYY-MM-DD date by whole days.
fn shift_date(date: &str, days: f64) -> String {
    let t = js_sys::Date::parse(&format!("{date}T00:00:00Z")) + days * 86_400_000.0;
    let iso: String = js_sys::Date::new(&t.into()).to_iso_string().into();
    iso[..10].to_string()
}

/// Plan wrapped for memoisation: the GeoJSON string is the change signal.
#[derive(Clone)]
struct StoredPlan(Arc<Plan>, String);

impl PartialEq for StoredPlan {
    fn eq(&self, other: &Self) -> bool {
        self.1 == other.1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tool {
    Place,
    Time,
    Moon,
    Setup,
}

impl Tool {
    const ALL: [Tool; 4] = [Tool::Place, Tool::Time, Tool::Moon, Tool::Setup];

    fn label(self) -> &'static str {
        match self {
            Tool::Place => "Place",
            Tool::Time => "Time",
            Tool::Moon => "Moon",
            Tool::Setup => "Setup",
        }
    }

    fn icon(self) -> impl IntoView {
        let body = match self {
            Tool::Place => view! {
                <path d="M12 21s-7-6.2-7-11a7 7 0 0 1 14 0c0 4.8-7 11-7 11z" />
                <circle cx="12" cy="10" r="2.5" />
            }
            .into_any(),
            Tool::Time => view! {
                <circle cx="12" cy="12" r="9" />
                <path d="M12 7v5l3 2" />
            }
            .into_any(),
            Tool::Moon => view! { <path d="M20 14.5A8 8 0 1 1 9.5 4a6.5 6.5 0 0 0 10.5 10.5z" /> }
                .into_any(),
            Tool::Setup => view! {
                <path d="M4 7h16M4 17h16" />
                <circle cx="9" cy="7" r="2.2" class="fill" />
                <circle cx="15" cy="17" r="2.2" class="fill" />
            }
            .into_any(),
        };
        view! {
            <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
                {body}
            </svg>
        }
    }
}

/// All app state; every field is a cheap `Copy` handle.
#[derive(Clone, Copy)]
struct State {
    landmark: RwSignal<Landmark>,
    date: RwSignal<String>,
    minutes: RwSignal<f64>,
    observer_elev: RwSignal<f64>,
    align: RwSignal<Align>,
    picking: RwSignal<bool>,
    tool: RwSignal<Option<Tool>>,
    tz: Memo<String>,
    inputs: Memo<Inputs>,
    computed: Memo<StoredPlan>,
}

impl State {
    fn new() -> Self {
        let landmark = RwSignal::new(PRESETS[0].landmark());
        let (today, now_min) = glue::now_in(PRESETS[0].tz);
        let date = RwSignal::new(today);
        let minutes = RwSignal::new((now_min / 5.0).round() * 5.0);
        let observer_elev = RwSignal::new(500.0_f64);
        let align = RwSignal::new(Align::Center);

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

        State {
            landmark,
            date,
            minutes,
            observer_elev,
            align,
            picking: RwSignal::new(false),
            tool: RwSignal::new(None),
            tz,
            inputs,
            computed,
        }
    }

    fn fmt(self, t: f64) -> String {
        glue::format_time(t, &self.tz.get())
    }

    fn selected(self) -> Sample {
        self.computed.with(|c| c.0.selected)
    }

    /// Minutes after local midnight for a time on the current day.
    fn jump_to(self, t: f64) {
        let start = self.inputs.with_untracked(|i| i.day_start);
        self.minutes.set(((t - start) / 60_000.0).round().clamp(0.0, 1439.0));
    }

    fn toggle(self, tool: Tool) {
        self.tool.update(|t| *t = if *t == Some(tool) { None } else { Some(tool) });
    }
}

#[component]
pub fn App() -> impl IntoView {
    let st = State::new();

    // Map: created on first run, then redrawn whenever the GeoJSON changes.
    // A tap on the map drops the landmark when picking, else closes the sheet.
    let map_el = NodeRef::<Div>::new();
    let on_click: js_sys::Function = Closure::<dyn Fn(f64, f64)>::new(move |lat: f64, lon: f64| {
        if st.picking.get_untracked() {
            st.picking.set(false);
            st.landmark.update(|l| {
                l.name = "Dropped pin".into();
                l.lat = lat;
                l.lon = lon;
            });
            st.tool.set(Some(Tool::Place));
        } else {
            st.tool.set(None);
        }
    })
    .into_js_value()
    .unchecked_into();
    Effect::new(move |_| {
        let (lat, lon) = st.landmark.with(|l| (l.lat, l.lon));
        let gj = st.computed.with(|c| c.1.clone());
        if let Some(el) = map_el.get() {
            glue::update(&el, &gj, lat, lon, &on_click);
        }
    });

    // Tell the map how much of it the sheet covers.
    let sheet_el = NodeRef::<leptos::html::Section>::new();
    Effect::new(move |_| {
        glue::track_sheet(st.tool.get().and(sheet_el.get()));
    });

    let sheet = move || {
        let tool = st.tool.get()?;
        let body = match tool {
            Tool::Place => view! { <PlaceSheet st=st /> }.into_any(),
            Tool::Time => view! { <TimeSheet st=st /> }.into_any(),
            Tool::Moon => view! { <MoonSheet st=st /> }.into_any(),
            Tool::Setup => view! { <SetupSheet st=st /> }.into_any(),
        };
        Some(view! {
            <section class="sheet" role="dialog" aria-label=tool.label() node_ref=sheet_el>
                <header class="sheet-head">
                    <h2>{tool.label()}</h2>
                    <button type="button" class="close" aria-label="Close" on:click=move |_| st.tool.set(None)>
                        "×"
                    </button>
                </header>
                <div class="sheet-body">{body}</div>
            </section>
        })
    };

    view! {
        <main class="app" on:keydown=move |ev| if ev.key() == "Escape" { st.tool.set(None) }>
            <div class="map-wrap">
                <div class="map" class:picking=move || st.picking.get() node_ref=map_el></div>
                <StatusChip st=st />
                <MapKey />
                <Show when=move || st.picking.get()>
                    <div class="banner">
                        "Tap the map to place the landmark"
                        <button type="button" on:click=move |_| st.picking.set(false)>"Cancel"</button>
                    </div>
                </Show>
            </div>
            {sheet}
            <nav class="toolbar" aria-label="Tools">
                {Tool::ALL
                    .into_iter()
                    .map(|tool| {
                        view! {
                            <button
                                type="button"
                                class="tool"
                                class:active=move || st.tool.get() == Some(tool)
                                aria-pressed=move || (st.tool.get() == Some(tool)).to_string()
                                on:click=move |_| st.toggle(tool)
                            >
                                {tool.icon()}
                                <span>{tool.label()}</span>
                            </button>
                        }
                    })
                    .collect_view()}
            </nav>
        </main>
    }
}

/// Compact summary floating over the map; tap for moon details.
#[component]
fn StatusChip(st: State) -> impl IntoView {
    view! {
        <button type="button" class="chip" on:click=move |_| st.tool.set(Some(Tool::Moon))>
            <strong>{move || st.landmark.with(|l| l.name.clone())}</strong>
            <span>
                {move || {
                    let t = st.inputs.with(|i| i.selected);
                    format!("{} · {}", st.fmt(t), glue::zone_abbrev(t, &st.tz.get()))
                }}
            </span>
            <span>
                {move || {
                    let s = st.selected();
                    match s.stand {
                        Some(stand) => {
                            format!(
                                "Moon {:.0}° / {:.1}° · stand {:.1} km",
                                s.moon.pos.azimuth,
                                s.moon.pos.altitude,
                                stand.distance_km,
                            )
                        }
                        None if s.moon.pos.altitude < 0.0 => "Moon below horizon".into(),
                        None => format!("Moon {:.0}° / {:.1}°", s.moon.pos.azimuth, s.moon.pos.altitude),
                    }
                }}
            </span>
        </button>
    }
}

/// How a map feature is drawn; mirrors the styles in `public/glue.js`.
#[derive(Clone, Copy)]
enum Swatch {
    /// Filled circle: fill, stroke.
    Dot(&'static str, &'static str),
    /// Line: colour, width, dash pattern (empty for solid).
    Line(&'static str, f64, &'static str),
    /// White line over a dark halo.
    Selected,
}

impl Swatch {
    fn view(self) -> impl IntoView {
        let body = match self {
            Swatch::Dot(fill, stroke) => view! {
                <circle cx="16" cy="8" r="5.5" fill=fill stroke=stroke stroke-width="2" />
            }
            .into_any(),
            Swatch::Line(color, width, dash) => view! {
                <line x1="2" y1="8" x2="30" y2="8" stroke=color stroke-width=width stroke-dasharray=dash />
            }
            .into_any(),
            Swatch::Selected => view! {
                <line x1="2" y1="8" x2="30" y2="8" stroke="#111" stroke-opacity="0.35" stroke-width="5" />
                <line x1="2" y1="8" x2="30" y2="8" stroke="#fff" stroke-width="2.5" />
            }
            .into_any(),
        };
        view! {
            <svg class="map-key-swatch" viewBox="0 0 32 16" aria-hidden="true">
                {body}
            </svg>
        }
    }
}

#[rustfmt::skip]
const KEY: &[(Swatch, &str, &str)] = &[
    (Swatch::Dot("#e4572e", "#fff"), "Landmark", "The summit you're lining the moon up with."),
    (Swatch::Line("#4f86f7", 4.0, ""), "Where to stand",
        "Spots where the moon sits on the summit as it crosses the sky — far away when it's low, close when it's high."),
    (Swatch::Line("#4f86f7", 1.5, "4 3"), "Hourly sight lines", "Each hour's standing spot joined to the summit."),
    (Swatch::Selected, "Selected time", "From you, through the summit, toward the moon at the slider's time."),
    (Swatch::Dot("#f5f5f5", "#111"), "You", "Where to stand at the slider's time."),
    (Swatch::Line("#f2c14e", 3.0, "6 4"), "Moonrise", "Direction the moon rises, seen from the summit."),
    (Swatch::Line("#c97b3d", 3.0, "6 4"), "Moonset", "Direction the moon sets, seen from the summit."),
];

/// Collapsible legend in the map's top-right corner.
#[component]
fn MapKey() -> impl IntoView {
    let open = RwSignal::new(false);
    view! {
        <div class="map-key" class:open=move || open.get()>
            <button
                type="button"
                class="map-key-toggle"
                aria-expanded=move || open.get().to_string()
                aria-controls="map-key-list"
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <svg class="icon" viewBox="0 0 24 24" aria-hidden="true">
                    <path d="M12 3 3 8l9 5 9-5-9-5zM3 13l9 5 9-5M3 18l9 5 9-5" />
                </svg>
                <span>"Key"</span>
            </button>
            <Show when=move || open.get()>
                <dl id="map-key-list" class="map-key-list">
                    {KEY
                        .iter()
                        .map(|&(swatch, name, desc)| {
                            view! {
                                <div class="map-key-row">
                                    <dt>{swatch.view()} {name}</dt>
                                    <dd>{desc}</dd>
                                </div>
                            }
                        })
                        .collect_view()}
                </dl>
            </Show>
        </div>
    }
}

#[component]
fn PlaceSheet(st: State) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let search_msg = RwSignal::new(None::<String>);
    let landmark = st.landmark;

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
        <form
            class="search"
            on:submit=move |ev| {
                ev.prevent_default();
                run_search();
            }
        >
            <input
                type="search"
                enterkeyhint="search"
                placeholder="Search OpenStreetMap…"
                prop:value=move || query.get()
                on:input=move |ev| query.set(event_target_value(&ev))
            />
            <button type="submit">"Find"</button>
        </form>
        {move || search_msg.get().map(|m| view! { <p class="hint">{m}</p> })}
        <div class="row">
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
            <button
                type="button"
                class="pick"
                on:click=move |_| {
                    st.picking.set(true);
                    st.tool.set(None);
                }
            >
                "Pick on map"
            </button>
        </div>
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
                    inputmode="decimal"
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
                    inputmode="decimal"
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
        <label>
            "Summit elevation (m)"
            <input
                type="number"
                inputmode="decimal"
                step="1"
                prop:value=move || landmark.with(|l| l.elevation_m).to_string()
                on:change=move |ev| {
                    if let Some(v) = parse_f64(&ev) {
                        landmark.update(|l| l.elevation_m = v);
                    }
                }
            />
        </label>
    }
}

#[component]
fn TimeSheet(st: State) -> impl IntoView {
    let events = move || st.computed.with(|c| c.0.events.clone());
    view! {
        <div class="clock">
            <span class="time">{move || st.fmt(st.inputs.with(|i| i.selected))}</span>
            <small>{move || glue::zone_abbrev(st.inputs.with(|i| i.selected), &st.tz.get())}</small>
        </div>
        <input
            type="range"
            class="slider"
            aria-label="Time of day"
            min="0"
            max="1439"
            step="5"
            prop:value=move || st.minutes.get().to_string()
            on:input=move |ev| {
                if let Some(v) = parse_f64(&ev) {
                    st.minutes.set(v);
                }
            }
        />
        <div class="row date-row">
            <button type="button" aria-label="Previous day" on:click=move |_| st.date.update(|d| *d = shift_date(d, -1.0))>
                "‹"
            </button>
            <input
                type="date"
                aria-label="Date"
                prop:value=move || st.date.get()
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    if !v.is_empty() {
                        st.date.set(v);
                    }
                }
            />
            <button type="button" aria-label="Next day" on:click=move |_| st.date.update(|d| *d = shift_date(d, 1.0))>
                "›"
            </button>
            <button
                type="button"
                on:click=move |_| {
                    let (d, m) = glue::now_in(&st.tz.get_untracked());
                    st.date.set(d);
                    st.minutes.set(m);
                }
            >
                "Now"
            </button>
        </div>
        <div class="chips">
            {move || {
                let events = events();
                if events.is_empty() {
                    return view! { <p class="hint">"No moonrise or moonset this day."</p> }.into_any();
                }
                events
                    .into_iter()
                    .map(|(kind, t)| {
                        let label = match kind {
                            Crossing::Rise => "Moonrise",
                            Crossing::Set => "Moonset",
                        };
                        view! {
                            <button type="button" on:click=move |_| st.jump_to(t)>
                                {format!("{label} {}", st.fmt(t))}
                            </button>
                        }
                    })
                    .collect_view()
                    .into_any()
            }}
        </div>
    }
}

#[component]
fn MoonSheet(st: State) -> impl IntoView {
    let download = move |_| {
        let name = format!(
            "moon-{}-{}.geojson",
            slug(&st.landmark.with_untracked(|l| l.name.clone())),
            st.date.get_untracked(),
        );
        glue::download(&name, &st.computed.with_untracked(|c| c.1.clone()));
    };
    view! {
        {move || {
            let s = st.selected();
            let m = s.moon;
            view! {
                <dl class="readout">
                    <dt>"Position"</dt>
                    <dd>{format!("az {:.1}° · alt {:.1}°", m.pos.azimuth, m.pos.altitude)}</dd>
                    <dt>"Phase"</dt>
                    <dd>
                        {format!("{} · {:.0}% lit", phase_name(m.illumination, m.waxing), m.illumination * 100.0)}
                    </dd>
                    <dt>"Stand"</dt>
                    <dd>
                        {match s.stand {
                            Some(p) => format!("{:.1} km from summit · {:.5}, {:.5}", p.distance_km, p.lat, p.lon),
                            None if m.pos.altitude < 0.0 => "Moon is below the horizon".into(),
                            None => "No spot within range".into(),
                        }}
                    </dd>
                    <dt>"Sky"</dt>
                    <dd>{format!("{} · sun {:.1}°", sky(s.sun_alt), s.sun_alt)}</dd>
                </dl>
            }
        }}
        <HourTable st=st />
        <button type="button" class="primary" on:click=download>
            "Download GeoJSON"
        </button>
        <p class="hint">
            "Terrain is not modelled — check that nothing blocks the view."
        </p>
    }
}

#[component]
fn SetupSheet(st: State) -> impl IntoView {
    let zones = glue::time_zones();
    view! {
        <label>
            "Your elevation (m)"
            <input
                type="number"
                inputmode="decimal"
                step="1"
                prop:value=move || st.observer_elev.get().to_string()
                on:change=move |ev| {
                    if let Some(v) = parse_f64(&ev) {
                        st.observer_elev.set(v);
                    }
                }
            />
        </label>
        <label>
            "Alignment"
            <select
                prop:value=move || match st.align.get() {
                    Align::Center => "center",
                    Align::Resting => "resting",
                }
                on:change=move |ev| {
                    st.align.set(if event_target_value(&ev) == "resting" { Align::Resting } else { Align::Center })
                }
            >
                <option value="center">"Moon centred on summit"</option>
                <option value="resting">"Moon resting on summit"</option>
            </select>
        </label>
        <label>
            "Time zone"
            <input
                type="text"
                list="zones"
                autocapitalize="off"
                prop:value=move || st.tz.get()
                on:change=move |ev| {
                    let v = event_target_value(&ev);
                    if glue::is_valid_zone(&v) {
                        st.landmark.update(|l| l.tz = v);
                    }
                }
            />
            <datalist id="zones">
                {zones.into_iter().map(|z| view! { <option value=z /> }).collect_view()}
            </datalist>
        </label>
        <p class="hint">
            "Works offline once loaded; map tiles you've viewed are cached for offline use."
        </p>
    }
}

#[component]
fn HourTable(st: State) -> impl IntoView {
    let rows = move || {
        st.computed.with(|c| {
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
                                    <tr on:click=move |_| st.jump_to(s.t)>
                                        <td>{st.fmt(s.t)}</td>
                                        <td>{format!("{:.0}°", s.moon.pos.azimuth)}</td>
                                        <td>{format!("{:.1}°", s.moon.pos.altitude)}</td>
                                        <td>
                                            {s
                                                .stand
                                                .map(|p| format!("{:.0} km", p.distance_km))
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
