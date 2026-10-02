# Sotto — Local, Offline Voice-Dictation for Windows

<p align="center">
  <img src="./icons/icon.png" width="128" height="128" alt="Sotto Logo" />
</p>

Sotto (*from "sotto voce" — in a quiet voice*) is a local, offline, hands-free voice-dictation utility for Windows. Built with **Rust, Tauri v2, and ONNX/llama.cpp**, it runs entirely offline on your device, respecting your privacy and system resources.

Hold (or toggle) a hotkey, speak, and Sotto will transcribe your voice using a local ASR model, clean it up with a local LLM, and paste/type it instantly into whatever application is currently focused.

The design philosophy of Sotto is **calm, quiet, precise, and unobtrusive** — a utility that lives at the edge of attention. State is communicated using **colors** so you can read the app's status peripherally without reading text.

<p align="center">
  <img src="./docs/images/screenshots/home-light.png" width="49%" alt="Sotto home screen, light theme" />
  <img src="./docs/images/screenshots/home-dark.png" width="49%" alt="Sotto home screen, dark theme" />
</p>
<p align="center">
  <img src="./docs/images/screenshots/insights.png" width="49%" alt="Sotto Insights dashboard" />
  <img src="./docs/images/screenshots/dictionary.png" width="49%" alt="Sotto custom Dictionary screen" />
</p>

---

## ✨ Features

- **Push-to-Talk & Toggle Modes:** Hold to speak and release to transcribe, or tap once to start and tap again to stop.
- **Offline ASR (Speech-to-Text):** **Parakeet v3 int8** (ONNX, English-only, default), **Whisper large-v3-turbo** (multilingual, Vulkan GPU-accelerated), or **Egyptian Arabic** (a Whisper-small model tuned for Egyptian Arabic mixed with English; known issue: on **Auto** language it can mistake English speech for Arabic, [#68](https://github.com/khairyKY/sotto/issues/68)). Pick your engine in Settings; it takes over on your next dictation, no restart. The speech model is unloaded after 5 idle minutes to free memory and reloads while you start speaking.
- **AI Polish Tier:** Uses a local **Qwen2.5 1.5B Instruct** model via a background `llama.cpp` sidecar to clean up speech, correct grammar, fix stutters, and add punctuation. If the model drops clauses or adds words you never said, Sotto keeps the rules-tier result instead.
- **Rule-Based Polish Tier:** Fast, instant, zero-cost rules cleanup for short phrases, bypassing the LLM round-trip.
- **Polish Word-Count Threshold:** Automatically routes shorter phrases through the rules tier and longer dictations through the AI tier.
- **Custom Dictionary & Snippets:** Define custom replacements (e.g., `"gee pee tee"` ➔ `GPT`, `"my email"` ➔ `dev@sotto.app`). An entry can have aliases ("my main email", "my primary email" ➔ one address) and its own on/off switch, and one master switch pauses them all.
- **Pronunciation Trainer:** Teach Sotto a name by saying it: type the word, press **Listen for it**, and say it. A miss can be added as a correction. Your trained words then guide AI polish and, on the Whisper engines, the speech model itself. The optional **Say it first** switch (off by default) plays the word in an offline Windows voice before you speak.
- **Fix Trained Words by Sound:** A word that merely *sounds like* one you trained (a new mishearing of "Claude", say) is pulled back to the right spelling. Ordinary English words, filenames and code are left alone. On by default; switch it off in Settings → Dictation.
- **Voice Formatting Commands:** Say "new line" or "new paragraph" for a real break, and "open quote" / "close quote" or "quote … unquote" for quotation marks (straight by default; curly in Settings → Dictation → Quote style).
- **Spoken Numbers as Digits:** "twenty three" ➔ `23`, "seven fifteen a.m." ➔ `7:15 a.m.`. Both this and the voice commands are on by default, with a switch each in Settings → Dictation.
- **Long Dictations:** Past about 10 seconds of audio, transcription starts while you are still talking, so the wait after you stop is only the last stretch.
- **Crash-Safe Takes:** Audio is journaled to disk while you speak and deleted once the take is delivered or dismissed. If Sotto or the PC dies mid-dictation, the take comes back on the next launch under **Last dictation wasn't delivered** on Home, with **Retry**.
- **Cancel & Retry:** Press Escape (or click ✕ on the pill) to abort a dictation. The take is kept in memory, so ↻ on the pill — or **Retry last dictation** in the tray — re-runs it without speaking again.
- **Insights Dashboard:** Words dictated, time saved, WPM, per-app breakdown, and a weekday-aligned streak calendar. Stats are local-only and can be turned off.
- **Dictation History:** Recent dictations, with one click to re-copy or re-polish any row. **±** shows word by word what polish changed (for lines from this session), with **Use what I said** to copy your original wording instead, and **⚑** flags a bad line into a local file, `bug-reports.jsonl`, for you to review. Nothing is sent anywhere. Opt in to **Keep history** in Settings → Data & privacy to have the list survive a restart.
- **Calm UI Overlay:** A transparent pill, click-through except over its own buttons and — while listening — the pill body itself, so a click stops and delivers the take (✕ still cancels). Pick one of nine screen positions in Settings → Appearance → Pill position. Turn on **Always show the pill** (Settings → Dictation, opt-in) to keep it tucked on screen when idle and start a take by clicking it.
- **Themes & Zoom:** Light / Dark / follow-system, plus `Ctrl +` / `Ctrl -` / `Ctrl 0` to scale the whole window.
- **Launch at Login** and **Minimized Launch:** Start automatically, hidden to the system tray.
- **Clipboard Safety Net:** Every delivered dictation is also left on the clipboard, in case focus moved.

### Off by default

These ship switched off. Turn them on when you want them. (**Keep history** and **Always show the pill**, above, are opt-in too.)

| Feature | What it does | Turn it on |
| :--- | :--- | :--- |
| **Transforms** (beta) | Select text in any app, press a shortcut (defaults: `Ctrl+Alt+1` Polish, `Ctrl+Alt+2` Prompt engineer; change or add your own; with Right Ctrl as your dictation key, use the left Ctrl), and the local model rewrites it in place. Your clipboard is put back afterwards. | Transforms page → **Enabled** |
| **Auto-send** | Presses Enter after a dictation lands, only in the apps you list. | Settings → Dictation → **Auto-send** → **+ Add app** |
| **Keep recordings** | Saves each take's audio, what Sotto heard and the polished text, so you can compare them later. Local only, capped at 500 MB by default (oldest go first). | Settings → Data & privacy → **Keep recordings** |
| **Voice correction** | Right after a dictation, say "correction: Claude, not clawed" and Sotto retypes that dictation with the fix; for a misheard name or term it also teaches the pair: as a hint for the speech model and polish, plus a Dictionary entry (removable) when what was heard isn't a real word. If the text has moved on, it leaves the document alone and puts the fix on the clipboard. The command itself is never typed. | `config.toml`: `voice_correction = true` |
| **Backtrack** | A take that is only "scratch that", "undo that" or "delete that" deletes your last dictation (not in terminals). | `config.toml`: `backtrack = true` |
| **Calibrate** | The Pronunciation page gets a Calibrate card: read sentences built from your trained words, and add any Sotto heard differently as a correction. | `config.toml`: `calibration = true` |
| **Scratchpad** (beta) | A private pad page: dictate while it is in front and the take lands there instead of in an app. Opens and closes with `Ctrl+Alt+Space` (the default). Keeps the last 200 lines, on this device. | `config.toml`: `scratchpad = true` |
| **Auto tone** | When a take starts in the AI tier, Sotto reads up to 300 characters before the caret in the focused field so polish can match its register. Read on your machine through UI Automation, never from password fields or terminals, never logged or stored. A per-app tone from Settings → Tone still wins. | `config.toml`: `auto_tone = true` |

The `config.toml` switches are plain top-level lines in `%APPDATA%\sotto\config.toml` (above any `[section]` heading); restart Sotto after editing. These features are new and were tested with unit tests and in the UI preview rather than in daily use, so expect rough edges, and please report them.

---

## 🎨 Visual Identity & State Signaling

Sotto uses the **Marshmallow** design language — a soft cream/lilac palette, Newsreader + Hanken Grotesk type, and a calm neumorphic surface treatment. The overlay is a small pill that reports state by **color and motion**, so you can read it peripherally without reading text:

<p align="center">
  <img src="./docs/images/screenshots/pill-states-light.gif" width="49%" alt="The Sotto overlay pill animating through listening, transcribing, polishing, and done — light theme" />
  <img src="./docs/images/screenshots/pill-states-dark.gif" width="49%" alt="The Sotto overlay pill animating through listening, transcribing, polishing, and done — dark theme" />
</p>
<p align="center"><sub>Live capture of the overlay pill — <b>listening → transcribing → polishing → done</b>, light and dark. The remaining states are described below.</sub></p>

| State | Color | What the pill shows |
| :--- | :--- | :--- |
| **Listening** | Lilac (`#8E74D0`) | Five bars dancing to your live mic level, plus a ✕ to cancel. |
| **Transcribing** | Amber (`#D4A06A`) | Bars give way to dots sweeping left to right. |
| **Polishing** | Gold (`#E8C78A`) | Drifting blobs with a shimmer and sparkles (AI tier only). |
| **Done** | Lilac (`#8E74D0`) | A checkmark draws itself, then the pill fades out. |
| **Cancelled** | Neutral | "Cancelled" + a ↻ to re-run the take. Auto-dismisses in 6s. |
| **Error** | Blush (`#F0BFCF`) | "Didn't catch that" + ↻. Muted, non-alarming. |
| **Model downloading** | Neutral | First-run only — the take is stashed, so ↻ works once the download lands. |

> [!NOTE]
> The design source is in the repo: **[`Sotto Marshmallow.dc.html`](./Sotto%20Marshmallow.dc.html)** (open it in any browser for the live spec — colors, shadows, type, motion), with a literal extraction at [`docs/marshmallow-spec-extracted.md`](./docs/marshmallow-spec-extracted.md).
> The earlier, superseded design lives at [`docs/images/design_handoff_sotto/Sotto.dc.html`](./docs/images/design_handoff_sotto/Sotto.dc.html) for reference only — it does **not** describe the current UI.

---

## 📦 Getting Started (For Users)

### 1. Install (~16 MB)
Grab the latest installer from the [**Releases page**](https://github.com/khairyKY/sotto/releases/latest) — pick `Sotto_x.y.z_x64-setup.exe` and run it.

> [!IMPORTANT]
> **Windows will warn you before it runs.** You'll see a blue **"Windows protected your PC"** screen. Click **More info → Run anyway**.
>
> This is expected, and it does **not** mean Sotto is malware. Windows shows that screen for any installer that isn't signed with a paid Authenticode certificate (~$100–200/year), which this project doesn't have. SmartScreen is reporting *"I don't recognise this publisher"*, not *"this is dangerous"*.
>
> Don't just take our word for it. You can check:
> - **Read the source.** All of it is in this repo, published for transparency. The installer is built from exactly this code.
> - **Scan it.** Upload the `.exe` to [VirusTotal](https://www.virustotal.com/) before running it.
> - **Watch the network.** Sotto only ever talks to `github.com`: the [release feed](https://github.com/khairyKY/sotto/releases/latest/download/latest.json) (update check) and, first run only, the [`assets-v1`](https://github.com/khairyKY/sotto/releases/tag/assets-v1) release (Qwen, the llama.cpp runtime, and Parakeet) plus [`assets-v2`](https://github.com/khairyKY/sotto/releases/tag/assets-v2) if you pick the Whisper or Egyptian Arabic engine instead. There is no telemetry or analytics of any kind — grep the source. It also talks to `127.0.0.1:8177`, which is the AI-polish model running on your own machine; that's loopback and never leaves your PC. Once the models are downloaded, pull your network cable and it still works.
> - **Check the signature.** Every release *is* cryptographically signed with [minisign](https://jedisct1.github.io/minisign/) for the auto-updater; that's what stops a tampered update from installing. It's just not the certificate flavour SmartScreen recognises.
>
> If you'd rather trust nothing, build it yourself — see [Development](#️-development--building-from-source).

### 2. First launch — one-time model download (~2.1 GB)
The installer is intentionally tiny because the models aren't in it. On first launch Sotto opens Settings and downloads them once into `%APPDATA%\sotto`, with a progress banner. If the connection drops, restart Sotto and the download picks up where it stopped. After that, dictation works fully offline, and app updates never re-download any of it.

| Download | On disk | What it is |
| :--- | :--- | :--- |
| 1066 MB | 1066 MB | `qwen2.5-1.5b-instruct-q4_k_m.gguf` — the local LLM behind the **AI polish** tier |
| 446 MB | 639 MB | **Parakeet v3 int8** — the speech-recognition model |
| 647 MB | 1141 MB | `llama.cpp` runtime (the bulk is CUDA: `ggml-cuda` + `cuBLAS`) |
| 13 MB | 13 MB | `onnxruntime.dll` — runs the speech model |
| **~2.1 GB** | **~2.8 GB** | |

> [!TIP]
> Only the AI polish tier needs the LLM and its CUDA runtime (~1.7 GB of the total). The **Rules** tier is instant and local-only.

#### Putting the models on another drive

Short on space on your system drive? Set `assets_dir` in `%APPDATA%\sotto\config.toml` to put the big files anywhere you like, then restart Sotto:

```toml
assets_dir = 'D:\sotto'
```

Your settings and stats stay in `%APPDATA%\sotto` (a few KB) — only the ~2.8 GB moves. Settings then shows both locations, under **Data folder** and **Models folder**.

Already downloaded the models? Move `models\`, `runtime\`, and `onnxruntime.dll` into the new folder rather than re-downloading them.

### 3. Use it
Sotto launches minimized to the **system tray** (check the `^` overflow menu next to the clock). **Left-click** the tray icon to open the app; **right-click** for a menu with **Settings**, **Insights**, **History**, **Dictionary**, **Retry last dictation**, **Pause**, and **Quit**.

Open any app, hold **Right Ctrl** (the default — rebindable in Settings), speak, release. Sotto transcribes locally and pastes into the focused window. Press **Escape** to cancel; the take is kept so you can retry it.

### 4. Updates — one click, ~16 MB
Sotto checks GitHub on launch. When a newer version is out, open the tray icon → **Settings**, where an **Install & restart** banner appears (dismissable with ✕). Click it — the small installer downloads, verifies its minisign signature, and relaunches. Your models and settings are untouched.

### 5. Uninstalling
Uninstall Sotto from **Settings → Apps** or via `Sotto_*_x64-setup.exe /uninstall`. The uninstaller removes the app, disables launch-at-login, and asks whether to also delete the ~2.8 GB of downloaded models and your settings (default: **keep**, so a reinstall is instant).

---

## 🩺 Troubleshooting

**Something's wrong — where's the log?**

`%APPDATA%\sotto\logs\sotto.log` (paste that into the Explorer address bar). Settings → **Data folder** → *Open folder* gets you there too.

- `sotto.log` — the current run. `sotto.log.1` — the previous one. Plain text; open it in Notepad.
- It records the app version, where it's reading models from, and what failed. **When reporting a bug, attach it** — it's the difference between a fix and a guessing game.
- It never contains your dictated text or your dictionary/snippet entries. Counts and timings only.
- Need more detail? Set `SOTTO_LOG=debug` and relaunch. **Debug logs DO include what you dictated** (raw vs polished), so read one before attaching it anywhere public. `llama-server.log` in the same folder covers the AI polish sidecar.

| Symptom | Likely cause |
| :--- | :--- |
| Tray icon there, hotkey does nothing | Another app grabbed the same key — rebind it in Settings. |
| "Model downloading…" on the pill | First-run download hasn't finished. The take is stashed; press ↻ when it lands. |
| Transcription is slow | Parakeet (the default engine) is CPU-only ONNX, but still roughly real-time. Switch to the Whisper engine in Settings for Vulkan GPU acceleration (auto-picks your best GPU; falls back to CPU if none is found). |
| Text lands in the wrong window | Sotto pastes into whatever was focused when you *started* talking. It's also on your clipboard. |

---

## 🛠️ Development & Building from Source

Sotto is structured as a Tauri v2 application:
- **Rust Core:** Handles hotkey listening (`rdev`), audio recording (`cpal`), ASR (`transcribe-rs` over ONNX Runtime), local LLM integration (`llama.cpp` sidecar), and text injection (`windows`).
- **Frontend:** Transparent overlay pill and Settings windows built using HTML, CSS, and vanilla JS (`ui/`).

### Prerequisites
1. **Rust Toolchain:** Install Rust with MSVC support:
   ```powershell
   rustup override set stable-x86_64-pc-windows-msvc
   ```
2. **C++ Toolchain:** Requires MSVC Build Tools and the Windows SDK (installed by running `./scripts/setup-msvc.ps1` as Administrator).
3. **CMake + libclang.** The Whisper engine compiles `whisper.cpp` from source, and its Rust bindings are generated with `bindgen`. Without both of these the build fails on `Unable to find libclang`, which does not obviously mean "you're missing CMake too".
   ```powershell
   # CMake — any 3.x+; portable zip is fine, no installer needed
   $env:PATH = 'C:\path\to\cmake\bin;' + $env:PATH

   # libclang.dll. Full LLVM works, but it's ~2 GB for one DLL.
   # The `libclang` PyPI wheel ships the same DLL in ~80 MB:
   #   python -m pip install libclang
   $env:LIBCLANG_PATH = 'C:\path\to\python\Lib\site-packages\clang\native'
   ```
   Both are build-time only — neither is shipped or required by end users.
4. **Node.js:** For dev dependencies and bundling:
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
  *(Preview at `http://localhost:5173/index.html`, `/menu.html`, and `/preview.html`)*

- **Headless ASR on a WAV file:**
  ```powershell
  cargo run -- --transcribe path\to\audio.wav
  ```

- **Run the AI polish tier on a string (spawns the local LLM sidecar):**
  ```powershell
  cargo run -- --polish "<raw text>"
  ```

- **QA the AI polish tier on the fixed set of invented dictations (`tests/polish-qa/`):**
  ```powershell
  cargo run -- --polish-qa
  ```

### Cutting a release

The updater workflow (bump version → sign → publish to GitHub Releases so every running app picks it up as a 16 MB update) is documented step-by-step in [`docs/updating.md`](./docs/updating.md).

---

## 🔒 Security & Privacy

- **100% Local:** All voice recordings are processed on your local CPU/GPU. No speech, transcripts, or keystrokes ever leave your device.
- **Nothing Dictated Is Stored Unless You Opt In:** Keeping History across restarts, keeping recordings and the Scratchpad are all off by default, and the log holds counts only (unless you set `SOTTO_LOG=debug`). Two things do write your words locally: a take in flight is journaled to `%APPDATA%\sotto\pending\` until it is delivered or dismissed (see **Crash-Safe Takes**), and a line you flag with ⚑ is saved to `bug-reports.jsonl`. Neither leaves your machine.
- **Single-Instance Protection:** Sotto uses a single-instance guard to ensure only one session can hook the keyboard at any time. Launching it a second time just brings the running window forward.
- **Keystroke Injection Safety:** Global key-event interception is temporarily suspended during text injection to prevent cyclic key-repeats or focus issues.

---

## 🙏 Models & Components

Sotto's own code is covered by the license below. The models and runtime it downloads on first launch are other people's work, under their own licenses:

| What | By | License |
| :--- | :--- | :--- |
| [Parakeet TDT 0.6b v3](https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3) (default speech-to-text, int8 ONNX export) | NVIDIA | CC-BY-4.0 |
| [Whisper large-v3-turbo](https://huggingface.co/openai/whisper-large-v3-turbo) (GGML from [whisper.cpp](https://huggingface.co/ggerganov/whisper.cpp)) | OpenAI | MIT |
| [Code-switched Egyptian Arabic Whisper small](https://huggingface.co/IbrahimAmin/code-switched-egyptian-arabic-whisper-small) (converted to GGML) | Ibrahim Amin; a fine-tune of OpenAI's whisper-small | Apache-2.0 |
| [Qwen2.5-1.5B-Instruct](https://huggingface.co/Qwen/Qwen2.5-1.5B-Instruct) (AI polish, GGUF) | Alibaba Cloud | Apache-2.0 |
| [llama.cpp](https://github.com/ggml-org/llama.cpp) (runs the polish model; ships with NVIDIA CUDA runtime libraries) | the llama.cpp authors | MIT; CUDA libraries under NVIDIA's license |
| [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (runs the Whisper engines) | the whisper.cpp authors | MIT |
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) (runs Parakeet) | Microsoft | MIT |
| [Harper](https://github.com/Automattic/harper) (grammar checks in the rules tier) | Automattic | Apache-2.0 |

---

## ⚖️ License
All rights reserved. The source is public for transparency, not for reuse. See [LICENSE](./LICENSE).
