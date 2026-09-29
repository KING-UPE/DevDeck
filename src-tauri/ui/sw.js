/* DevDeck Remote service worker.
 *
 * Exists mainly to make the app installable, but it also keeps the shell usable
 * when the PC goes to sleep: the UI still opens and explains it is offline
 * rather than showing the browser's dinosaur.
 *
 * Deliberately narrow: only the app shell is cached. Project previews live on
 * other ports and must never be served stale, and /api/previews is live state.
 */

const SHELL = 'devdeck-shell-v1';
const ASSETS = ['/', '/manifest.webmanifest', '/icon-256.png', '/icon-512.png'];

self.addEventListener('install', (event) => {
  event.waitUntil(
    caches.open(SHELL).then((c) => c.addAll(ASSETS)).then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k !== SHELL).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', (event) => {
  const { request } = event;
  if (request.method !== 'GET') return;

  const url = new URL(request.url);

  // Only ever handle our own origin; previews are proxied on other ports.
  if (url.origin !== self.location.origin) return;

  // Live state must not be cached, but a failure should be a clean JSON empty
  // list so the UI renders its "can't reach DevDeck" state instead of throwing.
  if (url.pathname.startsWith('/api/')) {
    event.respondWith(
      fetch(request).catch(
        () =>
          new Response('[]', {
            status: 503,
            headers: { 'Content-Type': 'application/json' },
          })
      )
    );
    return;
  }

  // Shell: network first so a DevDeck update lands immediately, cache as backup.
  event.respondWith(
    fetch(request)
      .then((res) => {
        const copy = res.clone();
        caches.open(SHELL).then((c) => c.put(request, copy)).catch(() => {});
        return res;
      })
      .catch(() => caches.match(request).then((hit) => hit || caches.match('/')))
  );
});
