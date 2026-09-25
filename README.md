# azimuth

Photography utility for planning **moon-over-landmark** shots. Pick a landmark
(default: Mount Hood) and a date, and the app draws on an OpenStreetMap map:

- **Where to stand** (blue): the path of spots from which the moon sits on the
  summit as it crosses the sky. It accounts for summit/observer elevation, Earth
  curvature, and refraction.
- **Moonrise / moonset** (dashed): the direction of the moon from the landmark
  when it rises and sets.
- **Selected time** (white): the line from observer to summit to moon for the
  time on the slider.

All of this is generated as GeoJSON, which you can download.

Built with [Leptos](https://leptos.dev) (CSR, Rust → WebAssembly) and Leaflet.
It's an installable PWA that works offline: the app shell is precached, and map
tiles you've viewed are cached (the OSM tile policy forbids bulk prefetching).
Place search uses Nominatim and needs a connection.

## Develop

```sh
rustup target add wasm32-unknown-unknown
cargo install trunk
trunk serve --open
cargo test          # astronomy + geometry unit tests
```

## Deploy

Every push to `main` builds and publishes to GitHub Pages via
`.github/workflows/pages.yml`. In the repo's settings, set
**Settings → Pages → Source** to **GitHub Actions**.

## Layout

| Path | What |
| --- | --- |
| `src/astro.rs` | Moon (truncated Meeus ch. 47) and Sun positions, rise/set search |
| `src/plan.rs` | Standing-distance solver, GeoJSON generation |
| `src/app.rs` | Leptos UI |
| `public/glue.js` | Leaflet map, `Intl` time zones, download, Nominatim search |
| `public/sw.js` | Service worker; precache list is filled in by `scripts/sw-manifest.sh` |
| `public/vendor/leaflet` | Vendored Leaflet 1.9.4 (BSD-2) so the map works offline |
