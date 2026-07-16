# Regenerates the Sotto app icons from the Marshmallow design doc's #icons
# spec (treatment A — cream squircle + lilac 4-hump wave).
#
#   pwsh -File scripts/make-icons.ps1
#
# Why this exists instead of a plain `tauri icon icons/icon.svg`:
# the doc gives a DIFFERENT wave stroke-width per size (4.5 @96px, 5.5 @48px,
# 7 @32px) precisely so the mark stays legible when it shrinks. `tauri icon`
# rasterizes every size from one source, so small icons come out spindly —
# which is exactly what the taskbar and Start menu show. Here each size is
# rendered from its own SVG at the doc's stroke, then packed into icon.ico.
#
# `tauri icon` is still used afterwards for the store/mobile PNG sets, which
# are all large enough that the 96px stroke is correct.

$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$icons = Join-Path $root 'icons'
$tmp = Join-Path $env:TEMP 'sotto-icons'
New-Item -ItemType Directory -Force -Path $tmp | Out-Null

# size -> wave stroke-width (in the 56x24 viewBox's own units), per the doc.
$strokes = @{ 16 = 7.0; 24 = 7.0; 32 = 7.0; 48 = 5.5; 64 = 5.5; 128 = 4.5; 256 = 4.5 }

function New-IconSvg([double]$stroke) {
  # Wave is 46% of the box wide; scale 471/56 = 8.411 at a 1024 canvas. The
  # inner translate re-centres the path's own midpoint (28,12).
  @"
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024" width="1024" height="1024">
  <defs>
    <linearGradient id="bg" x1="21.3%" y1="9.0%" x2="78.7%" y2="91.0%">
      <stop offset="0%" stop-color="#F6F0E6"/>
      <stop offset="100%" stop-color="#E6DFD4"/>
    </linearGradient>
  </defs>
  <rect width="1024" height="1024" rx="225" ry="225" fill="url(#bg)"/>
  <g transform="translate(512,512) scale(8.411) translate(-28,-12)">
    <path d="M4 12 Q10 3 16 12 T28 12 T40 12 T52 12"
          fill="none" stroke="#8E74D0" stroke-width="$stroke"
          stroke-linecap="round" stroke-linejoin="round"/>
  </g>
</svg>
"@
}

Write-Host '== Rendering per-size PNGs (doc stroke per size) ==' -ForegroundColor Cyan
$pngs = @()
foreach ($size in ($strokes.Keys | Sort-Object)) {
  $svg = Join-Path $tmp "icon-$size.svg"
  $png = Join-Path $tmp "icon-$size.png"
  New-IconSvg $strokes[$size] | Set-Content -Path $svg -Encoding utf8
  magick -background none $svg -resize "${size}x${size}" $png
  $pngs += $png
  Write-Host ("  {0,3}px  stroke {1}" -f $size, $strokes[$size])
}

Write-Host '== Packing icon.ico (all sizes) ==' -ForegroundColor Cyan
magick @pngs (Join-Path $icons 'icon.ico')

Write-Host '== tauri icon for the PNG/store/mobile sets ==' -ForegroundColor Cyan
Push-Location $root
# The canonical source stays the 96px-treatment stroke; every size tauri
# generates from it is large enough for that to read correctly.
npx tauri icon icons/icon.svg 2>&1 | Select-Object -Last 2
Pop-Location

# tauri icon rewrites icon.ico from the single source — restore the per-size one.
Write-Host '== Re-packing icon.ico (tauri icon overwrites it) ==' -ForegroundColor Cyan
magick @pngs (Join-Path $icons 'icon.ico')

magick identify (Join-Path $icons 'icon.ico') | ForEach-Object { "  $_" }
Write-Host 'Done. Rebuild the app so the exe embeds the new icon.' -ForegroundColor Green
