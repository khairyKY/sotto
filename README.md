# Sotto — Local, Offline Voice-Dictation for Windows

<p align="center">
  <img src="./icons/icon.png" width="128" height="128" alt="Sotto Logo" />
</p>

Sotto (*from "sotto voce" — in a quiet voice*) is a local, offline, hands-free voice-dictation utility for Windows. Built with **Rust, Tauri v2, and ONNX/llama.cpp**, it runs entirely offline on your device, respecting your privacy and system resources.

Hold (or toggle) a hotkey, speak, and Sotto will transcribe your voice using a local ASR model, clean it up with a local LLM, and paste/type it instantly into whatever application is currently focused.

The design philosophy of Sotto is **calm, quiet, precise, and unobtrusive** — a utility that lives at the edge of attention. State is communicated using **colors** so you can read the app's status peripherally without reading text.

---

## ✨ Features

- **Push-to-Talk & Toggle Modes:** Hold to speak and release to transcribe, or tap once to start and tap again to stop.
- **Offline ASR (Speech-to-Text):** Powered by NVIDIA's **Parakeet v3 int8** model running locally via ONNX Runtime. Extremely fast and accurate.
- **AI Polish Tier:** Uses a local **Qwen2.5 1.5B Instruct** model via a background `llama.cpp` sidecar to clean up speech, correct grammar, fix stutters, and add punctuation.
- **Rule-Based Polish Tier:** Fast, instant, zero-cost rules cleanup for short phrases, bypassing the LLM round-trip.
- **Polish Word-Count Threshold:** Automatically routes shorter phrases through the rules tier and longer dictations through the AI tier.
- **Custom Dictionary & Snippet Replacements:** Define custom abbreviations or replacements (e.g., `"gee pee tee"` ➔ `GPT`, `"my email"` ➔ `dev@sotto.app`).
- **Dictation History:** Keeps a local in-memory history of recent dictations in the settings window. Click any history row to re-copy it.
- **Calm UI Overlay:** A transparent click-through overlay showing a live mic waveform and state colors.
- **Launch at Login:** Option to start automatically on system boot.
- **Minimized Launch:** Start hidden directly to the system tray so it doesn't interrupt your flow.

---

## 🎨 Visual Identity & State Signaling

Sotto communicates its active status using a custom visual identity. The UI uses the **"Quiet"** overlay direction — a clean transparent pill displaying the S-mark logo and a live waveform, with state indicated by color:

| State | Color | Action / Meaning |
| :--- | :--- | :--- |
| 🟢 **Listening** | Cyan (`#4FCFDB`) | S-mark + live waveform active. Ready for your speech. |
| 🟡 **Transcribing** | Amber (`#E3A857`) | Waveform collapses to a dim track; traveling dot sweeps across. |
| 🟠 **Polishing** | Gold (`#F0C982`) | Traveling dot changes to gold; small twinkles fade above/below (AI cleanup). |
| 🔵 **Done** | Cyan (`#4FCFDB`) | Success checkmark draws, stays for a moment, then the pill fades out. |
| 🔴 **Error** | Rose (`#D0959C`) | Display "!" glyph and "Didn't catch that". Muted, non-alarming. |

> [!NOTE]  
> The design source prototype is included in the repository at [docs/images/design_handoff_sotto/Sotto.dc.html](file:///D:/Coding/sotto-opencode/docs/images/design_handoff_sotto/Sotto.dc.html). You can open it in any browser to inspect the visual style, hover interactions, color specs, and animations.

---

## 📦 Getting Started (For Users)

### 1. Install (~4 MB)
Grab the latest signed installer from the [**Releases page**](https://github.com/khairyKY/sotto/releases/latest) — pick `Sotto_x.y.z_x64-setup.exe` and run it.

### 2. First launch — one-time model download (~1.8 GB)
The installer itself is intentionally tiny. On first launch Sotto opens Settings and downloads the voice models + local-LLM runtime once into `%APPDATA%\sotto`; a progress banner keeps you informed. After that, dictation works fully offline and updates never re-download this again.

### 3. Use it
Sotto launches minimized to the **system tray** (check the `^` overflow menu next to the clock). Right-click the tray icon (charcoal S-mark) for **Settings**, **Polish mode**, **Pause**, or **Quit**.

Open any app, hold **Right Ctrl**, speak, release — Sotto transcribes locally and pastes the text into the focused window.

### 4. Updates — one click, ~4 MB
Sotto checks GitHub on launch. When a newer version is out, a **native Windows toast** appears and the Settings window shows an **Install & restart** banner. Click it — the small installer downloads, verifies its signature, and relaunches. Your models and settings are untouched.

### 5. Uninstalling
Uninstall Sotto from **Settings → Apps** or via `Sotto_*_x64-setup.exe /uninstall`. The uninstaller removes the app, disables launch-at-login, and asks whether to also delete the ~1.8 GB of downloaded models and your settings (default: keep — so a reinstall is instant).

---

## 🛠️ Development & Building from Source

Sotto is structured as a Tauri v2 application:
- **Rust Core:** Handles hotkey listening (`rdev`), audio recording (`cpal`), ASR model compilation (`transcribe-rs`), local LLM integration, and text injection (`windows-sys`).
- **Frontend:** Transparent overlay pill and Settings windows built using HTML, CSS, and vanilla JS (`ui/`).

### Prerequisites
1. **Rust Toolchain:** Install Rust with MSVC support:
   ```powershell
   rustup override set stable-x86_64-pc-windows-msvc
   ```
2. **C++ Toolchain:** Requires MSVC Build Tools and the Windows SDK (installed by running `./scripts/setup-msvc.ps1` as Administrator).
3. **Node.js:** For dev dependencies and bundling:
   ```powershell
   npm install
   ```

### Dev Commands
- **Run Sotto in Dev Mode:**
  ```powershell
  npx tauri dev
  ```
- **Build the Release Installers:**
  ```powershell
  npx tauri build
  ```
- **Preview the UI Frontend Standalone (browser, no build):**
  ```powershell
  node scripts/serve-ui.mjs
  ```
  *(Preview at `http://localhost:5173/settings.html` and `http://localhost:5173/preview.html`)*

- **Headless ASR on a WAV file:**
  ```powershell
  cargo run -- --transcribe path\to\audio.wav
  ```

- **Run the AI polish tier on a string (spawns the local LLM sidecar):**
  ```powershell
  cargo run -- --polish "<raw text>"
  ```

### Cutting a release

The updater workflow (bump version → sign → publish to GitHub Releases so every running app picks it up as a 4 MB update) is documented step-by-step in [`docs/updating.md`](./docs/updating.md).

---

## 🔒 Security & Privacy

- **100% Local:** All voice recordings are processed on your local CPU/GPU. No speech, transcripts, or keystrokes ever leave your device.
- **Single-Instance Protection:** Sotto uses a single-instance guard to ensure only one session can hook the keyboard at any time.
- **Keystroke Injection Safety:** Global key-event interception is temporarily suspended during text injection to prevent cyclic key-repeats or focus issues.

---

## ⚖️ License
This project is proprietary and confidential. All rights reserved.
