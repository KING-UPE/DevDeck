/**
 * Tests for the rendezvous routing logic.
 *
 * Runs on plain Node against a fake store, so there is no Workers runtime or
 * Cloudflare account needed to check the behaviour that matters: that one
 * user cannot overwrite or read another's record.
 */
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { handle } from '../src/index.js';

function fakeStore() {
  const map = new Map();
  return {
    map,
    get: async (k) => (map.has(k) ? JSON.parse(map.get(k)) : null),
    put: async (k, v) => void map.set(k, JSON.stringify(v)),
    delete: async (k) => void map.delete(k),
  };
}

const ID = 'a'.repeat(64);
const TOKEN = 'b'.repeat(64);
const OTHER = 'c'.repeat(64);
const URL_A = 'https://frozen-maple-42.trycloudflare.com';
const URL_B = 'https://quiet-river-7.trycloudflare.com';

const put = (id, token, url) =>
  new Request(`https://rv.test/r/${id}`, {
    method: 'PUT',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ url }),
  });

const get = (id) => new Request(`https://rv.test/r/${id}`);

test('publishes then resolves a URL', async () => {
  const s = fakeStore();
  assert.equal((await handle(put(ID, TOKEN, URL_A), s)).status, 200);

  const res = await handle(get(ID), s);
  assert.equal(res.status, 200);
  assert.equal((await res.json()).url, URL_A);
});

test('never returns the write token on a read', async () => {
  const s = fakeStore();
  await handle(put(ID, TOKEN, URL_A), s);
  const body = await (await handle(get(ID), s)).json();
  assert.equal(body.token, undefined, 'token leaked to readers');
  assert.ok(!JSON.stringify(body).includes(TOKEN));
});

test('another user cannot hijack an id', async () => {
  const s = fakeStore();
  await handle(put(ID, TOKEN, URL_A), s);

  const res = await handle(put(ID, OTHER, URL_B), s);
  assert.equal(res.status, 401, 'someone else overwrote the record');

  const body = await (await handle(get(ID), s)).json();
  assert.equal(body.url, URL_A, 'record was changed despite the 401');
});

test('the owner can update their own record', async () => {
  const s = fakeStore();
  await handle(put(ID, TOKEN, URL_A), s);
  assert.equal((await handle(put(ID, TOKEN, URL_B), s)).status, 200);
  assert.equal((await (await handle(get(ID), s)).json()).url, URL_B);
});

test('unknown ids are a 404, not an empty success', async () => {
  const s = fakeStore();
  assert.equal((await handle(get(ID), s)).status, 404);
});

test('rejects a short or missing token', async () => {
  const s = fakeStore();
  assert.equal((await handle(put(ID, 'short', URL_A), s)).status, 401);

  const noAuth = new Request(`https://rv.test/r/${ID}`, {
    method: 'PUT',
    body: JSON.stringify({ url: URL_A }),
  });
  assert.equal((await handle(noAuth, s)).status, 401);
});

test('rejects ids that are not capability-shaped', async () => {
  const s = fakeStore();
  for (const bad of ['short', '../etc/passwd', 'a'.repeat(200)]) {
    const res = await handle(new Request(`https://rv.test/r/${encodeURIComponent(bad)}`), s);
    assert.ok(res.status === 400 || res.status === 404, `accepted id: ${bad}`);
  }
});

test('rejects anything that is not a plain http(s) URL', async () => {
  const s = fakeStore();
  for (const bad of [
    'javascript:alert(1)',
    'file:///etc/passwd',
    'https://user:pass@evil.test',
    '',
    'not a url',
  ]) {
    const res = await handle(put(ID, TOKEN, bad), s);
    assert.equal(res.status, 400, `accepted url: ${bad}`);
  }
});

test('delete requires the owner token', async () => {
  const s = fakeStore();
  await handle(put(ID, TOKEN, URL_A), s);

  const del = (token) =>
    new Request(`https://rv.test/r/${ID}`, {
      method: 'DELETE',
      headers: { Authorization: `Bearer ${token}` },
    });

  assert.equal((await handle(del(OTHER), s)).status, 401);
  assert.equal((await handle(del(TOKEN), s)).status, 200);
  assert.equal((await handle(get(ID), s)).status, 404);
});

test('/go redirects to the live URL', async () => {
  const s = fakeStore();
  await handle(put(ID, TOKEN, URL_A), s);

  const res = await handle(new Request(`https://rv.test/go/${ID}`), s);
  assert.equal(res.status, 302);
  assert.equal(res.headers.get('location'), URL_A);
});

test('CORS is present so the phone can call it cross-origin', async () => {
  const s = fakeStore();
  await handle(put(ID, TOKEN, URL_A), s);
  const res = await handle(get(ID), s);
  assert.equal(res.headers.get('access-control-allow-origin'), '*');

  const pre = await handle(new Request(`https://rv.test/r/${ID}`, { method: 'OPTIONS' }), s);
  assert.equal(pre.status, 204);
});

test('two users do not collide', async () => {
  const s = fakeStore();
  const id2 = 'd'.repeat(64);
  await handle(put(ID, TOKEN, URL_A), s);
  await handle(put(id2, OTHER, URL_B), s);

  assert.equal((await (await handle(get(ID), s)).json()).url, URL_A);
  assert.equal((await (await handle(get(id2), s)).json()).url, URL_B);
});
