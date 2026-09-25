// Offline support. The precache list and cache version placeholders below are
// filled in at build time by scripts/sw-manifest.sh (a Trunk post_build hook).
const PRECACHE = __PRECACHE__;
const SHELL = "azimuth-shell-__VERSION__";
const TILES = "azimuth-tiles-v1";
const MAX_TILES = 4000;

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
      .then((keys) => Promise.all(keys.filter((k) => k !== SHELL && k !== TILES).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);

  if (url.hostname === "tile.openstreetmap.org") {
    event.respondWith(tile(req, event));
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

// Map tiles: cache first. Only tiles you've viewed are stored — the OSM tile
// policy forbids bulk prefetching.
async function tile(req, event) {
  const cache = await caches.open(TILES);
  const hit = await cache.match(req);
  if (hit) return hit;
  const res = await fetch(req);
  if (res.ok) {
    event.waitUntil(cache.put(req, res.clone()).then(() => trim(cache)));
  }
  return res;
}

let trimming = false;
async function trim(cache) {
  if (trimming) return;
  trimming = true;
  try {
    const keys = await cache.keys();
    const excess = keys.length - MAX_TILES;
    for (let i = 0; i < excess; i++) await cache.delete(keys[i]);
  } finally {
    trimming = false;
  }
}
