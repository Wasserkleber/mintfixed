@echo off
rem One-command Windows launcher for release and experimental builds.
rem Use PowerShell with a process-only policy override (no machine setting changed).
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\build_release_and_experimental.ps1" %*
exit /b %ERRORLEVEL%
