@echo off
chcp 65001 >nul
start "" powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -WindowStyle Hidden -File "%~dp0DualSenseBattery.ps1"
