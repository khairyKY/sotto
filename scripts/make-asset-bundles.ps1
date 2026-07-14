# Bundles the heavy, rarely-changing assets (ASR + LLM models, llama runtime)
# into the archives the app downloads on first run, then prints the one command
# to publish them as the stable `assets-v1` GitHub release.
#
# Run this ONCE (and again only when the models/runtime themselves change — in
# which case bump the tag here AND `ASSET_BASE` in src/assets.rs together).
#
#   pwsh -File scripts/make-asset-bundles.ps1
#   # then run the printed `gh release create ...` line

param(
  [string]$DataDir = 'D:\sotto',
  [string]$Out     = 'D:\sotto\_release-assets',
  [string]$Tag     = 'assets-v1'
)
$ErrorActionPreference = 'Stop'

New-Item -ItemType Directory -Force -Path $Out | Out-Null

Write-Host "Zipping Parakeet model..." -ForegroundColor Cyan
Compress-Archive -Path (Join-Path $DataDir 'models\parakeet-tdt-0.6b-v3-int8\*') `
  -DestinationPath (Join-Path $Out 'parakeet-tdt-0.6b-v3-int8.zip') -Force

Write-Host "Zipping llama runtime..." -ForegroundColor Cyan
Compress-Archive -Path (Join-Path $DataDir 'runtime\llama\*') `
  -DestinationPath (Join-Path $Out 'llama-runtime.zip') -Force

Write-Host "Copying single-file assets..." -ForegroundColor Cyan
Copy-Item (Join-Path $DataDir 'onnxruntime.dll') $Out -Force
Copy-Item (Join-Path $DataDir 'models\qwen2.5-1.5b-instruct-q4_k_m.gguf') $Out -Force

Get-ChildItem $Out | Select-Object Name, @{n='MB';e={[math]::Round($_.Length/1MB,0)}} | Format-Table -AutoSize

Write-Host "`nAssets staged in $Out. Publish them with:" -ForegroundColor Green
Write-Host "  gh release create $Tag `"$Out\*`" --title `"Sotto assets ($Tag)`" --notes `"Model + runtime files, downloaded once on first run.`"" -ForegroundColor Yellow
Write-Host "`n(These file NAMES must match src/assets.rs. If the $Tag tag already exists, use: gh release upload $Tag `"$Out\*`" --clobber)"
