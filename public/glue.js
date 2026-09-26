// Thin browser glue for the Rust app: Leaflet map, time zones, downloads,
// geocoding. Exposed as `window.azimuthGlue` and bound via wasm-bindgen.
(function () {
  "use strict";

  let map = null;
  let dataLayer = null;
  let lastCenter = null;
  // Height (px) of the bottom sheet floating over the map; the map keeps its
  // points of interest centred in the clear area above it.
  let sheetInset = 0;
  let sheetObserver = null;

  const COLORS = {
    landmark: "#e4572e",
    path: "#4f86f7",
    sightline: "#4f86f7",
    moonrise: "#f2c14e",
    moonset: "#c97b3d",
    selected: "#f5f5f5",
    observer: "#f5f5f5",
  };

  function style(feature) {
    const k = feature.properties.kind;
    switch (k) {
      case "path":
        return { color: COLORS.path, weight: 4, opacity: 0.9 };
      case "sightline":
        return { color: COLORS.sightline, weight: 1.5, opacity: 0.55, dashArray: "4 6" };
      case "moonrise":
      case "moonset":
        return { color: COLORS[k], weight: 3, opacity: 0.9, dashArray: "10 6" };
      case "selected":
        return { color: "#111", weight: 5, opacity: 0.35, className: "sel-halo" };
      default:
        return { color: "#888", weight: 2 };
    }
  }

  function describe(p) {
    const rows = [];
    if (p.name) rows.push(`<b>${escapeHtml(p.name)}</b>`);
    if (p.kind === "landmark" && p.elevation_m != null) rows.push(`${Math.round(p.elevation_m)} m`);
    if (p.kind === "moonrise") rows.push(`Moonrise ${p.time}`);
    if (p.kind === "moonset") rows.push(`Moonset ${p.time}`);
    if (p.kind === "sightline" || p.kind === "selected" || p.kind === "observer") rows.push(`<b>${p.time}</b>`);
    if (p.azimuth != null) rows.push(`Az ${p.azimuth}°`);
    if (p.altitude != null && p.kind !== "moonrise" && p.kind !== "moonset") rows.push(`Alt ${p.altitude}°`);
    if (p.distance_km != null) rows.push(`${p.distance_km} km from summit`);
    return rows.join("<br>");
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  }

  function init(el, onClick) {
    map = L.map(el, { zoomControl: false, worldCopyJump: true });
    L.control.zoom({ position: "topright" }).addTo(map);
    L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", {
      maxZoom: 19,
      crossOrigin: "anonymous", // CORS responses can be cached without opaque-response quota padding
      attribution: '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a> contributors',
    }).addTo(map);
    L.control.scale({ imperial: true, metric: true }).addTo(map);
    map.attributionControl.addAttribution(
      'Terrain: <a href="https://github.com/tilezen/joerd/blob/master/docs/attribution.md">Mapzen, AWS</a>'
    );
    map.on("click", (e) => onClick(e.latlng.lat, e.latlng.lng));
    // The container may be resized by the layout after first paint.
    new ResizeObserver(() => map.invalidateSize()).observe(el);
    lockViewport();
  }

  /// Centre on a point within the clear area above the sheet.
  function centerOn(lat, lon, zoom) {
    const z = zoom ?? map.getZoom();
    const pt = map.project([lat, lon], z).add([0, sheetInset / 2]);
    const target = map.unproject(pt, z);
    if (zoom != null) map.setView(target, z);
    else map.panTo(target);
  }

  /// Follow the bottom sheet's height (null when closed): expose it to CSS as
  /// --sheet-h and pan the map so its centre stays visible above the sheet.
  function trackSheet(el) {
    if (sheetObserver) sheetObserver.disconnect();
    sheetObserver = null;
    const apply = (h) => {
      const delta = h - sheetInset;
      sheetInset = h;
      document.documentElement.style.setProperty("--sheet-h", `${h}px`);
      // Instant, not animated: an interrupted pan animation stops short and drifts.
      if (map && delta) map.panBy([0, delta / 2], { animate: false });
    };
    if (!el) return apply(0);
    // +8: the gap between the sheet and the toolbar (see .sheet in style.css).
    const measure = () => el.isConnected && apply(el.offsetHeight + 8);
    sheetObserver = new ResizeObserver(measure);
    sheetObserver.observe(el);
    measure(); // don't wait for the next frame's observer callback
  }

  /// iOS pans the whole page to make room for the keyboard or a native picker
  /// and doesn't always put it back, leaving the app offset and draggable.
  /// The page never scrolls by design, so snap it back and re-measure the map.
  function lockViewport() {
    const reset = () => {
      if (window.scrollX || window.scrollY) window.scrollTo(0, 0);
      const se = document.scrollingElement;
      if (se && (se.scrollTop || se.scrollLeft)) se.scrollTop = se.scrollLeft = 0;
      if (map) map.invalidateSize();
    };
    window.addEventListener("scroll", reset, { passive: true });
    // Belt and braces with the ResizeObserver: rotation, browser chrome
    // showing/hiding, and window resizes must all re-measure the map.
    window.addEventListener("resize", reset);
    window.addEventListener("orientationchange", () => setTimeout(reset, 300));
    document.addEventListener("focusout", () => setTimeout(reset, 50));
    if (window.visualViewport) window.visualViewport.addEventListener("resize", () => setTimeout(reset, 50));
  }

  function update(el, geojson, lat, lon, onClick) {
    if (!map) init(el, onClick);
    if (dataLayer) dataLayer.remove();
    const data = JSON.parse(geojson);

    dataLayer = L.geoJSON(data, {
      style,
      pointToLayer(feature, latlng) {
        const k = feature.properties.kind;
        if (k === "landmark") {
          return L.circleMarker(latlng, { radius: 8, color: "#fff", weight: 2, fillColor: COLORS.landmark, fillOpacity: 1 });
        }
        return L.circleMarker(latlng, { radius: 7, color: "#111", weight: 2, fillColor: "#f5f5f5", fillOpacity: 1 });
      },
      onEachFeature(feature, layer) {
        layer.bindTooltip(describe(feature.properties), { sticky: feature.geometry.type !== "Point" });
      },
    }).addTo(map);

    // Draw the selected line's bright core on top of its halo.
    data.features
      .filter((f) => f.properties.kind === "selected")
      .forEach((f) => {
        const ll = f.geometry.coordinates.map(([x, y]) => [y, x]);
        L.polyline(ll, { color: "#fff", weight: 2.5, opacity: 1, interactive: false }).addTo(dataLayer);
      });

    const key = `${lat.toFixed(5)},${lon.toFixed(5)}`;
    if (key !== lastCenter) {
      centerOn(lat, lon, lastCenter === null ? 9 : null);
      lastCenter = key;
    }
  }

  // --- time zones -------------------------------------------------------

  const fmtCache = new Map();
  function partsFormatter(tz) {
    if (!fmtCache.has(tz)) {
      fmtCache.set(
        tz,
        new Intl.DateTimeFormat("en-US", {
          timeZone: tz,
          hourCycle: "h23",
          year: "numeric",
          month: "2-digit",
          day: "2-digit",
          hour: "2-digit",
          minute: "2-digit",
          second: "2-digit",
        })
      );
    }
    return fmtCache.get(tz);
  }

  function wallClock(tz, ms) {
    const o = {};
    for (const p of partsFormatter(tz).formatToParts(new Date(ms))) o[p.type] = p.value;
    return { y: +o.year, mo: +o.month, d: +o.day, h: +o.hour, mi: +o.minute, s: +o.second };
  }

  function offsetMs(tz, ms) {
    const w = wallClock(tz, ms);
    return Date.UTC(w.y, w.mo - 1, w.d, w.h, w.mi, w.s) - Math.floor(ms / 1000) * 1000;
  }

  function isValidZone(tz) {
    try {
      new Intl.DateTimeFormat("en-US", { timeZone: tz });
      return true;
    } catch (_) {
      return false;
    }
  }

  /// Unix ms of `minutes` after local midnight of `date` (YYYY-MM-DD) in `tz`.
  function zonedTime(date, minutes, tz) {
    const [y, m, d] = date.split("-").map(Number);
    const naive = Date.UTC(y, m - 1, d) + minutes * 60000;
    let t = naive - offsetMs(tz, naive);
    t = naive - offsetMs(tz, t); // second pass settles DST transitions
    return t;
  }

  /// 12-hour wall-clock time in `tz`, e.g. "7:45 PM".
  function formatTime(ms, tz) {
    const w = wallClock(tz, ms);
    const h24 = w.h % 24; // some engines report midnight as 24
    return `${h24 % 12 || 12}:${String(w.mi).padStart(2, "0")} ${h24 < 12 ? "AM" : "PM"}`;
  }

  function zoneAbbrev(ms, tz) {
    const p = new Intl.DateTimeFormat("en-US", { timeZone: tz, timeZoneName: "short" })
      .formatToParts(new Date(ms))
      .find((x) => x.type === "timeZoneName");
    return p ? p.value : tz;
  }

  /// [YYYY-MM-DD, minutes since midnight] for "now" in `tz`.
  function nowIn(tz) {
    const w = wallClock(tz, Date.now());
    const date = `${w.y}-${String(w.mo).padStart(2, "0")}-${String(w.d).padStart(2, "0")}`;
    return [date, w.h * 60 + w.mi];
  }

  function timeZones() {
    return typeof Intl.supportedValuesOf === "function" ? Intl.supportedValuesOf("timeZone") : [];
  }

  // --- misc -------------------------------------------------------------

  function download(filename, text) {
    const url = URL.createObjectURL(new Blob([text], { type: "application/geo+json" }));
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.appendChild(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }

  /// GET a URL's body as bytes (terrain tiles; cached by the service worker).
  async function fetchBytes(url) {
    const res = await fetch(url);
    if (!res.ok) throw new Error(`${url}: ${res.status}`);
    return new Uint8Array(await res.arrayBuffer());
  }

  /// Nominatim lookup (online only). Resolves to JSON {name, lat, lon, ele} or null.
  /// Called on explicit submit only, per the Nominatim usage policy.
  async function search(q) {
    const url =
      "https://nominatim.openstreetmap.org/search?format=jsonv2&limit=1&extratags=1&q=" + encodeURIComponent(q);
    const res = await fetch(url, { headers: { Accept: "application/json" } });
    if (!res.ok) throw new Error(`search failed: ${res.status}`);
    const hits = await res.json();
    if (!hits.length) return null;
    const h = hits[0];
    const ele = h.extratags && parseFloat(h.extratags.ele);
    return JSON.stringify({
      name: h.name || h.display_name.split(",")[0],
      lat: parseFloat(h.lat),
      lon: parseFloat(h.lon),
      ele: Number.isFinite(ele) ? ele : null,
    });
  }

  window.azimuthGlue = {
    update,
    trackSheet,
    mapZoom: () => (map ? map.getZoom() : 9),
    zonedTime,
    formatTime,
    zoneAbbrev,
    nowIn,
    timeZones,
    isValidZone,
    download,
    fetchBytes,
    search,
  };
})();
