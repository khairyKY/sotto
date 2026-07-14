; Sotto — NSIS installer/uninstaller hooks.
;
; The installer itself only ships the app (~4 MB). Voice models + runtime
; (~1.8 GB) live in %APPDATA%\sotto — they are downloaded once on first launch
; and reused across every future update. On uninstall we therefore have to
; decide separately what to do with them: leave them (so re-install is fast) or
; remove them (so uninstall is truly complete).

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

; Post-uninstall: prompt whether to also delete the ~1.8 GB of voice models +
; user settings. Default = No, so a click-through uninstall keeps the models
; for a future reinstall. The launch-at-login Run key is already removed by
; Tauri's own uninstall stanza (matches value name "Sotto" written by startup.rs).
!macro NSIS_HOOK_POSTUNINSTALL
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 \
    "Also remove Sotto's voice models and settings?$\r$\n$\r$\nThis frees ~1.8 GB. Choose No if you plan to reinstall Sotto later — models will be reused." \
    /SD IDNO IDNO skip_data_wipe

    ; $APPDATA on Windows == %APPDATA% == C:\Users\<name>\AppData\Roaming
    RMDir /r "$APPDATA\sotto"
    ; Legacy dev location — safe no-op on end-user machines that don't have it.
    RMDir /r "D:\sotto"

  skip_data_wipe:
!macroend
