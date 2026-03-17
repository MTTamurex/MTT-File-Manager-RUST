@echo off
setlocal

set "SCRIPT_DIR=%~dp0"
start "MTT Diagnostic Console" powershell -NoExit -ExecutionPolicy Bypass -File "%SCRIPT_DIR%run_with_logs.ps1"
