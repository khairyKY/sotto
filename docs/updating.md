# Sotto — deployment & releases

The complete guide: how Sotto ships, how it updates, and the exact steps to cut
a release. Last verified against the repo and against GitHub on **2026-09-23**.

> **Standing instruction: do not push commits and do not publish a release until
> Kai explicitly says so.** Everything from §4 step 6 onward is gated on that.
> The build and verify steps are always safe to run.
>
> **Update, 2026-09-28:** Kai made the Conductor loop fully autonomous for
> push + publish. The Conductor no longer waits for a go-ahead to push commits
> or publish a release through the gate below. It still waits for Kai's word
> before swapping his installed build (§4 step 3's smoke test runs against
> that installed copy), and a public release only follows that installed smoke
> test passing.

---

## 0. Where things stand right now

| | |
|---|---|
| Local version | **v0.6.0** (`Cargo.toml` + `tauri.conf.json`) |
| Local HEAD | `2cfa6be` |
| `origin/master` | `1544e0b` — **18 commits behind** |
| Latest *published* release | **v0.3.0** (2026-07-17) |
| Asset releases | `assets-v1`, `assets-v2` (both published, pre-release) |

Anyone running the public build is on v0.3.0 and is missing v0.4.x, v0.5.x and
v0.6.0 entirely. That is deliberate, per the instruction above.

**Update, 2026-09-28:** the snapshot above has aged (it was accurate when
written on 2026-09-23). As of today, `origin/master` = local master, both at
`ab09d50` — no gap. PR #38 is merged. The next release to cut is **v0.6.1**.
The old snapshot is left in place above for history; don't treat its numbers
as current.

### Known release blocker — fix before ever publishing again

`src/assets.rs` manifests the Egyptian model as
`ggml-egyptian-codeswitch-small.bin` under tag `assets-v2`, but **that file was
never uploaded**. Verified 2026-09-23:

```
200  assets-v1/onnxruntime.dll
200  assets-v1/parakeet-tdt-0.6b-v3-int8.zip
200  assets-v1/qwen2.5-1.5b-instruct-q4_k_m.gguf
200  assets-v1/llama-runtime.zip
200  assets-v2/ggml-large-v3-turbo-q5_0.bin
404  assets-v2/ggml-egyptian-codeswitch-small.bin   <-- selecting this model 404s
```

The built model exists locally at
`D:\sotto\models\ggml-egyptian-codeswitch-small.bin` (465 MiB). Either upload it
(§5) or remove that engine from the model picker before shipping — a user who
picks it today gets a failed download and a stashed take.

---

## 1. The shipping model: app and assets are separate

The part that changes every release (Rust exe + HTML/JS UI) is ~16 MB. The
models and runtime are ~2.1 GB and **almost never change**. They ship apart:

| Piece | Size | Lives in | Updates via |
|---|---|---|---|
| **App** (exe + UI) | ~16 MB | NSIS installer on GitHub Releases, tag `v<ver>` | Tauri updater — in-app banner, one click |
| **Assets** (models, runtime) | ~2.1 GB | `assets-v1` / `assets-v2` releases | Downloaded once on first run, then never again |

A user updates by clicking one button and ~16 MB moves, not 2.1 GB.

**Asset tags are decoupled from the app version on purpose.** Bump an asset tag
only when that asset's *bytes* change — never on an ordinary app release.

Where each piece is configured:

- Updater endpoint + public key → `tauri.conf.json` → `plugins.updater`
- First-run downloader + asset manifest → `src/assets.rs` (`RELEASE_BASE`, `Asset.tag`)
- Installer behaviour → `tauri.conf.json` → `bundle.windows.nsis` + `nsis/hooks.nsi`
- Signing key → `D:\Coding\sotto-signing\sotto-updater.key` (**private, never commit**)

### Current asset layout

| Tag | File | Size | Needed by |
|---|---|---|---|
| `assets-v1` | `onnxruntime.dll` | 13 MB | always |
| `assets-v1` | `parakeet-tdt-0.6b-v3-int8.zip` | 446 MB | `parakeet-v3` (default) |
| `assets-v1` | `qwen2.5-1.5b-instruct-q4_k_m.gguf` | 1065 MB | AI polish |
| `assets-v1` | `llama-runtime.zip` | 647 MB | AI polish |
| `assets-v2` | `ggml-large-v3-turbo-q5_0.bin` | 547 MB | `whisper-turbo` |
| `assets-v2` | `ggml-egyptian-codeswitch-small.bin` | — | `egyptian-small` — **MISSING** |

Exactly one ASR model is ever fetched (`asr_asset()` picks by config), so a
Parakeet user never downloads Whisper's 547 MB and vice versa.

---

## 2. Build environment — mandatory, not optional

whisper.cpp **compiles from source** on every clean build, so the native
toolchain has to be on the shell. Miss one of these and the failure will not
name its real cause.

Verified present on this machine (2026-09-23):

```
D:\VS\BuildTools\VC\Auxiliary\Build\vcvars64.bat
D:\Coding\Tools\VulkanSDK
D:\Coding\Tools\cmake\bin\cmake.exe
D:\Coding\Tools\ninja\ninja.exe
D:\Coding\Tools\Python\Lib\site-packages\clang\native
D:\Coding\sotto-signing\sotto-updater.key
```

> Only the Windows SDK (~0.9 GB) is still on C:, because it cannot be relocated;
> the MSVC toolset itself lives on D:. `docs/msvc-setup.md` was corrected to
> match on 2026-09-23.

### The Rust toolchain lives on D: (reinstalled 2026-09-24)

`rustup`/`cargo` are **not** in the default `%USERPROFILE%\.cargo` / `.rustup`.
They were reinstalled to D: to keep C: clear, via persistent user env vars:

```
CARGO_HOME  = D:\Coding\.cargo   (cargo.exe, rustup.exe, all shims in \bin)
RUSTUP_HOME = D:\Coding\.rustup  (toolchains: stable-x86_64-pc-windows-msvc)
```

If `cargo` is "not recognized", these are why — a shell that predates the env
change won't have `D:\Coding\.cargo\bin` on PATH. Call cargo by full path
(`D:\Coding\.cargo\bin\cargo.exe`) or open a fresh shell. Reinstall if ever
lost: set both env vars first, then `winget install Rustlang.Rustup`.

**Reliable build invocation** (MSYS/Git-Bash mangles inline `cmd` quoting — do
not fight it). Drive builds through a PowerShell wrapper that imports the
vcvars environment and calls cargo by full path:

```powershell
$env:CARGO_HOME='D:\Coding\.cargo'; $env:RUSTUP_HOME='D:\Coding\.rustup'
$env:LIBCLANG_PATH='D:\Coding\Tools\Python\Lib\site-packages\clang\native'
$env:VULKAN_SDK='D:\Coding\Tools\VulkanSDK'
cmd /c '"D:\VS\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1 && set' |
  ForEach-Object { if ($_ -match '^([^=]+)=(.*)$') { Set-Item "env:$($matches[1])" $matches[2] } }
$env:PATH += ';D:\Coding\.cargo\bin;D:\Coding\Tools\cmake\bin;D:\Coding\Tools\ninja;D:\Coding\Tools\VulkanSDK\Bin'
& 'D:\Coding\.cargo\bin\cargo.exe' <args>
```

A first clean build compiles whisper.cpp from source (~30 min); incremental
builds are ~2–10 min.

### The env block — paste at the top of any build shell (PowerShell)

```powershell
$env:VULKAN_SDK    = "D:\Coding\Tools\VulkanSDK"
$env:PATH          = "$env:PATH;D:\Coding\Tools\cmake\bin;D:\Coding\Tools\ninja;D:\Coding\Tools\VulkanSDK\Bin"
$env:LIBCLANG_PATH = "D:\Coding\Tools\Python\Lib\site-packages\clang\native"
```

Then run cargo/tauri **through `vcvars64.bat`** so MSVC's own PATH/LIB/INCLUDE
are present:

```powershell
cmd /c 'call "D:\VS\BuildTools\VC\Auxiliary\Build\vcvars64.bat" > nul 2>&1 && cargo test --bin sotto 2>&1'
```

### What is already handled for you

`.cargo/config.toml` pins two things so you cannot forget them:

- `CMAKE_TOOLCHAIN_FILE = scripts/msvc-avx2-toolchain.cmake` (`relative = true`)
  — restores `/O2` and turns ggml's SIMD kernels on. cmake-rs strips `/O2`, and
  `GGML_NATIVE=ON` is a **no-op on MSVC**. Without this the build is scalar and
  unoptimized, and the failure mode is a silently ~20x slower binary, **not an
  error**: 237 s to transcribe 1.5 s of audio, with a perfectly correct
  transcript.
- `CMAKE_GENERATOR = "Ninja"` — ggml's Vulkan backend builds a nested
  `vulkan-shaders-gen` whose MSBuild `.tlog` paths blow past Windows' 260-char
  MAX_PATH. The error it surfaces is the useless *"The C compiler is not able to
  compile a simple test program"*. Ninja never creates those paths.

### Signing

```powershell
$env:TAURI_SIGNING_PRIVATE_KEY          = Get-Content D:\Coding\sotto-signing\sotto-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''
```

The key lives outside `D:\sotto` on purpose, so an "uninstall + delete my data"
can never destroy it. The matching public key is embedded in `tauri.conf.json`.

### GitHub CLI

`gh` 2.95.0, authenticated as `khairyKY`. Needed only for publishing.

---

## 3. Pre-flight — run every one of these before you build

```powershell
# 1. Tests
cmd /c 'call "D:\VS\BuildTools\VC\Auxiliary\Build\vcvars64.bat" > nul 2>&1 && cargo test --bin sotto 2>&1' | Select-String "test result"

# 2. Frontend syntax
node --check ui/index.js
node --check ui/overlay.js

# 3. The [hidden] guard — author CSS `display` beats the UA's
#    `[hidden] { display: none }`, a bug class this project keeps hitting
python scripts/check-hidden.py

# 4. Version bumped in BOTH files, and they agree
Select-String '^version' Cargo.toml ; Select-String '"version"' tauri.conf.json
```

### 5. Asset-manifest audit — the check that would have caught the 404 above

Every `file` / `tag` pair in `src/assets.rs` must resolve to a real download.
Run this **before every release**, not only when assets change:

```bash
for u in \
  "assets-v1/onnxruntime.dll" \
  "assets-v1/parakeet-tdt-0.6b-v3-int8.zip" \
  "assets-v1/qwen2.5-1.5b-instruct-q4_k_m.gguf" \
  "assets-v1/llama-runtime.zip" \
  "assets-v2/ggml-large-v3-turbo-q5_0.bin" \
  "assets-v2/ggml-egyptian-codeswitch-small.bin" ; do
  code=$(curl -sI -L --max-time 20 -o /dev/null -w '%{http_code}' \
    "https://github.com/khairyKY/sotto/releases/download/$u")
  echo "$code  $u"
done
```

Anything other than `200` is a release blocker. Use `curl -sI` (HEAD) — a plain
`curl -L -o /dev/null` downloads all 2.7 GB.

---

## 4. Cutting an app release

### Step 1 — bump the version in both files

They must match; the updater compares `latest.json`'s version against the
running app's.

```powershell
$v = "0.6.1"
(Get-Content Cargo.toml)      -replace '^version = ".*"', "version = `"$v`""     | Set-Content Cargo.toml
(Get-Content tauri.conf.json) -replace '"version": ".*"',  "`"version`": `"$v`"" | Set-Content tauri.conf.json
```

### Step 2 — build, signed

Close any running Sotto first, or the build fails with
`failed to remove file target\release\sotto.exe`.

```powershell
Get-Process sotto -ErrorAction SilentlyContinue | Stop-Process -Force

$env:VULKAN_SDK    = "D:\Coding\Tools\VulkanSDK"
$env:PATH          = "$env:PATH;D:\Coding\Tools\cmake\bin;D:\Coding\Tools\ninja;D:\Coding\Tools\VulkanSDK\Bin"
$env:LIBCLANG_PATH = "D:\Coding\Tools\Python\Lib\site-packages\clang\native"
$env:TAURI_SIGNING_PRIVATE_KEY          = Get-Content D:\Coding\sotto-signing\sotto-updater.key -Raw
$env:TAURI_SIGNING_PRIVATE_KEY_PASSWORD = ''

cmd /c 'call "D:\VS\BuildTools\VC\Auxiliary\Build\vcvars64.bat" > nul 2>&1 && npx tauri build 2>&1'
```

`npx tauri build` runs the local `@tauri-apps/cli` from `node_modules`.
(`cargo tauri` is **not** installed — `cargo tauri build` fails with "no such
command". `node_modules\.bin\tauri.cmd build` also works.)

Produces, in `target/release/bundle/nsis/`:

- `Sotto_<ver>_x64-setup.exe` (~16 MB)
- `Sotto_<ver>_x64-setup.exe.sig` — **if this is missing, the signing env var was
  not set.** Stop; the updater will reject the release.

### Step 3 — install locally and smoke-test

Never publish a build you have not run.

```powershell
Get-Process sotto -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Process "target\release\bundle\nsis\Sotto_${v}_x64-setup.exe" -ArgumentList "/S" -Wait
Start-Process "D:\Installations\Sotto\Sotto.exe"
Start-Sleep -Seconds 8
Get-Process sotto | Select-Object Id, Responding, @{n='Ver';e={$_.MainModule.FileVersionInfo.FileVersion}}
```

**Check `Responding`, not just that the process exists.** A self-deadlock once
shipped because `Get-Process` showed the app alive while `Responding: False` sat
in output that got skimmed past.

Then do one real dictation and confirm text lands. Logs live in
`%APPDATA%\sotto\logs\sotto.log`.

> Headless trick: with `activation_mode = "toggle"` the hotkey can be driven by a
> synthetic `ControlRight` (VK `0xA3`, extended flag) via `keybd_event`, then read
> the log. **Caveat:** dictating an empty room can hallucinate a phrase and
> **inject it into whatever window has focus** — park the cursor somewhere
> harmless first.

### Step 4 — generate `latest.json`

```powershell
pwsh -File scripts/make-latest-json.ps1 -Version $v -Notes "What changed"
```

Reads the `.sig`, writes `target/release/bundle/nsis/latest.json`, and prints the
`gh release create` line. It filters by version, so a stale older `setup.exe`
sitting alongside cannot be picked up by mistake.

### Step 5 — commit and tag locally

```powershell
git add -A
git commit -m "v$v: <summary>"
git tag "v$v"
```

### Step 6 — GATE: get Kai's explicit go-ahead

Everything below publishes to the world. Do not proceed without it.

### Step 7 — push and publish

```powershell
git push origin master
git push origin "v$v"

gh release create "v$v" `
  "target/release/bundle/nsis/Sotto_${v}_x64-setup.exe" `
  "target/release/bundle/nsis/latest.json" `
  --title "Sotto v$v" --notes "What changed"
```

The tag **must** be `v<ver>` — `make-latest-json.ps1` writes the download URL as
`releases/download/v$Version/...`, and a mismatch 404s every updater.

### Step 8 — verify the published release

```bash
# latest.json resolves and reports the new version
curl -sL https://github.com/khairyKY/sotto/releases/latest/download/latest.json

# the installer URL inside it actually resolves
curl -sI -L -o /dev/null -w '%{http_code}\n' \
  "https://github.com/khairyKY/sotto/releases/download/v<ver>/Sotto_<ver>_x64-setup.exe"
```

Both must be correct or running apps see a broken update. Best final check:
install the *previous* version, launch it, and confirm the in-app update banner
appears and completes.

---

## 5. Publishing or refreshing assets

Only when the asset bytes themselves change.

**The rule: an asset's `tag` in `src/assets.rs` and the release it lives in must
change together, in the same commit.** Bumping one without the other is how you
ship an app that 404s on first run for every new user.

### Staging the `assets-v1` bundle

```powershell
pwsh -File scripts/make-asset-bundles.ps1      # -DataDir D:\sotto -Out D:\sotto\_release-assets
```

Zips Parakeet and the llama runtime, copies `onnxruntime.dll` and the Qwen GGUF,
prints sizes and the publish command. **File names must match `src/assets.rs`
exactly.**

### Publishing

```powershell
# first time for a tag
gh release create assets-v1 "D:\sotto\_release-assets\*" `
  --title "Sotto assets (assets-v1)" `
  --notes "Model + runtime files, downloaded once on first run." --prerelease

# tag already exists — add or replace files in place
gh release upload assets-v1 "D:\sotto\_release-assets\*" --clobber
```

### Fixing the Egyptian 404 specifically

```powershell
gh release upload assets-v2 "D:\sotto\models\ggml-egyptian-codeswitch-small.bin" --clobber
```

Then re-run the §3.5 audit and confirm it returns `200`.

Asset releases are marked **pre-release** so they never become "Latest" — the
updater endpoint resolves `releases/latest`, and an asset release winning that
slot would break every update check.

---

## 6. Rollback

There is no "unpublish" that reaches apps which already updated. Options, best
first:

1. **Roll forward.** Bump the patch version, fix, publish again. Almost always
   right — the updater only moves users *forward*.
2. **Delete the bad release** so `releases/latest` falls back to the previous
   one: `gh release delete v<bad> --yes --cleanup-tag`. Only helps users who have
   not updated yet.
3. If a bad `latest.json` shipped but the installer is fine, re-upload a
   corrected one: `gh release upload v<ver> latest.json --clobber`.

---

## 7. What the user actually experiences

1. **Install** — ~16 MB installer from the Releases page. Windows SmartScreen
   shows *"Windows protected your PC"* because the installer is not signed with a
   paid Authenticode certificate (~$100–200/yr, which this project does not
   have). **More info → Run anyway.** The README explains this honestly and at
   length; keep that wording in sync with reality.
2. **First launch** — Settings opens with a progress banner while ~2.1 GB of
   models download once into the assets dir (default `%APPDATA%\sotto`; this
   machine uses `D:\sotto` via `assets_dir`). Downloads resume on a dropped
   connection (HTTP `Range`, added in v0.5.3), and a stream that ends short of
   `Content-Length` is rejected rather than renamed into place.
3. **Updates** — on launch Sotto checks GitHub; a banner offers **Install &
   restart**. ~16 MB moves; models and settings are untouched.
4. **Uninstall** — `installMode: currentUser`, so no admin prompt and no HKLM
   entry. `nsis/hooks.nsi` kills `sotto.exe` and `llama-server.exe` first, then
   asks whether to also delete the models and settings — **default No**, so a
   click-through uninstall keeps the 2.1 GB for a fast reinstall.

---

## 8. Gotchas that have actually bitten this project

- **The updater does nothing in dev builds.** `cargo run` / `tauri dev` have no
  installed version to replace; `check()` returns `ReleaseNotFound` and is
  handled silently. Only test updates from an installed build.
- **Version bumped in one file only.** `Cargo.toml` and `tauri.conf.json` both
  carry it. Bump both or the updater compares against the wrong number.
- **Missing `.sig`.** Without `TAURI_SIGNING_PRIVATE_KEY` the build still
  succeeds and produces an unsigned installer. `make-latest-json.ps1` throws in
  that case — do not work around it.
- **Losing the signing key strands every existing user.** They cannot verify any
  future update and must reinstall by hand. Back up
  `D:\Coding\sotto-signing\sotto-updater.key` somewhere private. It currently has
  **no password**; adding one means regenerating and replacing the pubkey in
  `tauri.conf.json`, which invalidates updates for everyone on the old key — so
  only do that before there are real users.
- **Asset releases must stay pre-release** (see §5).
- **GitHub caps release assets at 2 GB/file.** Everything here is under it; the
  Qwen GGUF at 1065 MB is the largest.
- **Build fails with "failed to remove file ...\sotto.exe"** — a running Sotto is
  holding the binary. Kill it.
- **No CI.** There is no `.github/workflows`; every release is built and verified
  by hand on this machine. The §3 checklist *is* the pipeline.
- **Green tests are not proof.** A silence-skip guard once passed all 82 tests
  while silently destroying the chunked-transcription win; only reading the
  runtime log caught it. Always do step 3's smoke test.
