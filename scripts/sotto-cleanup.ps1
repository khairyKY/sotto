# Sotto disk cleanup protocol. Reclaims space that dev tooling accumulates,
# with the C: (system) drive as the priority since it's space-constrained.
#
# Usage (normal shell is fine; add -Deep to also nuke the Rust build cache):
#   pwsh -ExecutionPolicy Bypass -File D:\Coding\sotto\scripts\sotto-cleanup.ps1
#   pwsh -ExecutionPolicy Bypass -File D:\Coding\sotto\scripts\sotto-cleanup.ps1 -Deep

param([switch]$Deep)
$ErrorActionPreference = 'Continue'

function FreeGB($d) { [math]::Round((Get-PSDrive $d).Free / 1GB, 1) }
$beforeC = FreeGB 'C'; $beforeD = FreeGB 'D'

function Nuke($path, $label) {
  if (Test-Path $path) {
    $before = (Get-ChildItem $path -Recurse -File -ErrorAction SilentlyContinue |
               Measure-Object Length -Sum).Sum
    Remove-Item "$path\*" -Recurse -Force -ErrorAction SilentlyContinue
    Write-Host ("  {0,-28} freed ~{1} MB" -f $label, [math]::Round(($before/1MB),0))
  }
}

Write-Host "== Sotto cleanup ==" -ForegroundColor Cyan

# VS package download cache (redirected to D: by setup-msvc.ps1) - safe to empty
# after installs; VS re-downloads if it ever needs a repair.
Nuke 'D:\VS\cache' 'VS download cache'

# Per-user + system temp.
Nuke $env:TEMP 'User temp'
Nuke 'C:\Windows\Temp' 'Windows temp'

# Old VS installer logs/telemetry that pile up on C:.
Nuke "$env:TEMP\..\..\Local\Microsoft\VisualStudio\Packages\_Instances" 'VS instance cache'

if ($Deep) {
  # Rust build output for this repo (on D:). Frees the most, but the next build
  # recompiles everything from scratch.
  Push-Location 'D:\Coding\sotto'
  Write-Host "  cargo clean (full rebuild next time)..."
  cargo clean 2>$null
  Pop-Location
}

Write-Host "`nWindows Disk Cleanup (optional, deeper system cleanup): cleanmgr /d C:" -ForegroundColor DarkGray
Write-Host ("`nC: {0} -> {1} GB free   |   D: {2} -> {3} GB free" -f `
  $beforeC, (FreeGB 'C'), $beforeD, (FreeGB 'D')) -ForegroundColor Green
