<div align="center">
  <img src="src/assets/logo.png" alt="DevDeck Logo" width="128" />
  <h1>DevDeck</h1>
  <p>Your beautiful, centralized workspace and terminal manager.</p>
</div>

DevDeck is a sleek desktop application that allows you to manage all of your development workspaces in one place. Easily scan folders for projects, manage built-in terminal tabs, run custom commands, and organize your environment.

## 🚀 Download & Install

You can download the latest installer for Windows, macOS, or Linux from the [Releases Page](../../releases).

---

## ⚠️ Installation Guide (Bypassing Warnings)

Because this app is independently developed and currently unsigned by a paid global certificate authority, your operating system will likely display a security warning when you try to install or run it for the first time. **This is completely normal for open-source independent apps.**

Here is how to safely bypass the warnings and install DevDeck:

### 🪟 For Windows Users (SmartScreen Warning)
When you run the `.exe` or `.msi` setup file, Windows SmartScreen will display a blue box saying **"Windows protected your PC"**.

1. Click on the **"More info"** text link directly under the message.
2. A new button will appear at the bottom right that says **"Run anyway"**.
3. Click **"Run anyway"** to launch the installer!

*(Note: Depending on your antivirus settings, you may also need to click "Allow" if it prompts you about an unknown publisher).*

### 🍎 For macOS Users (Unverified Developer Warning)
When you try to open the `.dmg` or the installed DevDeck app, macOS will tell you that the app **"cannot be opened because the developer cannot be verified."**

1. Click **Cancel** on the warning popup.
2. Open your Mac's **System Settings**.
3. Go to **Privacy & Security** and scroll down to the "Security" section.
4. You will see a message saying DevDeck was blocked. Click the **"Open Anyway"** button next to it.
5. Provide your Mac password and click **Open**. 
*(You only have to do this once! DevDeck will open normally from then on).*

### 🐧 For Linux Users
If you downloaded the `.AppImage`:
1. Right-click the `.AppImage` file and select **Properties**.
2. Go to the **Permissions** tab.
3. Check the box that says **"Allow executing file as program"**.
4. Double-click the file to run it!

---

## Contributing
Clone the repository and run:
```bash
npm install
npm run tauri dev
```
