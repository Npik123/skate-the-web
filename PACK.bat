@echo off
setlocal
cd /d "%~dp0"
node "%~dp0scripts\make-game-data-pack.js" %*
if errorlevel 1 pause
