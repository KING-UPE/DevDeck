# DevDeck v1.1.0 — what to test, in order

Everything below is built and its tests pass, but **no part of it has been
through a real end-to-end run**: no genuine sign-in, no real phone, no
installer downloaded from the website. That is what this list is for.

Work down it in order. Each step unblocks the next, so a failure early on
explains failures later.

---

## 0. Unblock the release · 1 min · **only you can do this**

The release is prepared locally — version bumped, `main` moved forward, tag
`v1.1.0` created — but the push is refused:

```
refusing to allow an OAuth App to create or update workflow
`.github/workflows/android.yml` without `workflow` scope
```

The GitHub token on this machine can push code but not workflow files, and
this release adds two of them. Grant the scope:

1. Open **https://github.com/login/device**
2. Enter the one-time code shown in the chat (codes expire after ~15 minutes)
3. Make sure you are signed in as **KING-UPE**, not the work account
4. Approve the `workflow` scope

Nothing reaches GitHub until this is done. After it, one push publishes
everything.

> The active `gh` account on this machine was switched to `KING-UPE`, because
> only that account can write to this repo. Switch back whenever you like with
> `gh auth switch`.

---

## 1. Watch the release build · ~15 min, unattended

Pushing tag `v1.1.0` starts three workflows:

| Workflow | Produces |
| --- | --- |
| Publish Tauri Apps | Windows `.exe` + `.msi`, macOS `.dmg`, Linux `.AppImage` / `.deb` / `.rpm` |
| Build DevDeck Remote (Android) | `DevDeck-Remote-v1.1.0.apk` |
| Tests | 112 Rust tests, 12 worker tests, syntax checks, an Android cross-compile |

**Expect:** all green, and the release page carrying both the desktop
installers and the APK.

**The APK is the one to watch.** It has never been built anywhere. It failed
locally because Windows refuses the symlink Tauri uses to link the compiled
library into the Android project; a Linux runner has no such restriction, so
this is the first real attempt. If it fails, the workflow log is the only
place that will say why.

---

## 2. The website · 2 min

<https://king-upe.github.io/DevDeck/download.html>

Pages rebuilds from `main` within a couple of minutes of the push.

- The desktop card reads **The app · for your computer**
- Below a dividing rule, **Companion app · for your phone**
- The Android button changes from "Not in this release yet" to **Download
  .apk**
- Above that button, the notice: *Needs DevDeck on your computer*

**Test the downloads themselves** — click the `.exe` and the `.apk`. The page
reads the release from the GitHub API, so an unexpected asset name shows up as
a dead button here and nowhere else.

---

## 3. Install the desktop app · 3 min

Install from the website rather than from a local build. That is the path a
real user takes, and it is the one nobody has walked.

**Expect:** it opens, your workspaces are still there, your projects list as
before.

This release moves the app's storage out of browser localStorage and into
SQLite, and **the first launch migrates it**. If projects are missing, stop
there and tell me — a migration bug matters more than everything below it.

⚠️ Windows Firewall prompts the first time the gateway starts. Tick **Private
networks** and allow, or nothing on your network can reach it.

---

## 4. Sign in · 2 min

Your cloud account **exists and is confirmed** — the confirmation link worked,
it just redirected to a dead `localhost:3000` page, which looks like a failure
but wasn't. The password is what does not match.

1. Click the **account chip** at the top of the sidebar
2. **Forgot password?** → reset via email
3. Sign in

Do **not** use *Create account*. It will now correctly tell you the address is
already registered — that message was wrong until recently, when it would
claim a brand new account had been created.

**Expect:** the chip changes from "Sign in" to your name with a letter avatar.

---

## 5. Add the Supabase redirect URL · 1 min

So future confirmation emails don't land on a dead page.

Supabase dashboard → **Authentication → URL Configuration → Redirect URLs** →
add:

```
https://king-upe.github.io/DevDeck/confirmed.html
```

**Test it:** sign up with a throwaway address and click the link in the email.
You should land on an "Email confirmed" page, not connection-refused.

---

## 6. Put the PC online · 1 min

The account records *where* a machine is. With no tunnel there is no address,
so a phone lists your PC as **Offline**.

1. **⚙ gear** in the sidebar → **Link a device**
2. **Use anywhere** — up to 45s while Cloudflare assigns a hostname

**Expect:** the address becomes `https://….trycloudflare.com` and the label
reads "Reachable anywhere".

---

## 7. Use it from a phone browser · 5 min

This works without the APK, and it is the fastest way to find out whether the
whole chain holds.

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

## 8. Share a project · 3 min

1. Open a project → the **share icon in its header**
2. Set **Anyone with the link**
3. Copy the link and open it on something **not signed in** — another phone, or
   a private window

**Expect:** it opens with no sign-in, and shows only that one project.

Then set it back to **Only me** and reload the same link. It must now demand a
sign-in. If it still opens, that is a serious bug — tell me straight away.

---

## 9. The phone app · 5 min

Install the APK from the website. Android asks you to allow installs from your
browser the first time; that is normal for an app distributed outside the Play
Store.

1. Sign in with the **same account** as the desktop
2. Or: **⚙ → Link a device** on the desktop, and scan the QR from the app
3. Your PC appears under **Your computers**, marked **Online**
4. Tap it → projects load → **Start** one → **Preview**
5. **Copy link**, and open it in the phone's browser

If the PC shows **Offline**, step 6 was not done, or the tunnel dropped.

**This is a remote control, not DevDeck.** It cannot open an editor, scan for
projects or edit files, and it is useless without the desktop app running.
That is deliberate.

---

## What I expect to break first

In rough order of likelihood:

1. **The APK build** — never built anywhere, on any machine.
2. **The localStorage → SQLite migration** — tested against synthetic data,
   never against your real workspaces.
3. **The tunnel** — Cloudflare quick tunnels are free and unreliable by
   nature; a hostname can take 45s, or simply fail.
4. **Live reload on a project without HMR** — the most fragile path, because
   it rewrites HTML on the way through the proxy.

---

## Known issues

| | |
| --- | --- |
| `startWorkspaceWizard is not defined` | **Pre-existing**, not from this work — confirmed against the original v1.0.21. Fires after a scan finds new projects. Say the word and I'll fix it. |
| Unsigned APK | Installs fine, but Android shows a warning, and distributing it widely needs a real signing key. Add `ANDROID_KEYSTORE_BASE64`, `ANDROID_KEY_ALIAS`, `ANDROID_KEY_PASSWORD` and `ANDROID_KEYSTORE_PASSWORD` as repo secrets and CI signs it automatically. |
| Rendezvous Worker | Built and tested, never deployed. Cloud accounts replaced its purpose, so it is probably dead weight — I would delete it unless you want a fallback for machines with no tunnel. |
| No iPhone build | Needs a Mac and a paid Apple Developer account. The phone-browser path in step 7 works on iOS today. |
