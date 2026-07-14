# Generates the `latest.json` the Tauri updater reads, from the signed installer
# produced by `npx tauri build`. Run AFTER a build, then upload BOTH the
# setup.exe and latest.json to the GitHub release for that version.
#
#   pwsh -File scripts/make-latest-json.ps1 -Version 0.1.1 -Notes "Fixes + faster polish"

param(
  [Parameter(Mandatory)][string]$Version,
  [string]$Notes = "",
  [string]$Repo  = "khairyKY/sotto"
)
$ErrorActionPreference = 'Stop'

$nsis  = "D:\Coding\sotto\target\release\bundle\nsis"
# Filter to the target Version and grab the newest match (guards against a
# stale 0.1.0 setup.exe sitting next to the fresh 0.1.1 one).
$setup = Get-ChildItem $nsis -Filter "*_${Version}_*-setup.exe" |
         Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $setup) { throw "No -setup.exe in $nsis. Run 'npx tauri build' first." }
$sigPath = "$($setup.FullName).sig"
if (-not (Test-Path $sigPath)) { throw "No signature at $sigPath. Was TAURI_SIGNING_PRIVATE_KEY set during build?" }

$latest = [ordered]@{
  version   = $Version
  notes     = $Notes
  pub_date  = (Get-Date).ToUniversalTime().ToString("yyyy-MM-ddTHH:mm:ssZ")
  platforms = [ordered]@{
    "windows-x86_64" = [ordered]@{
      signature = (Get-Content $sigPath -Raw).Trim()
      url       = "https://github.com/$Repo/releases/download/v$Version/$($setup.Name)"
    }
  }
}
$outFile = Join-Path $nsis 'latest.json'
$latest | ConvertTo-Json -Depth 6 | Out-File $outFile -Encoding utf8
Write-Host "Wrote $outFile" -ForegroundColor Green
Write-Host "`nPublish the release with:" -ForegroundColor Green
Write-Host "  gh release create v$Version `"$($setup.FullName)`" `"$outFile`" --title `"Sotto v$Version`" --notes `"$Notes`"" -ForegroundColor Yellow
