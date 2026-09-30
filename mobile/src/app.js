/* DevDeck Remote.
 *
 * Signs in to the account, lists the computers signed in to it, and controls
 * whichever one you pick. Two services are involved and they do different jobs:
 *
 *   the account service   says which computers exist and where each one is
 *   the computer itself   does the work - listing projects, starting, logs
 *
 * The account service never proxies control. It only answers "where is this
 * computer right now", and every command goes straight to that computer. So a
 * compromised account yields an address, not a dev server: the computer still
 * checks the session it issued.
 */
'use strict';

/* Shipped project. Public by design - this key carries no privileges of its
 * own, and Row Level Security is what protects the data. */
const CLOUD_URL = 'https://bedkemoieumrwlhkfnoh.supabase.co';
const CLOUD_KEY =
  'eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImJlZGtlbW9pZXVtcndsaGtmbm9oIiwicm9sZSI6ImFub24iLCJpYXQiOjE3OTA2Njk0NDgsImV4cCI6MjEwNjI0NTQ0OH0.B0L4hhCgBSAJ1JCpoINGWQD-vCcCHQpsfZ7QEMwxQwo';

const $ = (id) => document.getElementById(id);
const store = {
  get: (k) => { try { return localStorage.getItem(k); } catch (e) { return null; } },
  set: (k, v) => { try { localStorage.setItem(k, v); } catch (e) {} },
  del: (k) => { try { localStorage.removeItem(k); } catch (e) {} }
};

let session = null;      // { access_token, refresh_token, email }
let device = null;       // { name, base } - the computer being controlled
let deviceToken = null;  // session that computer issued us
let projects = [];

function at(screen) { document.body.dataset.at = screen; }
function esc(s) {
  return String(s == null ? '' : s).replace(/[&<>"]/g, (c) =>
    ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]));
}
function say(el, text, tone) {
  el.textContent = text || '';
  el.className = 'msg' + (tone ? ' ' + tone : '');
}

/* ------------------------------------------------------------------ account */

async function cloud(path, options) {
  const opts = options || {};
  const headers = Object.assign(
    { apikey: CLOUD_KEY, 'Content-Type': 'application/json' },
    opts.headers || {}
  );
  if (session && session.access_token) {
    headers.Authorization = 'Bearer ' + session.access_token;
  }
  const res = await fetch(CLOUD_URL + path, Object.assign({}, opts, { headers }));
  const text = await res.text();
  let body = null;
  try { body = text ? JSON.parse(text) : null; } catch (e) {}
  if (!res.ok) {
    const code = body && (body.error_code || body.code);
    throw new Error(friendly(res.status, code, body));
  }
  return body;
}

/* The service writes for developers; people need something else. */
function friendly(status, code, body) {
  const msg = (body && (body.msg || body.message || body.error_description)) || '';
  if (code === 'invalid_credentials' || code === 'invalid_grant') return 'Incorrect email or password.';
  if (code === 'email_not_confirmed') return 'Check your inbox and confirm your email first.';
  if (code === 'bad_jwt' || status === 401) return 'Your sign-in expired. Sign in again.';
  // Keep the server's wording: it is the only thing that says how long the
  // wait is, and an email limit lasts an hour where a sign-in limit lasts
  // minutes.
  if (code === 'over_email_send_rate_limit') {
    return 'Too many emails requested. ' + (msg || 'The mail service allows only a couple an hour.');
  }
  if (status === 429) return msg ? 'Too many attempts. ' + msg : 'Too many attempts. Wait a few minutes.';
  if (/already registered/i.test(msg)) return 'That email already has an account. Sign in instead.';
  return msg || ('Something went wrong (' + status + ').');
}

async function signIn(email, password) {
  const body = await cloud('/auth/v1/token?grant_type=password', {
    method: 'POST',
    body: JSON.stringify({ email: email, password: password })
  });
  if (!body || !body.access_token) throw new Error('The account service did not return a session.');
  session = {
    access_token: body.access_token,
    refresh_token: body.refresh_token,
    email: (body.user && body.user.email) || email
  };
  store.set('session', JSON.stringify(session));
}

async function signUp(email, password) {
  const body = await cloud('/auth/v1/signup', {
    method: 'POST',
    body: JSON.stringify({ email: email, password: password })
  });
  /* An existing address comes back as a user with no identities rather than an
   * error, so that sign-up cannot be used to discover who has an account. */
  if (body && Array.isArray(body.identities) && body.identities.length === 0) {
    return 'That email already has an account. Sign in instead.';
  }
  if (body && body.access_token) return 'Account created. You are signed in.';
  return 'Account created. Confirm the link in your email, then sign in.';
}

async function refreshSession() {
  if (!session || !session.refresh_token) return false;
  try {
    const body = await cloud('/auth/v1/token?grant_type=refresh_token', {
      method: 'POST',
      body: JSON.stringify({ refresh_token: session.refresh_token })
    });
    if (!body || !body.access_token) return false;
    session.access_token = body.access_token;
    session.refresh_token = body.refresh_token || session.refresh_token;
    store.set('session', JSON.stringify(session));
    return true;
  } catch (e) {
    return false;
  }
}

/* ------------------------------------------------------------------ devices */

async function loadDevices() {
  const el = $('devices');
  try {
    const rows = await cloud('/rest/v1/devices?select=name,tunnel_url,updated_at&order=updated_at.desc');
    renderDevices(rows || []);
  } catch (e) {
    /* An expired token is the common case; try once before giving up. */
    if (/expired/i.test(e.message) && (await refreshSession())) return loadDevices();
    el.innerHTML = '<div class="empty"><strong>Could not load your computers</strong><p>' +
      esc(e.message) + '</p></div>';
  }
}

function renderDevices(rows) {
  const el = $('devices');
  $('who').textContent = session ? session.email : '';

  const usable = rows.filter((r) => r.tunnel_url);
  if (!rows.length) {
    el.innerHTML =
      '<div class="empty">' +
      '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="2" y="3" width="20" height="14" rx="2"/><path d="M8 21h8M12 17v4"/></svg>' +
      '<strong>No computers yet</strong>' +
      '<p>Open DevDeck on your computer and sign in with this same account.</p></div>';
    return;
  }

  el.innerHTML = rows.map((r) => {
    const online = !!r.tunnel_url;
    return '<button class="card" ' + (online ? 'data-base="' + esc(r.tunnel_url) + '" ' : 'disabled ') +
      'data-name="' + esc(r.name) + '" style="' + (online ? '' : 'opacity:.55;') + '">' +
      '<span class="dot ' + (online ? 'on' : 'off') + '"></span>' +
      '<span style="flex:1;min-width:0;">' +
        '<span class="name">' + esc(r.name) + '</span>' +
        '<span class="meta">' + (online ? 'Online' : 'Offline — turn on "Use anywhere" on that computer') + '</span>' +
      '</span>' +
      (online ? '<span class="chev"><svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18l6-6-6-6"/></svg></span>' : '') +
    '</button>';
  }).join('') +
  (usable.length ? '' :
    '<p style="font-size:.8rem;color:var(--dim);text-align:center;margin-top:14px;line-height:1.5;">' +
    'A computer only appears online while its remote access is on. Open DevDeck there, ' +
    'go to Settings and turn on <strong>Use anywhere</strong>.</p>');
}

$('devices').addEventListener('click', (e) => {
  const card = e.target.closest('.card[data-base]');
  if (card) openDevice({ name: card.dataset.name, base: card.dataset.base });
});

/* ------------------------------------------------------- the chosen computer */

/* Every request to a computer carries the session that computer issued us. */
async function box(path, options) {
  const opts = options || {};
  const headers = Object.assign({}, opts.headers || {});
  if (deviceToken) headers.Authorization = 'Bearer ' + deviceToken;
  const res = await fetch(device.base.replace(/\/$/, '') + path,
    Object.assign({}, opts, { headers: headers, cache: 'no-store' }));
  if (res.status === 401) throw new Error('unauthorised');
  if (!res.ok) {
    let body = null;
    try { body = await res.json(); } catch (e) {}
    throw new Error((body && body.error) || ('Request failed (' + res.status + ')'));
  }
  const text = await res.text();
  try { return text ? JSON.parse(text) : null; } catch (e) { return null; }
}

/* Trade the account sign-in for a session on that computer. It verifies the
 * token with the account service itself before trusting it. */
async function authoriseWithDevice() {
  const res = await fetch(device.base.replace(/\/$/, '') + '/api/cloud-login', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ access_token: session.access_token })
  });
  const body = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(body.error || 'That computer refused the sign-in.');
  deviceToken = body.token || null;
  if (deviceToken) store.set('deviceToken:' + device.base, deviceToken);
}

async function openDevice(d) {
  device = d;
  deviceToken = store.get('deviceToken:' + d.base);
  $('dev-name').textContent = d.name;
  $('dev-sub').textContent = 'connecting…';
  $('projects').innerHTML = '<div class="skel"></div><div class="skel"></div>';
  at('projects');

  try {
    if (!deviceToken) await authoriseWithDevice();
    await loadProjects();
  } catch (e) {
    if (e.message === 'unauthorised') {
      /* The stored session was revoked on that computer; ask for a new one. */
      try { await authoriseWithDevice(); await loadProjects(); return; }
      catch (err) { e = err; }
    }
    $('dev-sub').textContent = 'not connected';
    $('projects').innerHTML =
      '<div class="empty"><strong>Could not reach that computer</strong><p>' +
      esc(e.message) + '</p></div>';
  }
}

async function loadProjects() {
  projects = (await box('/api/projects')) || [];
  $('dev-sub').textContent = projects.length + (projects.length === 1 ? ' project' : ' projects');
  renderProjects();
}

const openPaths = {};

function renderProjects() {
  const el = $('projects');
  if (!projects.length) {
    el.innerHTML =
      '<div class="empty"><strong>No projects</strong>' +
      '<p>Add a workspace on that computer and scan it.</p></div>';
    return;
  }

  el.innerHTML = projects.map((r) => {
    const scripts = r.scripts.map((sc) => {
      const key = r.path + ':' + sc.name;
      if (sc.running) {
        return '<div class="script">' +
          '<span class="dot on"></span>' +
          '<span class="script-name">' + esc(sc.name) + '</span>' +
          '<button class="mini go" data-act="open" data-port="' + sc.port + '" data-name="' + esc(r.name) + '">Preview</button>' +
          '<button class="mini icon" data-act="logs" data-key="' + esc(key) + '" aria-label="Logs">' +
            '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 6h16M4 12h10M4 18h13"/></svg>' +
          '</button>' +
          '<button class="mini icon" data-act="stop" data-key="' + esc(key) + '" aria-label="Stop">' +
            '<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><rect x="6" y="6" width="12" height="12" rx="2"/></svg>' +
          '</button>' +
        '</div>';
      }
      return '<div class="script">' +
        '<span class="script-name">' + esc(sc.name) + '</span>' +
        '<button class="mini go" data-act="start" data-key="' + esc(key) + '">Start</button>' +
      '</div>';
    }).join('');

    return '<div class="proj' + (openPaths[r.path] ? ' open' : '') + '" data-path="' + esc(r.path) + '">' +
      '<button class="proj-head" data-act="toggle">' +
        (r.running ? '<span class="dot on"></span>' : '') +
        '<span style="flex:1;min-width:0;">' +
          '<span class="proj-name">' + esc(r.name) + '</span>' +
          '<span class="proj-kind">' + esc(r.kind || 'Project') + '</span>' +
        '</span>' +
        '<span class="caret"><svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 18l6-6-6-6"/></svg></span>' +
      '</button>' +
      '<div class="proj-body">' + scripts + '</div>' +
    '</div>';
  }).join('');
}

$('projects').addEventListener('click', async (e) => {
  const btn = e.target.closest('[data-act]');
  if (!btn) return;
  const act = btn.dataset.act;

  if (act === 'toggle') {
    const card = btn.closest('.proj');
    openPaths[card.dataset.path] = !openPaths[card.dataset.path];
    card.classList.toggle('open');
    return;
  }
  if (act === 'open') { openPreview(btn.dataset.port, btn.dataset.name); return; }
  if (act === 'logs') { openLogs(btn.dataset.key); return; }

  const original = btn.textContent;
  btn.disabled = true;
  if (act === 'start') btn.textContent = 'Starting…';
  try {
    await box('/api/process/' + act + '?key=' + encodeURIComponent(btn.dataset.key), { method: 'POST' });
    /* A dev server needs a moment to bind before its preview exists. */
    setTimeout(loadProjects, act === 'start' ? 1600 : 500);
  } catch (err) {
    alert(err.message);
  } finally {
    btn.disabled = false;
    btn.textContent = original;
  }
});

/* ------------------------------------------------------------------ preview */

/* Each preview needs its own public address.
 *
 * A tunnel maps one port to one hostname, so the control plane's tunnel cannot
 * also carry previews, and path-prefixing them under it breaks every dev
 * server that requests assets from an absolute path. The computer opens a
 * tunnel for the project on demand and hands back the URL. */
const shareCache = {};

async function openPreview(port, name) {
  $('prev-name').textContent = name;
  $('prev-url').textContent = 'opening a public address…';
  $('frame').src = 'about:blank';
  at('preview');

  const key = currentKeyForPort(port);
  try {
    let url = shareCache[key];
    if (!url) {
      const body = await box('/api/share?key=' + encodeURIComponent(key), { method: 'POST' });
      url = body && body.url;
      if (!url) throw new Error('No address came back.');
      shareCache[key] = url;
    }
    $('prev-url').textContent = url;
    $('frame').src = url;
  } catch (e) {
    $('prev-url').textContent = 'could not open';
    $('frame').src = 'about:blank';
    alert('Could not get a public address for this project.\n\n' + e.message);
  }
}

/* Map a running port back to its process key, which is what the API takes. */
function currentKeyForPort(port) {
  for (const p of projects) {
    for (const sc of p.scripts) {
      if (String(sc.port) === String(port)) return p.path + ':' + sc.name;
    }
  }
  return '';
}

$('prev-back').addEventListener('click', () => { $('frame').src = 'about:blank'; at('projects'); });
$('prev-reload').addEventListener('click', function () {
  const f = $('frame');
  if (!f.src || f.src === 'about:blank') return;
  f.src = f.src.split('?')[0] + '?_r=' + Date.now();
  this.firstElementChild.classList.add('spin');
  setTimeout(() => this.firstElementChild.classList.remove('spin'), 600);
});
$('prev-share').addEventListener('click', async function () {
  try {
    await navigator.clipboard.writeText($('prev-url').textContent);
    this.setAttribute('aria-label', 'Copied');
    setTimeout(() => this.setAttribute('aria-label', 'Copy link'), 1200);
  } catch (e) { alert($('prev-url').textContent); }
});

/* --------------------------------------------------------------------- logs */

let logKey = null, logSeq = null, logStop = false;

function appendLines(lines) {
  if (!lines.length) return;
  const el = $('log-out');
  /* Only autoscroll when already at the bottom, so reading history is not
     yanked away every time a line arrives. */
  const stick = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
  const blank = el.querySelector('.empty');
  if (blank) blank.remove();

  const frag = document.createDocumentFragment();
  lines.forEach((l) => {
    const span = document.createElement('span');
    if (l.stream === 'stderr') span.className = 'e';
    span.textContent = l.text + '\n';
    frag.appendChild(span);
    logSeq = l.seq;
  });
  el.appendChild(frag);
  while (el.childNodes.length > 600) el.removeChild(el.firstChild);
  if (stick) el.scrollTop = el.scrollHeight;
  $('log-sub').textContent = 'live';
}

async function pumpLogs() {
  while (!logStop && logKey) {
    try {
      const q = '/api/logs/poll?key=' + encodeURIComponent(logKey) +
                (logSeq === null ? '' : '&after=' + logSeq);
      appendLines((await box(q)) || []);
    } catch (e) {
      $('log-sub').textContent = 'disconnected';
      await new Promise((r) => setTimeout(r, 2000));
    }
  }
}

async function openLogs(key) {
  logKey = key; logSeq = null; logStop = false;
  $('log-out').innerHTML = '<span class="empty">Waiting for output...</span>';
  $('log-name').textContent = key.slice(key.lastIndexOf(':') + 1) || 'Logs';
  $('log-sub').textContent = 'connecting';
  at('logs');
  try { appendLines((await box('/api/logs?key=' + encodeURIComponent(key))) || []); } catch (e) {}
  pumpLogs();
}

$('log-back').addEventListener('click', () => { logStop = true; logKey = null; at('projects'); });
$('log-bottom').addEventListener('click', () => { $('log-out').scrollTop = $('log-out').scrollHeight; });

async function control(action, btn, busy) {
  if (!logKey) return;
  const original = btn.textContent;
  btn.disabled = true; btn.textContent = busy;
  try {
    await box('/api/process/' + action + '?key=' + encodeURIComponent(logKey), { method: 'POST' });
    if (action === 'restart') {
      logSeq = null;
      $('log-out').innerHTML = '<span class="empty">Restarting...</span>';
    }
  } catch (e) {
    $('log-sub').textContent = e.message;
  } finally {
    btn.disabled = false; btn.textContent = original;
  }
}
$('log-restart').addEventListener('click', function () { control('restart', this, 'Restarting…'); });
$('log-stop').addEventListener('click', function () { control('stop', this, 'Stopping…'); });

/* ------------------------------------------------------------------- sign in */

let mode = 'in';
function setMode(next) {
  mode = next;
  $('tab-in').classList.toggle('on', next === 'in');
  $('tab-up').classList.toggle('on', next === 'up');
  /* Re-entering a password guards account creation only; at sign-in it is noise. */
  $('pass2').style.display = next === 'up' ? 'block' : 'none';
  $('forgot').style.display = next === 'up' ? 'none' : 'inline';
  $('go').textContent = next === 'up' ? 'Create account' : 'Sign in';
  say($('auth-msg'), '');
}
$('tab-in').addEventListener('click', () => setMode('in'));
$('tab-up').addEventListener('click', () => setMode('up'));

$('eye').addEventListener('click', () => {
  const f = $('pass');
  const showing = f.type === 'text';
  f.type = showing ? 'password' : 'text';
  $('eye').setAttribute('aria-label', showing ? 'Show password' : 'Hide password');
});

$('go').addEventListener('click', async function () {
  const email = $('email').value.trim();
  const pass = $('pass').value;
  const msg = $('auth-msg');
  if (!email || !pass) { say(msg, 'Enter your email and password.', 'bad'); return; }
  if (mode === 'up') {
    if (pass.length < 8) { say(msg, 'Use at least 8 characters.', 'bad'); return; }
    if (pass !== $('pass2').value) { say(msg, 'The two passwords do not match.', 'bad'); return; }
  }

  const original = this.textContent;
  this.disabled = true;
  this.textContent = mode === 'up' ? 'Creating…' : 'Signing in…';
  say(msg, '');
  try {
    if (mode === 'up') {
      /* Stay here and switch to sign-in rather than parking on a dead end. */
      const note = await signUp(email, pass);
      setMode('in');
      say(msg, note, 'good');
    } else {
      await signIn(email, pass);
      $('pass').value = '';
      at('devices');
      loadDevices();
    }
  } catch (e) {
    say(msg, e.message, 'bad');
  } finally {
    this.disabled = false;
    this.textContent = original;
  }
});
$('pass').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('go').click(); });
$('pass2').addEventListener('keydown', (e) => { if (e.key === 'Enter') $('go').click(); });

$('forgot').addEventListener('click', async function () {
  const email = $('email').value.trim();
  const msg = $('auth-msg');
  if (!email) { say(msg, 'Enter your email first, then press this.', 'bad'); return; }
  this.disabled = true;
  try {
    await cloud('/auth/v1/recover', { method: 'POST', body: JSON.stringify({ email: email }) });
    say(msg, 'If that address has an account, a reset link is on its way.');
  } catch (e) {
    say(msg, e.message, 'bad');
  } finally { this.disabled = false; }
});

/* Pairing by code, for getting in without typing a password. The code is the
 * pairing URL shown on the computer; scanning it is the same thing. */
$('link-device').addEventListener('click', async () => {
  const raw = prompt('Paste the link shown under Settings → Link a device on your computer:');
  if (!raw) return;
  let url;
  try { url = new URL(raw.trim()); } catch (e) { alert('That does not look like a link.'); return; }

  const token = url.hash.replace(/^#/, '') || url.searchParams.get('t');
  const base = url.origin;
  if (!token) { alert('That link has no pairing code in it.'); return; }

  /* Exchange the pairing token for a session on that computer. */
  try {
    const res = await fetch(base + '/?t=' + encodeURIComponent(token), { redirect: 'manual' });
    if (res.status >= 400) throw new Error('That computer rejected the code.');
    device = { name: url.hostname, base: base };
    deviceToken = null;
    at('projects');
    await loadProjects();
  } catch (e) {
    alert(e.message || 'Could not reach that computer.');
  }
});

$('proj-back').addEventListener('click', () => at('devices'));
$('dev-refresh').addEventListener('click', function () {
  this.firstElementChild.classList.add('spin');
  setTimeout(() => this.firstElementChild.classList.remove('spin'), 600);
  loadDevices();
});
$('proj-refresh').addEventListener('click', function () {
  this.firstElementChild.classList.add('spin');
  setTimeout(() => this.firstElementChild.classList.remove('spin'), 600);
  loadProjects().catch(() => {});
});
$('sign-out').addEventListener('click', () => {
  store.del('session');
  session = null; device = null; deviceToken = null;
  setMode('in');
  at('auth');
});

/* --------------------------------------------------------------- lifecycle */

(function boot() {
  const saved = store.get('session');
  if (saved) {
    try { session = JSON.parse(saved); } catch (e) { session = null; }
  }
  if (session && session.access_token) {
    at('devices');
    loadDevices();
  } else {
    at('auth');
    setMode('in');
  }
})();
