# Time one 16 kHz WAV through the app (`sotto.exe --transcribe`) and through
# examples/whisper_probe.rs, alternating so both see the same machine load, and
# print medians (#14). The app gets a throwaway SOTTO_DATA_DIR (never your real
# config) whose models\ holds a hard link to -Model, so -Data must be on the
# model's volume.
#
#   pwsh scripts/whisper-bench.ps1 -Model D:\sotto\models\ggml-large-v3-turbo-q5_0.bin -Wav clip.wav
#
# Build first: cargo build --release --bin sotto --example whisper_probe. The first
# run of a new exe also pays the GPU driver's one-time pipeline compile (tens of
# seconds); that shows up as the app's warmup_ms or the probe's run 1.
param(
  [Parameter(Mandatory)][string]$Model,
  [Parameter(Mandatory)][string]$Wav,
  [string]$Engine = 'whisper-turbo',   # must be the engine whose file -Model is
  [string]$Language = 'en',
  [string]$Target = $(if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { "$PSScriptRoot\..\target" }),
  [string]$Data = "$env:TEMP\sotto-bench",
  [int]$N = 5
)
$ErrorActionPreference = 'Stop'
$Wav = (Resolve-Path $Wav).Path
New-Item -ItemType Directory -Force "$Data\models" | Out-Null
$link = "$Data\models\$(Split-Path $Model -Leaf)"
if (-not (Test-Path $link)) { New-Item -ItemType HardLink -Path $link -Target $Model | Out-Null }
@"
hotkey = "ControlRight"
activation_mode = "hold"
injection_mode = "paste"

[asr]
model = "$Engine"
language = "$Language"
"@ | Set-Content "$Data\config.toml"
$env:SOTTO_DATA_DIR = $Data
$env:PROBE_LANG = $Language

function Median($xs) { $s = @($xs | Sort-Object); $s[[int][math]::Floor(($s.Count - 1) / 2)] }
$app = @(); $cold = @(); $warm = @()
for ($i = 1; $i -le $N; $i++) {
  # GUI-subsystem exe: `&` wouldn't wait for it.
  Start-Process "$Target\release\sotto.exe" -ArgumentList '--transcribe', "`"$Wav`"" -Wait -NoNewWindow `
    -RedirectStandardOutput "$Data\app_out.txt" -RedirectStandardError "$Data\app_err.txt"
  $log = Get-Content "$Data\logs\sotto.log" -Raw
  $f = { param($k) [regex]::Match($log, "$k=(\d+)").Groups[1].Value }
  "app   #$i load_ms=$(& $f load_ms) warmup_ms=$(& $f warmup_ms) transcribe_ms=$(& $f transcribe_ms)"
  $app += [int](& $f transcribe_ms)

  $out = & "$Target\release\examples\whisper_probe.exe" $link $Wav 2 2>$null
  $ms = @($out | ForEach-Object { if ($_ -match 'transcribe_ms=(\d+)') { [int]$matches[1] } })
  "probe #$i run1=$($ms[0]) run2=$($ms[1])"
  $cold += $ms[0]; $warm += $ms[1]
}
($out | Select-Object -Last 1) -replace '^run \d+: transcribe_ms=\d+ ', 'text '
"MEDIAN app transcribe_ms=$(Median $app) | probe run1=$(Median $cold) run2=$(Median $warm)"
