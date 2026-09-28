; Sotto — NSIS installer/uninstaller hooks.
;
; The installer itself only ships the app (~4 MB). Voice models + runtime
; (~2.8 GB) live in %APPDATA%\sotto by default — they are downloaded once on
; first launch and reused across every future update. On uninstall we
; therefore have to decide separately what to do with them: leave them (so
; re-install is fast) or remove them (so uninstall is truly complete).

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

; Terminate any running Sotto so the .exe isn't locked while files are removed.
; Uses taskkill (present on every supported Windows) rather than pulling in an
; NSIS plugin. Missing / not-running is fine — /F just returns nonzero.
!macro NSIS_HOOK_PREUNINSTALL
  nsExec::ExecToLog 'taskkill /F /IM sotto.exe /T'
  ; Also kill the LLM sidecar in case it's still resident (idle-killer clears
  ; it after 5 min normally; this covers a mid-session uninstall).
  nsExec::ExecToLog 'taskkill /F /IM llama-server.exe /T'
!macroend

; Post-uninstall: prompt whether to also delete the ~2.8 GB of voice models +
; user settings. Default = No, so a click-through uninstall keeps the models
; for a future reinstall. The launch-at-login Run key is already removed by
; Tauri's own uninstall stanza (matches value name "Sotto" written by startup.rs).
;
; A custom `assets_dir` (config.toml, e.g. "E:\sotto-models") is NOT read here
; — NSIS has no TOML parser and pulling one in for one field isn't worth it.
; Instead `config::write_assets_dir_marker()` (src/config.rs) drops the
; resolved path into "$APPDATA\sotto\assets_dir.txt" on every startup; we just
; read that plain-text file with NSIS's built-in FileOpen/FileRead. #53 was a
; hardcoded "D:\sotto" here that deleted the wrong folder for anyone who
; followed the README's own example path — so before touching the marker's
; target we require it to (a) not be empty, (b) not be $APPDATA\sotto itself
; (already covered by the RMDir below), (c) not be a bare drive root or the
; user's profile root. Any check failing means we leave that folder alone.
; Past the checks we still delete only Sotto's own files by exact name and
; remove folders only if they end up empty (see below).
; Removes Sotto's own files from the assets folder in $0 (see POSTUNINSTALL).
; Its own macro so nsis-check can run it against a sandbox folder.
!macro SOTTO_DELETE_ASSETS
    ; Delete ONLY what Sotto itself put there, by exact name (src/assets.rs,
    ; recordings.rs), plus each file's download leftover. The user may have
    ; pointed assets_dir at a shared folder (say D:\AI) holding other apps'
    ; models; none of that may go.
    Delete "$0\onnxruntime.dll"
    Delete "$0\onnxruntime.dll.part"
    Delete "$0\models\qwen2.5-1.5b-instruct-q4_k_m.gguf"
    Delete "$0\models\qwen2.5-1.5b-instruct-q4_k_m.gguf.part"
    Delete "$0\models\ggml-large-v3-turbo-q5_0.bin"
    Delete "$0\models\ggml-large-v3-turbo-q5_0.bin.part"
    Delete "$0\models\ggml-egyptian-codeswitch-small.bin"
    Delete "$0\models\ggml-egyptian-codeswitch-small.bin.part"
    ; Sotto-named subfolders (zip extraction targets) are wholly Sotto's.
    RMDir /r "$0\models\parakeet-tdt-0.6b-v3-int8"
    RMDir /r "$0\runtime\llama"
    ; Kept recordings: only when Sotto's own index is there, and only the
    ; index + .wav files, never anything else in a "recordings" folder.
    IfFileExists "$0\recordings\index.jsonl" 0 no_recordings
      Delete "$0\recordings\*.wav"
      Delete "$0\recordings\index.jsonl"
    no_recordings:
    ; Folders go only if now empty: RMDir without /r refuses otherwise.
    RMDir "$0\recordings"
    RMDir "$0\models"
    RMDir "$0\runtime"
    RMDir "$0"
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 \
    "Also remove Sotto's voice models, settings, dictation history and kept recordings?$\r$\n$\r$\nThis frees ~2.8 GB. Choose No if you plan to reinstall Sotto later — models, settings and history will be reused." \
    /SD IDNO IDNO skip_data_wipe

    ; Read the assets_dir marker BEFORE deleting $APPDATA\sotto — that's
    ; where the marker file itself lives.
    StrCpy $0 ""
    IfFileExists "$APPDATA\sotto\assets_dir.txt" 0 no_marker
      FileOpen $1 "$APPDATA\sotto\assets_dir.txt" r
      FileRead $1 $0
      FileClose $1
      ; Trim a trailing newline FileRead may include.
      StrCpy $2 $0 1 -1
      StrCmp $2 "$\n" 0 +2
        StrCpy $0 $0 -1
      StrCpy $2 $0 1 -1
      StrCmp $2 "$\r" 0 +2
        StrCpy $0 $0 -1
    no_marker:

    ; $APPDATA on Windows == %APPDATA% == C:\Users\<name>\AppData\Roaming
    RMDir /r "$APPDATA\sotto"

    ; Nothing to relocate-delete: unset marker, or it just points at the
    ; folder we already removed above.
    StrCmp $0 "" skip_data_wipe
    StrCmp $0 "$APPDATA\sotto" skip_data_wipe

    ; Reject the user's whole profile root.
    StrCmp $0 "$PROFILE" skip_data_wipe

    ; Reject a bare drive root ("D:\" or "D:"): strip one trailing backslash
    ; if present, then a 2-char "X:" left over is a root, never a real
    ; assets folder.
    StrCpy $3 $0
    StrCpy $2 $3 1 -1
    StrCmp $2 "\" 0 +2
      StrCpy $3 $3 -1
    StrLen $2 $3
    StrCmp $2 2 0 not_drive_root
      StrCpy $2 $3 1 1
      StrCmp $2 ":" skip_data_wipe
    not_drive_root:

    !insertmacro SOTTO_DELETE_ASSETS

  skip_data_wipe:
!macroend
