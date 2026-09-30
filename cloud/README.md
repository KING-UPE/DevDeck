# DevDeck cloud accounts

One login that works on every device: sign up once, sign in on your phone from
any network, and it finds your PC.

Backed by [Supabase](https://supabase.com) — Postgres with authentication built
in. Signup, email confirmation, password reset and token issuing are handled
there rather than hand-rolled, because subtly wrong credential handling is how
people get breached.

## What is stored in the cloud

| Table | Holds |
| --- | --- |
| `auth.users` | email, password hash, verification state (managed by Supabase) |
| `devices` | a machine's name, its current tunnel URL, when it was last seen |

That is the whole dataset.

**Workspaces, project paths, custom names and logs never leave the user's
machine.** They are absolute paths that mean nothing on another computer, and
they leak client names, employers and unreleased work. The cloud's only job is
answering *"where is this account's PC right now"*.

## Already configured

Builds ship pointing at a Supabase project, so cloud sign-in works out of the
box with no setup. The steps below are only needed to run **your own** project
instead - for a fork, or to keep your users' accounts separate.

## Setup (optional)

You need a free Supabase account. No card, no paid plan.

**1. Create a project** at [supabase.com/dashboard](https://supabase.com/dashboard).
Pick a region near your users.

**2. Create the tables.** Open **SQL Editor → New query**, paste the contents of
[`schema.sql`](schema.sql), and **Run**. It is idempotent, so re-running later
is safe.

**3. Copy your project's details.** **Project Settings → API** gives you:

- **Project URL** — `https://<something>.supabase.co`
- **anon / public key** — a long JWT

**4. Point DevDeck at it.** In the desktop app: **Mobile Remote → Cloud
account → Connect a project**, and paste both values.

> The anon key is meant to be public — it ships in every Supabase browser
> client and carries no privileges of its own. Row Level Security is what
> protects the data. Never ship the **service_role** key; it bypasses RLS
> entirely.

**5. Sign up**, then sign in on your phone with the same account.

## How a sign-in actually works

```
phone ──1── email + password ──────────► Supabase        (password never reaches the PC)
phone ◄─2── access token ───────────────┘
phone ──3── access token ──────────────► your PC
                     PC ──4── "is this token real, and is it my owner?" ──► Supabase
phone ◄─5── local session cookie ───────┘
```

The password goes straight to Supabase; DevDeck never sees it. Your PC then
independently confirms with Supabase that the token is genuine **and** belongs
to the account that owns that machine, before granting a local session.

A compromised cloud account therefore yields a **URL**, not a dev server — the
machine still decides.

## Why Row Level Security matters

The anon key is public, so without RLS every row would be readable by anyone.
[`schema.sql`](schema.sql) enables it and adds policies scoping every operation
to `auth.uid() = user_id`. Isolation is enforced by Postgres, not by DevDeck
behaving correctly.

The insert policy uses `with check`, which stops a client writing a row that
claims to belong to someone else.

## Free tier

50,000 monthly active users, 500 MB database, 5 GB bandwidth. A DevDeck
install writes one row per machine and updates it when a tunnel starts, so the
database size is measured in kilobytes.

## Confirmation emails — read this before anyone else signs up

Supabase's built-in mail service sends **two emails per hour for the whole
project**. Not two per person: two in total. Supabase says plainly that it is
for development, not production.

With email confirmation switched on, that cap is a wall. The third person to
sign up in an hour gets a rate-limit error, no email arrives, and there is no
way for them to finish — the account exists but cannot sign in. It is also easy
to hit by accident while testing, because every retry of a signup or a password
reset spends one of the two.

### While it is only you

Turn confirmation off:

**Authentication → Sign In / Providers → Email → _Confirm email_ → off.**

Signup then completes immediately, sends nothing, and cannot be rate limited.
DevDeck notices the account is usable straight away and signs you in rather
than asking for the password a second time.

### Before other people use it

Configure custom SMTP — **Authentication → Emails → SMTP Settings** — and then
turn confirmation back on. Two workable options:

| | |
| --- | --- |
| **Resend** | 3,000/month free. Needs a domain you control: three DNS records (DKIM, SPF, and an MX for bounces). Best deliverability, because confirmation mail is signed. |
| **Brevo** | 300/day free. A single sender address is verified by email, with no DNS at all. Quicker, but unsigned mail frequently lands in spam — and a confirmation link in a spam folder means people simply never finish signing up. |

Whichever you use, add the confirmation page to
**Authentication → URL Configuration → Redirect URLs**:

```
https://king-upe.github.io/DevDeck/confirmed.html
```

Without it the link in the email redirects to `localhost:3000`, which looks
exactly like a failure even though the account was confirmed correctly.

## Optional

Cloud accounts are entirely optional. With no project configured, DevDeck works
exactly as it does offline: LAN pairing by QR, a local password, tunnels and
previews. Nothing is sent anywhere.
