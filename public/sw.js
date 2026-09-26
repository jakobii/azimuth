// Offline support. The precache list and cache version placeholders below are
// filled in at build time by scripts/sw-manifest.sh (a Trunk post_build hook).
const PRECACHE = __PRECACHE__;
const SHELL = "azimuth-shell-__VERSION__";
const TILES = "azimuth-tiles-v1";
const MAX_TILES = 4000;
// Elevation tiles (AWS Terrain Tiles) used for ground heights and summits.
const TERRAIN = "azimuth-terrain-v1";
const MAX_TERRAIN = 1500;
const TERRAIN_PREFIX = "https://s3.amazonaws.com/elevation-tiles-prod/terrarium/";

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(SHELL)
      .then((c) => c.addAll(["./", ...PRECACHE]))
      .then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => ![SHELL, TILES, TERRAIN].includes(k)).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);

  if (url.hostname === "tile.openstreetmap.org") {
    event.respondWith(tile(req, event, TILES, MAX_TILES));
  } else if (req.url.startsWith(TERRAIN_PREFIX)) {
    event.respondWith(tile(req, event, TERRAIN, MAX_TERRAIN));
  } else if (url.origin === self.location.origin) {
    event.respondWith(shell(req));
  }
  // Everything else (e.g. Nominatim search) goes straight to the network.
});

// App files: network first so deploys show up immediately; cache when offline.
async function shell(req) {
  const cache = await caches.open(SHELL);
  try {
    // Revalidate so unhashed files (glue.js, vendor/) never lag a deploy.
    // (By URL: a navigate-mode Request can't be re-initialised with options.)
    const res = await fetch(req.url, { cache: "no-cache" });
    if (res.ok) cache.put(req, res.clone());
    return res;
  } catch (err) {
    const hit = (await cache.match(req, { ignoreSearch: true })) || (req.mode === "navigate" && (await cache.match("./")));
    if (hit) return hit;
    throw err;
  }
}

// Map and terrain tiles: cache first. Only tiles actually used are stored —
// the OSM tile policy forbids bulk prefetching.
async function tile(req, event, name, max) {
  const cache = await caches.open(name);
  const hit = await cache.match(req);
  if (hit) return hit;
  const res = await fetch(req);
  if (res.ok) {
    event.waitUntil(cache.put(req, res.clone()).then(() => trim(cache, name, max)));
  }
  return res;
}

const trimming = new Set();
async function trim(cache, name, max) {
  if (trimming.has(name)) return;
  trimming.add(name);
  try {
    const keys = await cache.keys();
    const excess = keys.length - max;
    for (let i = 0; i < excess; i++) await cache.delete(keys[i]);
  } finally {
    trimming.delete(name);
  }
}
