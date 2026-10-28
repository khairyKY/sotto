# Build Sotto's themed setup window (setup/) around the NSIS installer that
# `tauri build` already made. Run it after the installer build:
#   pwsh -File scripts\build-setup.ps1
# Output: target\release\bundle\nsis\Sotto_<ver>_x64-setup-themed.exe
# The updater keeps shipping the plain NSIS installer; this one is for downloads.
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$ver = (Select-String '^version = "(.*)"' (Join-Path $root 'Cargo.toml')).Matches[0].Groups[1].Value
if (-not $env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR = Join-Path $root 'target' }
$nsis = Join-Path $env:CARGO_TARGET_DIR "release\bundle\nsis\Sotto_${ver}_x64-setup.exe"
if (-not (Test-Path $nsis)) { throw "build the installer first (tauri build): $nsis" }
$env:SOTTO_INSTALLER = $nsis
$env:SOTTO_VERSION = $ver
& D:\Coding\_claude-cloud\sotto-tools\cargo.ps1 -Dir (Join-Path $root 'setup') build --release --offline
if ($LASTEXITCODE) { exit $LASTEXITCODE }
$out = Join-Path $env:CARGO_TARGET_DIR "release\bundle\nsis\Sotto_${ver}_x64-setup-themed.exe"
Copy-Item (Join-Path $env:CARGO_TARGET_DIR 'release\sotto-setup.exe') $out -Force
"setup: $out ($([math]::Round((Get-Item $out).Length / 1MB, 1)) MB)"
