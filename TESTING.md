# DevDeck v1.1.0 — what to test, in order

v1.1.0 is published. The website, the release and all seven downloads are live
and verified. **What has never happened is a real end-to-end run** — no genuine
sign-in, no real phone, nobody installing from the website. That is what this
list is for.

Work down it in order. Each step unblocks the next, so a failure early on
explains failures later.

---

## Already done and verified

| | |
| --- | --- |
| Website | Live, serving the new download page |
| Release `v1.1.0` | Published, and what `releases/latest` returns |
| Windows | `.exe` 5 MB, `.msi` 7 MB — both return HTTP 200 |
| macOS | `.dmg` 7 MB, **Apple Silicon only** — returns 200 |
| Linux | `.AppImage` 83 MB, `.deb` 9 MB, `.rpm` 9 MB |
| Android | `DevDeck-Remote-v1.1.0.apk` 43 MB — returns 200 |
| Download page | All seven buttons resolve to real v1.1.0 URLs |
| Rust tests | 112 passing on the tagged commit |

Two things broke on the way and were fixed:

- **The APK was 441 MB.** Four architectures of unstripped debug library, two of
  them emulator-only. Now 43 MB.
- **The release upload was refused** despite the job being granted
  `contents: write`. Unexplained — see Known issues.

---

## 1. Install the desktop app · 3 min

<https://king-upe.github.io/DevDeck/download.html>

Install from the website rather than a local build. That is the path a real user
takes, and nobody has walked it.

**Expect:** it opens, your workspaces are still there, your projects list as
before.

This release moves app data out of browser storage and into SQLite, and **the
first launch migrates it**. If projects are missing, stop there and tell me — a
migration bug matters more than everything below it.

⚠️ Windows Firewall prompts the first time the gateway starts. Tick **Private
networks** and allow, or nothing on your network can reach it.

⚠️ SmartScreen will warn, because the installer is unsigned. **More info →
Run anyway.**

---

## 2. Sign in · 2 min

**First, turn email confirmation off**, or nothing here will work. Supabase's
built-in mailer allows two emails per hour for the entire project, and that
quota is spent — which is why both signing up and resetting the password
return "too many attempts".

**Authentication → Sign In / Providers → Email → _Confirm email_ → off.**

No emails are sent after that, so there is nothing left to rate limit. Then
either:

- **Create a fresh account** in DevDeck. It completes immediately and signs you
  straight in.
- **Or reuse the existing account**, whose password is what does not match. Set
  it directly in the Supabase **SQL Editor**, which needs no email:

  ```sql
  update auth.users
  set encrypted_password = crypt('a-new-password', gen_salt('bf'))
  where email = 'the-account-address';
  ```

**Expect:** the chip changes from "Sign in" to your name with a letter avatar.

The account you already had **is confirmed** — the original confirmation link
worked, it just redirected to a dead `localhost:3000` page, which looks like a
failure but wasn't.

---

## 3. Add the Supabase redirect URL · 1 min

Only matters once confirmation emails are switched back on, but it costs a
minute now and is easy to forget later. It stops those emails landing on a dead
page.

Supabase dashboard → **Authentication → URL Configuration → Redirect URLs** →
add:

```
https://king-upe.github.io/DevDeck/confirmed.html
```

**Test it:** sign up with a throwaway address and click the link in the email.
You should land on an "Email confirmed" page, not connection-refused.

---

## 4. Put the PC online · 1 min

The account records *where* a machine is. With no tunnel there is no address, so
a phone lists your PC as **Offline**.

1. **⚙ gear** in the sidebar → **Link a device**
2. **Use anywhere** — up to 45s while Cloudflare assigns a hostname

**Expect:** the address becomes `https://….trycloudflare.com` and the label
reads "Reachable anywhere".

---

## 5. Use it from a phone browser · 5 min

This needs no APK, and it is the fastest way to find out whether the whole chain
holds.

1. **⚙ → Link a device** → scan the QR code with your phone camera
2. Tap a project to expand its scripts
3. **Start** a script → wait → **Preview** appears → open it
4. Tap the **☰** icon on a running script for live output
5. **Restart**, then **Stop**, from the phone

**Worth checking specifically:**

- A project with no HMR — a static site, or PHP — should still reload on the
  phone when you edit a file. That is the injected live-reload path, separate
  from anything Vite does on its own.
- A failing build should show its `stderr` in red.
- Lock the phone, wait a minute, unlock. The log should catch up rather than
  hang. It is a long poll, and that is the case that breaks it.

---

## 6. Share a project · 3 min

1. Open a project → the **share icon in its header**
2. Set **Anyone with the link**
3. Copy the link and open it on something **not signed in** — another phone, or
   a private window

**Expect:** it opens with no sign-in, and shows only that one project.

Then set it back to **Only me** and reload the same link. It must now demand a
sign-in. If it still opens, that is a serious bug — tell me straight away.

---

## 7. The phone app · 5 min

Install `DevDeck-Remote-v1.1.0.apk` from the download page. Android asks you to
allow installs from your browser the first time, and warns that the app is
unsigned; both are normal outside the Play Store.

1. Sign in with the **same account** as the desktop
2. Or: **⚙ → Link a device** on the desktop, and scan the QR from the app
3. Your PC appears under **Your computers**, marked **Online**
4. Tap it → projects load → **Start** one → **Preview**
5. **Copy link**, and open it in the phone's browser

If the PC shows **Offline**, step 4 was not done, or the tunnel dropped.

**This is a remote control, not DevDeck.** It cannot open an editor, scan for
projects or edit files, and it is useless without the desktop app running. That
is deliberate.

---

## What I expect to break first

Now that the APK exists, in rough order of likelihood:

1. **The localStorage → SQLite migration** — tested against synthetic data,
   never against your real workspaces.
2. **The tunnel** — Cloudflare quick tunnels are free and unreliable by nature;
   a hostname can take 45s, or simply fail.
3. **Live reload on a project without HMR** — the most fragile path, because it
   rewrites HTML on the way through the proxy.
4. **The APK on a real phone** — it builds and it is the right size, but it has
   never been installed on anything.

---

## Known issues

| | |
| --- | --- |
| Release uploads refused in CI | `POST /releases` returns *Resource not accessible by integration* even though the job log shows `Contents: write` granted. No rulesets, no tag protection, not a fork, and the same workflow published v1.0.21. **Unexplained.** Worked around: the installers now also upload as build artifacts, which need no release permission, and v1.1.0's assets were attached by hand. |
| Two emails per hour, project-wide | Supabase's built-in mailer is a development service, and the cap is for the whole project rather than per person. With confirmation on, the third stranger to sign up in an hour cannot finish. Confirmation is off for now; custom SMTP is required before anyone else uses this. See [cloud/README.md](cloud/README.md). |
| Unsigned APK | Installs fine, but Android warns, and each CI build is signed with a throwaway debug key, so an upgrade needs an uninstall first. Add `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` and `ANDROID_KEYSTORE_PASSWORD` as repo secrets and CI signs it properly. |
| macOS is Apple Silicon only | The workflow builds `macos-latest` with no explicit target. The site no longer claims otherwise. Making it universal is a workflow change — two Rust targets and `--target universal-apple-darwin` — at roughly double the macOS build time. |
| `startWorkspaceWizard is not defined` | **Pre-existing**, not from this work — confirmed against the original v1.0.21. Fires after a scan finds new projects. Say the word and I'll fix it. |
| Rendezvous Worker | Built and tested, never deployed. Cloud accounts replaced its purpose, so it is probably dead weight — I would delete it unless you want a fallback for machines with no tunnel. |
| No iPhone build | Needs a Mac and a paid Apple Developer account. The phone-browser path in step 5 works on iOS today. |
| Gmail address in history | Commit `3c45ffb` carries your Supabase login address in an earlier draft of this file. It is public now. Removing it would mean rewriting published history, which I would not recommend. |
