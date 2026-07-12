# Sotto — current state & path to done (handoff)

> Snapshot for planning the finish. Sotto is a **local, offline, hands-free
> voice-dictation app for Windows**: hold (or toggle) a hotkey, speak, and your
> speech is transcribed by a local model and typed into whatever app is focused,
> optionally cleaned up by a local LLM.

## TL;DR

- **A complete, working version already exists** on the `master` branch (the
  egui build). It does everything below, including **toggle mode**. It builds
  and runs today on the current (gnu) toolchain.
- The `tauri-migration` branch is an **optional visual-fidelity upgrade** —
  rebuilding the two windows in HTML/CSS so they match the Claude Design mockups
  exactly. Frontend is **done**; the Rust is rewritten as a Tauri app but is an
  **uncompiled draft**. It is blocked on **one thing: installing the MSVC C++
  toolchain**, which Tauri/WebView2 requires.
- So there are two clean finish lines. Pick one:
  - **Path A — ship now:** use the egui app on `master`. ~0 remaining work.
  - **Path B — finish Tauri:** install MSVC → compile → fix the draft → verify →
    package. Higher visual fidelity. Steps below.

## Feature checklist (all implemented; live on `master`, wired in the Tauri draft)

| Feature | Status | Where |
|---|---|---|
| Push-to-talk (hold) | ✅ | `hotkey.rs` (`ActivationMode::Hold`) |
| **Toggle mode** (press once on/off) | ✅ already done | `hotkey.rs` (`ActivationMode::Toggle`), settings control |
| Offline ASR (Parakeet v3 int8) | ✅ | `asr.rs`, model in `D:\sotto\models` |
| AI polish (local Qwen2.5 via llama.cpp sidecar) | ✅ | `llm.rs`, `polish.rs` |
| Rules-only + Off polish tiers | ✅ | `polish.rs` |
| Polish word-count threshold | ✅ | `polish.rs`, settings slider |
| Dictionary / snippet replacements | ✅ | `polish.rs::apply_dictionary`, settings |
| History (recent dictations, click to re-copy) | ✅ | `history.rs`, settings |
| Pause dictation | ✅ | tray, `Controls.paused` |
| Launch at login | ✅ | `startup.rs` (registry Run key) |
| Overlay pill (5 animated states) | ✅ | egui `overlay.rs` / Tauri `ui/overlay.js` |
| Tray icon (S-mark) + menu | ✅ | `tray.rs` |
| Settings window (7 sections, light/dark) | ✅ | egui `settings.rs` / Tauri `ui/settings.*` |
| Single-instance guard, text injection (unicode/paste) | ✅ | `single_instance.rs`, `inject.rs` |

**Nothing on the feature list is missing.** Toggle mode in particular is already
in the config, the hotkey listener, and both settings UIs.

## Branch map

- `master` @ `6f7437e` — **working egui app**, gnu toolchain. Run it:
  ```
  git checkout master
  rustup override set stable-x86_64-pc-windows-gnu
  # mingw dlltool on PATH (see RUNNING.md), then:
  cargo run
  ```
- `tauri-migration` (HEAD) — the Tauri rewrite. Frontend done, Rust draft
  uncompiled, blocked on MSVC.

## The one blocker: MSVC install

Tauri/WebView2 must build on the `x86_64-pc-windows-msvc` target. The machine has
the VS Build Tools **shell** but not the C++ toolset. Installing it is gated by
disk space (C: is tight) so the script redirects the cache to D:. Three earlier
attempts failed on *my* scripting bugs, all now fixed in `scripts/setup-msvc.ps1`:
1. `--wait` is bootstrapper-only → removed (used `--passive` + poll).
2. Em-dash in a string broke PS 5.1 parsing → script is pure ASCII now.
3. `--installPath` with a space got split by `-ArgumentList` array → now passed
   as one pre-quoted argument string.

**To install (admin PowerShell):**
```powershell
powershell -ExecutionPolicy Bypass -File "D:\Coding\sotto\scripts\setup-msvc.ps1"
```
Then: `rustup override set stable-x86_64-pc-windows-msvc` in the repo.
Cleanup protocol + build-env details: `docs/msvc-setup.md`. Cleanup:
`scripts/sotto-cleanup.ps1`.

## Path B — remaining work to finish Tauri (for planning)

1. **Install MSVC** (above) — the only external blocker.
2. **Compile** in a VS dev environment (import `vcvars64.bat`, then `cargo build`).
   This is the first compile of the Tauri Rust draft.
3. **Fix compile errors** in `src/main.rs` + `src/tray.rs`. The draft is written
   against Tauri v2 from knowledge, not verified. Likely touch-ups: exact
   `tauri::image::Image` ctor, `tauri::menu` builder method names,
   `TrayIconBuilder`, command/`State` signatures, `WebviewWindow` methods,
   `Emitter`/`Manager` imports. (High-confidence bugs were pre-fixed in `04a75e0`.)
4. **Install the Tauri CLI** for run/package: `cargo install tauri-cli --version "^2"`
   (or `npm i -g @tauri-apps/cli`). Run with `cargo tauri dev`.
5. **Verify end-to-end** (see `verify` below): overlay states, tray menu,
   settings commands, dictionary, history, launch-at-login.
6. **Fix runtime issues** likely on Windows: overlay window transparency +
   click-through (`set_ignore_cursor_events`) + bottom-center positioning; the
   show/hide timing between Rust (`overlay-state` events) and the JS auto-hide;
   theme following the OS.
7. **Package**: `cargo tauri build` → MSI/NSIS installer, using `icons/`.

### How the Tauri app is wired (so the plan has the map)
- `src/main.rs` — Tauri app: managed `AppState { Controls, Mutex<Config> }`;
  commands `get_settings` / `set_hotkey|activation|polish|threshold|dictionary|
  launch_login|start_hidden` / `copy_text`; tray icon+menu; spawns the hotkey
  listener + dictation worker + mic-level emitter; emits `overlay-state`,
  `overlay-level`, `history-updated`.
- `ui/overlay.{html,js}` — transparent click-through pill; canvas port of the
  design's own render code; listens to `overlay-state`/`overlay-level`.
- `ui/settings.{html,css,js}` — the 7-section window; calls the commands above;
  browser-mock fallback so it previews standalone.
- `ui/fonts/` + `ui/fonts.css` — bundled Inter + JetBrains Mono (the exact design
  fonts) for offline, pixel-matching typography.
- `tauri.conf.json`, `build.rs`, `capabilities/default.json`, `icons/` — Tauri scaffold.

### Preview the finished frontend now (no build needed)
```
node scripts/serve-ui.mjs
# http://localhost:5173/settings.html   and   /preview.html
```

## Verify (definition of done for Path B)
Hold the hotkey → overlay shows Listening (live waveform) → release →
Transcribing → (Polishing) → Done, text injected into the focused app. Toggle
mode: press once to start, again to stop. Tray: pause, switch polish tier, open
settings, quit. Settings: change hotkey/activation/polish/threshold/dictionary,
toggle launch-at-login — all persist to `D:\sotto\config.toml` and take effect
live. History rows re-copy on click.
