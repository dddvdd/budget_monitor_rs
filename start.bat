@echo off
title Budget Monitoring System
echo ==============================================
echo   Budget Monitoring System (PESO)
echo ==============================================
echo.
echo Starting web server at http://localhost:8080 ...
start "" http://localhost:8080
budget_monitor.exe --dev --bind 127.0.0.1:8080
if %ERRORLEVEL% NEQ 0 (
    echo.
    echo Server stopped with error.
    pause
)

