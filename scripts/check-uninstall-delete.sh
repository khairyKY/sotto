#!/usr/bin/env bash
# Regression check for nsis/hooks.nsi's SOTTO_DELETE_ASSETS (#53): the
# uninstaller must delete only Sotto's own files from a (possibly shared)
# assets_dir. Runs the macro against two throwaway folders, never real data.
#   bash scripts/check-uninstall-delete.sh
set -euo pipefail
M="${MAKENSIS:-$LOCALAPPDATA/tauri/NSIS/makensis.exe}"
S="$(mktemp -d)"; trap 'rm -rf "$S"' EXIT
cp "$(dirname "$0")/../nsis/hooks.nsi" "$S/"
mk() { mkdir -p "$(dirname "$1")"; echo x > "$1"; }
for d in shared own; do
  mk "$S/$d/onnxruntime.dll"; mk "$S/$d/models/qwen2.5-1.5b-instruct-q4_k_m.gguf"
  mk "$S/$d/models/ggml-large-v3-turbo-q5_0.bin.part"
  mk "$S/$d/models/parakeet-tdt-0.6b-v3-int8/encoder-model.int8.onnx"
  mk "$S/$d/runtime/llama/llama-server.exe"
  mk "$S/$d/recordings/index.jsonl"; mk "$S/$d/recordings/1790000000.wav"
done
mk "$S/shared/models/other-app-model.bin"; mk "$S/shared/notes.txt"; mk "$S/shared/runtime/other/x.dll"
W="$(cygpath -w "$S")"
cat > "$S/t.nsi" <<NSI
Unicode true
!include "hooks.nsi"
OutFile "t.exe"
RequestExecutionLevel user
SilentInstall silent
Section
  StrCpy \$0 "$W\shared"
  !insertmacro SOTTO_DELETE_ASSETS
SectionEnd
Section
  StrCpy \$0 "$W\own"
  !insertmacro SOTTO_DELETE_ASSETS
SectionEnd
NSI
(cd "$S" && "$M" -V1 t.nsi) && powershell -NoProfile -Command "Start-Process '$W\t.exe' -Wait"
got="$(cd "$S/shared" && find . -type f | sort | tr '\n' ' ')"
want="./models/other-app-model.bin ./notes.txt ./runtime/other/x.dll "
[ "$got" = "$want" ] || { echo "FAIL shared: kept [$got], want [$want]"; exit 1; }
[ ! -e "$S/own" ] || { echo "FAIL own: folder should be gone"; exit 1; }
echo "PASS: only Sotto's files deleted; a Sotto-only folder removed"
