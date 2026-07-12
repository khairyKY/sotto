# MSVC toolchain + disk protocol (for the Tauri build)

Tauri renders through WebView2, which requires building Sotto on the
**`x86_64-pc-windows-msvc`** target — not the `windows-gnu` toolchain the
earlier egui build used. This is a one-time setup, made careful because the
**C: drive is space-constrained** (~9–10 GB free).

## Where things live

| Piece | Size | Drive | Note |
|---|---|---|---|
| VS download cache | several GB | **D:** `D:\VS\cache` | redirected by the setup script; transient, safe to purge |
| MSVC toolset + Windows SDK | ~4–5 GB | **C:** | the Windows SDK cannot be relocated off C: |
| Rust build output (`target\`) | grows | **D:** | the repo lives on D:, so this is already off C: |

Net cost to C: is ~4–5 GB (leaves ~5 GB free), and the cleanup script keeps it lean.

## 1. Install (once)

Run **as Administrator** (Start → "Terminal (Admin)"):

```powershell
pwsh -ExecutionPolicy Bypass -File D:\Coding\sotto\scripts\setup-msvc.ps1
```

It redirects the VS cache to D:, installs the minimal MSVC toolset + Windows 11
SDK, and prints free space when done. Then, in the repo, switch the project to
the MSVC toolchain:

```powershell
cd D:\Coding\sotto
rustup override set stable-x86_64-pc-windows-msvc
```

## 2. Building

Cargo needs the MSVC environment (PATH/LIB/INCLUDE) on the build shell. Either
build from a **"x64 Native Tools Command Prompt for VS 2022"**, or import the
environment into any shell first:

```powershell
$vc = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat"
cmd /c "`"$vc`" && set" | % { if ($_ -match '^([^=]+)=(.*)$') { Set-Item "Env:\$($matches[1])" $matches[2] } }
cargo build
```

## 3. Cleanup protocol

Reclaim space whenever C: gets tight:

```powershell
pwsh -File D:\Coding\sotto\scripts\sotto-cleanup.ps1        # temp + VS cache
pwsh -File D:\Coding\sotto\scripts\sotto-cleanup.ps1 -Deep  # + cargo clean (full rebuild after)
```

It empties the VS download cache (D:), user/system temp, and stale VS installer
caches, and reports before/after free space per drive. For a deeper system pass,
`cleanmgr /d C:`.

## Reverting to the old gnu build

The complete working egui version is on the `master` branch, toolchain gnu:

```powershell
git checkout master
rustup override set stable-x86_64-pc-windows-gnu
```
