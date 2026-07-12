@echo off
:: Sotto startup script
:: This launches Sotto as a detached process so it runs in the background.
:: The app starts minimized to the system tray by default (check the '^' overflow menu near your clock).

set BIN_DIR=D:\Coding\sotto-opencode\target

if exist "%BIN_DIR%\release\sotto.exe" (
    echo Launching Sotto (Release build)...
    start "" "%BIN_DIR%\release\sotto.exe"
) else if exist "%BIN_DIR%\debug\sotto.exe" (
    echo Launching Sotto (Debug build)...
    start "" "%BIN_DIR%\debug\sotto.exe"
) else (
    echo Sotto executable not found! Please build it first.
    pause
)
