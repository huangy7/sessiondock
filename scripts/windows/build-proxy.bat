@echo off
setlocal EnableDelayedExpansion

rem ============================================================
rem Claudia — Build sessiondock-proxy for Windows
rem ============================================================
rem Usage:
rem   scripts\windows\build-proxy.bat [TARGET]
rem   TARGET: x86_64-pc-windows-msvc (default) or aarch64-pc-windows-msvc
rem ============================================================

set SCRIPT_DIR=%~dp0
set PROJECT_DIR=%SCRIPT_DIR%..\..
set BINARIES_DIR=%PROJECT_DIR%\src-tauri\binaries

set TARGET=%~1
if "%TARGET%"=="" (
    set DETECTED_ARCH=%PROCESSOR_ARCHITECTURE%
    if not "%PROCESSOR_ARCHITEW6432%"=="" set DETECTED_ARCH=%PROCESSOR_ARCHITEW6432%

    if /i "%DETECTED_ARCH%"=="ARM64" (
        set TARGET=aarch64-pc-windows-msvc
    ) else (
        set TARGET=x86_64-pc-windows-msvc
    )
)

if not exist "%BINARIES_DIR%" mkdir "%BINARIES_DIR%"

echo [proxy] Building sessiondock-proxy for %TARGET%...
pushd "%PROJECT_DIR%"
cargo build -p sessiondock-proxy --release --target %TARGET%
if errorlevel 1 (
    echo [ERROR] Build failed. 1>&2
    popd
    exit /b 1
)
popd

copy /y "%PROJECT_DIR%\target\%TARGET%\release\sessiondock-proxy.exe" ^
        "%BINARIES_DIR%\sessiondock-proxy-%TARGET%.exe" >nul
echo [proxy] Binary placed at binaries\sessiondock-proxy-%TARGET%.exe
exit /b 0
