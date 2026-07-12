# Running Sotto

## One-time setup

The build needs MinGW's `dlltool.exe` on `PATH` (the `windows-gnu` target uses
it for raw-dylib import libs; the linker itself is already pinned in
`.cargo/config.toml`). Add it to your shell for the session:

```powershell
$env:PATH = "D:\Coding\Tools\mingw64\bin;$env:PATH"
```

```bash
export PATH="/d/Coding/Tools/mingw64/bin:$PATH"
```

Data dir (`D:\sotto` by default, override with `SOTTO_DATA_DIR`) already has
everything the app needs: `onnxruntime.dll`, the Parakeet ASR model, and the
`llama-server` sidecar + Qwen2.5 model for AI polish. Nothing else to install.

## Run the real app

```
cargo run
```

Hold **Right Ctrl**, speak, release. You'll see the overlay pill go
Listening → Transcribing → (Polishing) → Done, and the transcribed text is
typed/pasted into whatever app has focus. The tray icon turns cyan while
listening; right-click it for Pause / Polish tier / Settings / Quit.

Only one instance runs at a time — launching a second one while the first is
up just exits.

## Preview pieces without a mic

- **Overlay pill only**, cycling all 5 states with a synthetic mic level:
  ```
  cargo run -- --overlay-demo
  ```
- **Settings window only**, opened immediately:
  ```
  cargo run -- --settings
  ```

## Other dev flags

- `cargo run -- --transcribe <file.wav>` — run ASR on a 16kHz mono WAV and print the text.
- `cargo run -- --polish "<raw text>"` — force the AI polish path on arbitrary text and print raw vs. polished.
- `SOTTO_LOG=debug cargo run` — more verbose logging (default `info`).

## Config

Settings persist to `D:\sotto\config.toml` (hotkey, activation mode, polish
tier/threshold, dictionary). Edit it by hand or through the Settings window —
both take effect live, no restart needed.

Dictation **history** (recent transcripts, click to re-copy) lives only in
memory for the current run — it resets on restart, by design.
