# Sotto — updates & distribution

How Sotto ships and updates, and the exact steps to cut a release.

## The idea: split the app from its assets

The app that changes on every update (the Rust exe + the HTML/JS UI) is a few
MB. The models + CUDA runtime are ~1.8 GB and **almost never change**. So they
are shipped separately:

| Piece | Size | Where it lives | How it updates |
|---|---|---|---|
| **App** (exe + UI) | ~10–20 MB | NSIS installer on GitHub Releases | Tauri updater — toast + one-click, in-app |
| **Assets** (models, runtime) | ~1.8 GB | `assets-v1` GitHub release | Downloaded once on first run; re-downloaded ~never |

Result: a user updates by clicking one button and ~15 MB moves, not 1.8 GB.
The heavy assets are fetched a single time and reused across all future updates.

- App updater config: `tauri.conf.json` → `plugins.updater` (endpoint + pubkey).
- First-run downloader: `src/assets.rs` (manifest + `ASSET_BASE` tag).
- Signing key: `D:\Coding\sotto-signing\sotto-updater.key` (**private — never
  commit**; deliberately outside `D:\sotto` so an "uninstall + delete data"
  can never nuke it). Public key is embedded in `tauri.conf.json`.

---

## One-time setup

### 1. Publish the assets release (`assets-v1`)

Only redone if the models/runtime themselves change (then bump the tag here
**and** `ASSET_BASE` in `src/assets.rs` together).

```powershell
pwsh -File scripts/make-asset-bundles.ps1
# prints the gh command; run it (needs `gh auth login` once):
gh release create assets-v1 "D:\sotto\_release-assets\*" `
  --title "Sotto assets (assets-v1)" `
  --notes "Model + runtime files, downloaded once on first run."
```

The archive **file names** must match `src/assets.rs`:
`onnxruntime.dll`, `parakeet-tdt-0.6b-v3-int8.zip`,
`qwen2.5-1.5b-instruct-q4_k_m.gguf`, `llama-runtime.zip`.

### 2. Keep the signing key safe

`D:\Coding\sotto-signing\sotto-updater.key` signs every update. If you lose it, existing users
can't verify future updates and must reinstall manually. Back it up somewhere
private. It currently has **no password** — to add one later, regenerate with
`npx tauri signer generate -p <password>` and update the pubkey in
`tauri.conf.json` (this invalidates updates for users on the old key, so do it
before you have real users).

---

## Cutting an app update (every release)

1. **Bump the version** in `tauri.conf.json` (`"version"`) and `Cargo.toml`.
   The updater compares this against the running app's version.

2. **Build, signed** (from the repo root, in an MSVC dev shell):

   ```powershell
   $env:TAURI_SIGNING_PRIVATE_KEY = Get-Content D:\Coding\sotto-signing\sotto-updater.key -Raw
   $env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''   # empty = no password
   npx tauri build
   ```

   Produces `target/release/bundle/nsis/Sotto_<ver>_x64-setup.exe` and a
   matching `.sig`.

3. **Generate `latest.json`:**

   ```powershell
   pwsh -File scripts/make-latest-json.ps1 -Version <ver> -Notes "What changed"
   ```

4. **Publish** the release (tag **must** be `v<ver>`), attaching both files:

   ```powershell
   gh release create v<ver> `
     "target/release/bundle/nsis/Sotto_<ver>_x64-setup.exe" `
     "target/release/bundle/nsis/latest.json" `
     --title "Sotto v<ver>" --notes "What changed"
   ```

That's it. The updater endpoint
`https://github.com/khairyKY/sotto/releases/latest/download/latest.json`
always resolves to the newest release, so every running app picks it up.

---

## What the user sees

- On launch, Sotto checks GitHub. If a newer version exists, a **native Windows
  toast** appears ("Sotto update available").
- Opening **Settings** shows a banner ("Sotto vX is available") with
  **Install & restart**, plus an **About** row with the current version and a
  **Check for updates** button.
- Clicking Install downloads the small installer, verifies its signature,
  installs, and relaunches. Models are untouched.
- A brand-new user runs the ~15 MB installer, and on first launch Settings shows
  a **"Downloading voice models…"** progress bar while the ~1.8 GB assets fetch
  once. After that, dictation works and updates are always tiny.

## Notes / limits

- Updates only work in an **installed** build, not `cargo run` / `tauri dev`
  (there's no installed version to replace) — `check()` returns `ReleaseNotFound`
  in dev, which is handled silently.
- The first-run download has **no resume** yet: a dropped connection restarts the
  current file (never corrupts it). Add HTTP `Range` resume in `assets.rs` if
  flaky-link retries become a real problem.
- GitHub release assets cap at 2 GB/file; all Sotto assets are under that.
