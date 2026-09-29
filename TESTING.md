# What to test when you're back at the desk

Everything below is built and passing tests, but **none of it has been through a
real end-to-end run** — no genuine sign-up, no real phone. That is what this
list is for. Work down it in order; each step unblocks the next.

---

## 1. Sign in to the cloud account  ·  2 min

Your account `upendrauniversity@gmail.com` **exists and is confirmed** — the
confirmation link worked, it just redirected to a dead `localhost:3000` page,
which looks like a failure but isn't. The password is what's not matching.

1. Start DevDeck: `npm run tauri dev`
2. Click the **account chip** at the top of the sidebar
3. **Forgot password?** → reset via email
4. Sign in

> Do **not** use *Create account* — it will now correctly tell you the address
> is already registered. That message was wrong until today; it used to claim a
> brand new account had been created.

**Expect:** the sidebar chip changes from "Sign in" to your name with a letter
avatar.

---

## 2. Add the Supabase redirect URL  ·  1 min

So future confirmation emails don't land on a dead page.

Supabase dashboard → **Authentication → URL Configuration → Redirect URLs** →
add:

```
https://king-upe.github.io/DevDeck/confirmed.html
```

**Test it:** sign up with a throwaway address and click the link in the email.
You should land on a "Email confirmed" page, not connection-refused.

---

## 3. Put the PC online  ·  1 min

The account only records *where* a machine is. With no tunnel there is no
address, so the phone will list your PC as **Offline**.

1. **⚙ gear** in the sidebar → **Link a device**
2. **Use anywhere** — takes up to 45s while Cloudflare assigns a hostname

**Expect:** the address changes to `https://….trycloudflare.com` and the label
reads "Reachable anywhere".

⚠️ Windows Firewall prompts the first time the gateway starts. Tick **Private
networks** and allow, or nothing on your network can reach it.

---

## 4. Use it from a phone browser  ·  5 min

This works today, without the APK.

1. **⚙ → Link a device** → scan the QR with your phone camera
2. Tap a project to expand it
3. **Start** a script → wait a moment → **Preview** appears
4. Tap the **☰** icon on a running script for live output
5. **Restart** and **Stop** from the phone

**Worth checking specifically:**

- Does a project with no HMR (a static site, or PHP) reload on the phone when
  you edit a file? That is the live-reload path.
- Does `stderr` show in red when a build fails?

---

## 5. Share a project  ·  2 min

1. Open a project → **share icon** in its header
2. Set **Anyone with the link**
3. Copy the link, open it on a device that is **not** signed in

**Expect:** it opens with no login. Set it back to **Only me** and the same link
should then demand a sign-in.

---

## 6. Build the APK  ·  5 min, needs admin once

Blocked on one Windows setting: Tauri links the compiled library into the
Android project with a symlink, and Windows won't let a non-administrator make
one.

Right-click PowerShell → **Run as administrator**:

```powershell
cd D:\ME\DevDeck\mobile
.\build-apk.ps1
```

The script turns Developer Mode on, checks the toolchain, and builds. After the
first run a normal terminal is enough.

The Rust already compiles clean for `aarch64-linux-android`, so only the linking
step is untested.

---

## 7. The phone app  ·  5 min

Install the APK the script prints, then:

1. Sign in with the **same account** as the desktop
2. Your PC should appear under **Your computers**, marked Online
3. Tap it → projects load → **Start** one → **Preview**

If the PC shows **Offline**, step 3 is not done — its remote access is off.

---

## Known issues

| | |
| --- | --- |
| `startWorkspaceWizard is not defined` | **Pre-existing**, not from this work — confirmed against the original v1.0.21. Fires after a scan finds new projects. Say the word and I'll fix it. |
| Debug-signed APK | Installs fine for you. Distributing from the website needs a real signing key. |
| Rendezvous Worker | Built and tested, never deployed. Cloud accounts largely replace it, so it may just be dead weight. |

## Not yet done

- **Nothing is pushed.** ~25 commits sit on `feat/mobile-gateway` locally.
- No genuine sign-up → phone sign-in → start-a-project round trip has happened.
  Every layer is unit-tested and the live Supabase paths (auth, RLS, schema) are
  verified, but the whole loop has never run once.
