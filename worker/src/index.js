/**
 * DevDeck rendezvous.
 *
 * Quick tunnels hand out a new hostname on every restart, so a paired phone
 * would have to be re-paired constantly. This service is the one fixed address
 * in between: a PC publishes its current tunnel URL under an id, and the phone
 * resolves that id whenever it cannot reach the PC directly.
 *
 * It deliberately stores NO personal data — no accounts, no emails, no folder
 * paths, nothing a user could be identified by. One record is:
 *
 *     <random id>  ->  { url, token, updated_at }
 *
 * The id is a 256-bit random capability generated on the PC. Holding it lets
 * you read the current tunnel URL, which is a public hostname anyway and still
 * sits behind DevDeck's own login. The token is write-only proof of ownership
 * and is never returned by a read.
 *
 * Endpoints:
 *   PUT    /r/<id>   { url }   publish, Bearer token required
 *   GET    /r/<id>             resolve to { url, updated_at }
 *   DELETE /r/<id>             withdraw, Bearer token required
 *   GET    /go/<id>            302 to the live URL, for a home-screen shortcut
 */

/** Records expire so a machine that never comes back stops being advertised. */
const TTL_SECONDS = 60 * 60 * 24 * 7;

/** ids and tokens are hex-encoded 32-byte values produced by the desktop app. */
const ID_RE = /^[A-Za-z0-9_-]{16,128}$/;

const CORS = {
  'Access-Control-Allow-Origin': '*',
  'Access-Control-Allow-Methods': 'GET, PUT, DELETE, OPTIONS',
  'Access-Control-Allow-Headers': 'Content-Type, Authorization',
  'Access-Control-Max-Age': '86400',
};

function json(body, status = 200, extra = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json', ...CORS, ...extra },
  });
}

/**
 * Compare two secrets without leaking their contents through timing.
 *
 * Always walks the full length of the longer string so an early mismatch is
 * not faster than a late one.
 */
function secretsMatch(a, b) {
  if (typeof a !== 'string' || typeof b !== 'string') return false;
  const len = Math.max(a.length, b.length);
  let diff = a.length ^ b.length;
  for (let i = 0; i < len; i++) {
    diff |= a.charCodeAt(i % (a.length || 1)) ^ b.charCodeAt(i % (b.length || 1));
  }
  return diff === 0;
}

function bearer(request) {
  const raw = request.headers.get('authorization') || '';
  const m = raw.match(/^Bearer\s+(.+)$/i);
  return m ? m[1].trim() : '';
}

/** Only http(s), and no credentials smuggled into the authority. */
function isPublishableUrl(value) {
  if (typeof value !== 'string' || value.length > 2048) return false;
  let u;
  try {
    u = new URL(value);
  } catch {
    return false;
  }
  if (u.protocol !== 'https:' && u.protocol !== 'http:') return false;
  if (u.username || u.password) return false;
  return true;
}

/**
 * Core routing, written against a plain store so it can be tested without a
 * Workers runtime. `store` needs get(key) / put(key, value, ttl) / delete(key).
 */
export async function handle(request, store) {
  const url = new URL(request.url);

  if (request.method === 'OPTIONS') {
    return new Response(null, { status: 204, headers: CORS });
  }

  if (url.pathname === '/health') {
    return json({ ok: true });
  }

  const go = url.pathname.match(/^\/go\/([^/]+)$/);
  if (go && request.method === 'GET') {
    if (!ID_RE.test(go[1])) return json({ error: 'bad id' }, 400);
    const rec = await store.get(go[1]);
    if (!rec) return json({ error: 'not found' }, 404);
    return new Response(null, {
      status: 302,
      headers: { Location: rec.url, 'Cache-Control': 'no-store', ...CORS },
    });
  }

  const match = url.pathname.match(/^\/r\/([^/]+)$/);
  if (!match) return json({ error: 'not found' }, 404);

  const id = match[1];
  if (!ID_RE.test(id)) return json({ error: 'bad id' }, 400);

  if (request.method === 'GET') {
    const rec = await store.get(id);
    if (!rec) return json({ error: 'not found' }, 404);
    // Never echo the write token back.
    return json({ url: rec.url, updated_at: rec.updated_at }, 200, { 'Cache-Control': 'no-store' });
  }

  if (request.method === 'PUT') {
    const token = bearer(request);
    if (token.length < 32) return json({ error: 'unauthorized' }, 401);

    let body;
    try {
      body = await request.json();
    } catch {
      return json({ error: 'body must be JSON' }, 400);
    }
    if (!isPublishableUrl(body && body.url)) {
      return json({ error: 'url must be an absolute http(s) URL' }, 400);
    }

    const existing = await store.get(id);
    // Trust on first use: whoever claims an unused id owns it from then on.
    if (existing && !secretsMatch(existing.token, token)) {
      return json({ error: 'unauthorized' }, 401);
    }

    const updated_at = Math.floor(Date.now() / 1000);
    await store.put(id, { url: body.url, token, updated_at }, TTL_SECONDS);
    return json({ ok: true, updated_at });
  }

  if (request.method === 'DELETE') {
    const token = bearer(request);
    const existing = await store.get(id);
    if (!existing) return json({ ok: true });
    if (!secretsMatch(existing.token, token)) return json({ error: 'unauthorized' }, 401);
    await store.delete(id);
    return json({ ok: true });
  }

  return json({ error: 'method not allowed' }, 405);
}

/** Adapts Cloudflare KV to the small store interface `handle` expects. */
function kvStore(env) {
  return {
    get: (k) => env.RENDEZVOUS.get(k, 'json'),
    put: (k, v, ttl) => env.RENDEZVOUS.put(k, JSON.stringify(v), { expirationTtl: ttl }),
    delete: (k) => env.RENDEZVOUS.delete(k),
  };
}

export default {
  async fetch(request, env) {
    try {
      return await handle(request, kvStore(env));
    } catch (err) {
      return json({ error: 'internal error' }, 500);
    }
  },
};
