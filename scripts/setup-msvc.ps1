# Sets up the MSVC C++ toolchain for building Sotto on the windows-msvc target
# (required by Tauri / WebView2), keeping the big pieces off the space-tight C: drive.
#
# RUN AS ADMINISTRATOR:
#   Right-click Start > "Terminal (Admin)", then:
#   pwsh -ExecutionPolicy Bypass -File D:\Coding\sotto\scripts\setup-msvc.ps1
#
# What lands where:
#   - VS download cache (several GB, transient)      -> D:\VS\cache   (redirected)
#   - MSVC Build Tools (compiler/linker toolset)     -> D:\VS\BuildTools
#   - Windows 11 SDK (shared MSIs, not relocatable)  -> C: (~2 GB)
#   - Rust build output (target\)                    -> already on D: (repo is on D:)
# Reclaim transient space anytime with sotto-cleanup.ps1.

#Requires -RunAsAdministrator
$ErrorActionPreference = 'Stop'

Write-Host "== Sotto MSVC setup ==" -ForegroundColor Cyan

# 1) Redirect the VS package download cache to D: before installing.
$cache = 'D:\VS\cache'
New-Item -ItemType Directory -Force -Path $cache | Out-Null
$setupKey = 'HKLM:\SOFTWARE\Microsoft\VisualStudio\Setup'
New-Item -Path $setupKey -Force | Out-Null
Set-ItemProperty -Path $setupKey -Name 'CachePath' -Value $cache -Type String
Write-Host "VS download cache redirected to $cache"

# 2) Install (or modify) the Build Tools with the minimal C++ components.
#    NB: vswhere says NO VS product is installed on this machine (the 2022
#    folder under Program Files (x86) is an empty shell), so the default path
#    is a fresh `install` targeting D:. If a product does exist, `modify` it.
$vsRoot    = "${env:ProgramFiles(x86)}\Microsoft Visual Studio"
$installer = "$vsRoot\Installer\setup.exe"
$vswhere   = "$vsRoot\Installer\vswhere.exe"
if (-not (Test-Path $installer)) { throw "VS Installer not found at $installer" }

$components = ' --add Microsoft.VisualStudio.Component.VC.Tools.x86.x64' +
              ' --add Microsoft.VisualStudio.Component.Windows11SDK.22621'
$existing = & $vswhere -products Microsoft.VisualStudio.Product.BuildTools -format json |
            ConvertFrom-Json
if ($existing) {
  $installPath = $existing[0].installationPath
  $argStr = 'modify --installPath "' + $installPath + '"' + $components +
            ' --passive --norestart'
  Write-Host "Existing Build Tools found at $installPath - modifying."
} else {
  $installPath = 'D:\VS\BuildTools'
  $argStr = 'install --channelUri https://aka.ms/vs/17/release/channel' +
            ' --channelId VisualStudio.17.Release' +
            ' --productId Microsoft.VisualStudio.Product.BuildTools' +
            ' --installPath "' + $installPath + '"' + $components +
            ' --passive --norestart'
  Write-Host "No Build Tools product installed - fresh install to $installPath."
}

# NB: the installed setup.exe does NOT accept --wait (bootstrapper-only), and a
# -ArgumentList ARRAY does not quote spaces, so pass ONE pre-quoted string.
# --passive shows a progress window; the install runs in the VS Installer
# service, so we wait by polling for the toolset.
Write-Host "Launching the VS Installer (a progress window will appear; downloads a few GB)..."
Start-Process -FilePath $installer -ArgumentList $argStr

# 3) Wait for the toolset to appear, then report free space.
$vcvars = "$installPath\VC\Auxiliary\Build\vcvars64.bat"
$deadline = (Get-Date).AddMinutes(40)
Write-Host "Waiting for the MSVC toolset to finish installing" -NoNewline
while (-not (Test-Path $vcvars) -and (Get-Date) -lt $deadline) {
  Start-Sleep -Seconds 15
  Write-Host "." -NoNewline
}
Write-Host ""
if (Test-Path $vcvars) {
  Write-Host "SUCCESS: MSVC ready ($vcvars)" -ForegroundColor Green
  Write-Host "Next: in the repo run  rustup override set stable-x86_64-pc-windows-msvc" -ForegroundColor Yellow
} else {
  Write-Host "Not finished yet - let the VS Installer window complete, then re-run this script to confirm." -ForegroundColor Red
}
Get-PSDrive C, D | Select-Object Name, @{n='FreeGB'; e={[math]::Round($_.Free/1GB,1)}}
