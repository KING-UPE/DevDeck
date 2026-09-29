# DevDeck rendezvous

Resolves a DevDeck installation's **current tunnel URL**.

Cloudflare quick tunnels hand out a new `*.trycloudflare.com` hostname every
time they restart. Without something in between, a paired phone would have to
be re-paired constantly. This service is that fixed point: a PC publishes its
current URL under a random id, and the phone resolves that id whenever it
cannot reach the PC directly.

Deploy it **once**; every DevDeck installation uses the same deployment.

## What it stores

One record per installation:

```
<random id>  ->  { url, token, updated_at }
```

That is the whole dataset. There are **no accounts, no emails, no usernames and
no folder paths** — nothing that identifies a person. The `id` is a 256-bit
capability generated on the user's own machine and never derived from anything
about them, so records cannot be correlated back to individuals.

The `url` is a public hostname that Cloudflare already hands out, and it still
sits behind DevDeck's own login. The `token` is write-only proof of ownership
and is never returned by a read.

Records expire after 7 days, so a machine that stops publishing stops being
advertised.

## Deploying

You need a free Cloudflare account. No paid plan, no domain.

```bash
cd worker
npm install
npx wrangler login
```

Create the KV namespace that holds the records:

```bash
npx wrangler kv namespace create RENDEZVOUS
```

Paste the returned id into `wrangler.toml`, replacing
`REPLACE_WITH_YOUR_KV_NAMESPACE_ID`, then:

```bash
npx wrangler deploy
```

Wrangler prints the deployed URL, e.g.
`https://devdeck-rendezvous.<your-subdomain>.workers.dev`.

## Pointing DevDeck at your deployment

`DEFAULT_SERVICE` in `src-tauri/src/rendezvous.rs` is **empty on purpose**, so
remote reconnect is off until someone configures it. A baked-in hostname would
mean every install publishes where its tunnel is reachable to whoever happens
to control that name.

Set it to the URL you just deployed, which you own, so your builds use it out
of the box. Individual installs can
override it at runtime through the `rendezvous_set_service` command.

## Free tier

Workers allows 100,000 requests/day and KV 1,000 writes/day on the free plan.
A DevDeck install writes once per tunnel start and reads a handful of times per
phone reconnect, so one deployment comfortably serves a large number of users
before any of that matters.

## Endpoints

| Method | Path | Purpose |
| --- | --- | --- |
| `PUT` | `/r/<id>` | Publish `{ "url": "..." }`. Requires `Authorization: Bearer <token>`. |
| `GET` | `/r/<id>` | Resolve to `{ url, updated_at }`. Never returns the token. |
| `DELETE` | `/r/<id>` | Withdraw. Requires the owner token. |
| `GET` | `/go/<id>` | 302 to the live URL — a stable address to bookmark. |
| `GET` | `/health` | Liveness check. |

An id is claimed on first publish; afterwards only the holder of the matching
token can change or delete it.

## Tests

The routing logic is written against a plain store, so the behaviour that
matters can be checked without a Workers runtime or a Cloudflare account:

```bash
npm test
```
