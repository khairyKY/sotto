# Running Sotto

## One-time setup

Sotto builds on the **msvc** toolchain (Tauri/WebView2 requires it — the old
gnu/mingw toolchain is gone from `master`). Setup and the exact build
recipe live in `docs/updating.md` §2 — follow that instead of a copy here
that would drift out of sync. Short version: install MSVC (`docs/msvc-setup.md`),
`rustup override set stable-x86_64-pc-windows-msvc`, then build through
`vcvars64.bat` so MSVC's PATH/LIB/INCLUDE are present.

Config lives in `%APPDATA%\sotto\config.toml`. The ~2.8 GB of models and the
llama runtime sidecar live wherever that config's `assets_dir` points — on
this machine that's `D:\sotto` (models, runtime, recordings). Override the
whole data dir (config + assets) for one run with `SOTTO_DATA_DIR`, e.g. for
tests or a portable setup.

## Run the real app

```
cargo run
```

(if this fails with a compiler/linker error, you're missing the msvc build
environment — see `docs/updating.md` §2)

Hold **Right Ctrl**, speak, release. You'll see the overlay pill go
Listening → Transcribing → (Polishing) → Done, and the transcribed text is
typed/pasted into whatever app has focus. The tray mark turns accent-lilac while
listening; right-click it for Pause / Insights / Dictionary / History / Retry
last dictation / Settings / Quit.

Only one instance runs at a time — launching a second one while the first is
up just exits.

## Preview the UI without building Rust

```
node scripts/serve-ui.mjs
# http://localhost:5173/index.html   and   /preview.html
```

No Rust build needed — the pages have a browser-mock fallback for the Tauri
calls they'd normally make.

## Other dev flags

- `cargo run -- --transcribe <file.wav>` — run ASR on a 16kHz mono WAV and print the text.
- `cargo run -- --polish "<raw text>"` — force the AI polish path on arbitrary text and print raw vs. polished.
- `cargo run -- --polish-qa [cases.jsonl]` — run the fixed AI-polish QA set (`tests/polish-qa/cases.jsonl`, format in `src/polish_qa.rs`) and print a pass/fail table; exits non-zero on a failure that isn't a marked known limit. Uses default settings, only `[llm]` comes from `config.toml`, so run it with a temp `SOTTO_DATA_DIR` holding the model (and a free `llm.port`) to stay off the installed app's sidecar.
- `SOTTO_LOG=debug cargo run` — more verbose logging (default `info`).

There is no `--overlay-demo` or `--settings` flag — the overlay and settings
windows are the Tauri app's normal windows now, not standalone dev modes.

## Config

Settings persist to `%APPDATA%\sotto\config.toml` (hotkey, activation mode,
polish tier/threshold, dictionary, `assets_dir`, etc). Edit it by hand or
through the Settings window — both take effect live, no restart needed.

Dictation **history** (recent transcripts, click to re-copy) lives in memory
only by default and resets on restart. Turn on **Keep history** in Settings →
Data & privacy (or `persist_history` in `config.toml`) to mirror it to
`history.jsonl` in the data dir and reload it on the next launch (capped at
500 entries / 1 MB).

---

## Old (egui era) — superseded 2026-09-28, kept for history

Sotto used to build on the gnu toolchain and ship an egui UI. None of the
following applies to `master` anymore:

~~The build needs MinGW's `dlltool.exe` on `PATH` (the `windows-gnu` target
uses it for raw-dylib import libs; the linker itself is already pinned in
`.cargo/config.toml`).~~

~~**Overlay pill only**, cycling all 5 states with a synthetic mic level:
`cargo run -- --overlay-demo`~~

~~**Settings window only**, opened immediately: `cargo run -- --settings`~~

~~Settings persist to `D:\sotto\config.toml`.~~ (config now always lives in
`%APPDATA%\sotto\config.toml`; `D:\sotto` is this machine's `assets_dir`, not
where the config file itself lives)
