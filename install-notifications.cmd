@echo off
rem Double-click to let Islo show notifications (runs install-notifications.ps1).
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0install-notifications.ps1" %*
