@echo off
setlocal EnableDelayedExpansion

rem ============================================================
rem Claudia — Build & Sign Script (Windows)
rem ============================================================
rem Usage:
rem   scripts\windows\build-and-sign.bat              # default: x64
rem   scripts\windows\build-and-sign.bat x64
rem   scripts\windows\build-and-sign.bat arm64
rem
rem Prerequisites (code signing, optional):
rem   Set environment variables before running:
rem     WINDOWS_CERTIFICATE_THUMBPRINT  — SHA-1 thumbprint of your cert
rem     WINDOWS_SIGN_TOOL               — path to signtool.exe (auto-detected if not set)
rem   If neither is set the script builds without signing.
rem ============================================================

set SCRIPT_DIR=%~dp0
set PROJECT_DIR=%SCRIPT_DIR%..\..
set BINARIES_DIR=%PROJECT_DIR%\src-tauri\binaries

rem ---- Check required env vars ----
if not defined TAURI_SIGNING_PRIVATE_KEY set TAURI_SIGNING_PRIVATE_KEY=
if not defined TAURI_SIGNING_PRIVATE_KEY_PASSWORD set TAURI_SIGNING_PRIVATE_KEY_PASSWORD=

rem ---- Parse arch argument ----
set ARCH=%~1
if "%ARCH%"=="" set ARCH=x64

if /i "%ARCH%"=="x64" (
    set TARGET=x86_64-pc-windows-msvc
) else if /i "%ARCH%"=="x86_64" (
    set TARGET=x86_64-pc-windows-msvc
) else if /i "%ARCH%"=="amd64" (
    set TARGET=x86_64-pc-windows-msvc
) else if /i "%ARCH%"=="arm64" (
    set TARGET=aarch64-pc-windows-msvc
) else if /i "%ARCH%"=="aarch64" (
    set TARGET=aarch64-pc-windows-msvc
) else (
    echo [ERROR] Unknown architecture: %ARCH%. Use: x64 or arm64 1>&2
    exit /b 1
)

set BUNDLE_DIR=%PROJECT_DIR%\target\%TARGET%\release\bundle

echo [INFO]  === Claudia Build + Sign (Windows) ===
echo [INFO]  Target: %TARGET%

rem ---- Detect signtool if not specified ----
if "%WINDOWS_SIGN_TOOL%"=="" (
    for /f "delims=" %%i in ('where signtool 2^>nul') do set WINDOWS_SIGN_TOOL=%%i
)

rem ---- Step 0: Clean old bundle cache ----
echo [INFO]  Cleaning old bundle cache for %TARGET%...
if exist "%BUNDLE_DIR%" rmdir /s /q "%BUNDLE_DIR%"
echo [INFO]  Bundle cache cleaned.

rem ---- Step 1: Build claudia-proxy ----
echo [INFO]  Building claudia-proxy for %TARGET%...
if not exist "%BINARIES_DIR%" mkdir "%BINARIES_DIR%"
pushd "%PROJECT_DIR%"
cargo build -p claudia-proxy --release --target %TARGET%
if errorlevel 1 (
    echo [ERROR] claudia-proxy build failed. 1>&2
    popd
    exit /b 1
)
popd
copy /y "%PROJECT_DIR%\target\%TARGET%\release\claudia-proxy.exe" ^
        "%BINARIES_DIR%\claudia-proxy-%TARGET%.exe" >nul
echo [INFO]  claudia-proxy binary placed at binaries\claudia-proxy-%TARGET%.exe

rem ---- Step 2: Build Tauri app ----
echo [INFO]  Building Claudia with Tauri...
pushd "%PROJECT_DIR%"
set CLAUDIA_SKIP_PROXY_BUILD=1
npm run tauri build -- --target %TARGET%
if errorlevel 1 (
    echo [ERROR] Tauri build failed. 1>&2
    popd
    exit /b 1
)
echo [INFO]  Build completed.
popd

rem ---- Step 3: Sign (optional) ----
if not "%WINDOWS_CERTIFICATE_THUMBPRINT%"=="" (
    echo [INFO]  Signing installer with certificate: %WINDOWS_CERTIFICATE_THUMBPRINT%
    if "%WINDOWS_SIGN_TOOL%"=="" (
        echo [ERROR] signtool.exe not found. Install Windows SDK or set WINDOWS_SIGN_TOOL. 1>&2
        exit /b 1
    )
    set INSTALLER=
    for %%f in ("%BUNDLE_DIR%\nsis\Claudia_*.exe") do set INSTALLER=%%f
    if "!INSTALLER!"=="" (
        for %%f in ("%BUNDLE_DIR%\msi\Claudia_*.msi") do set INSTALLER=%%f
    )
    if "!INSTALLER!"=="" (
        echo [ERROR] Installer not found — cannot sign. 1>&2
        exit /b 1
    )
    echo [INFO]  Signing: !INSTALLER!
    "%WINDOWS_SIGN_TOOL%" sign /sha1 "%WINDOWS_CERTIFICATE_THUMBPRINT%" ^
        /fd SHA256 /tr http://timestamp.digicert.com /td SHA256 ^
        "!INSTALLER!"
    if errorlevel 1 (
        echo [ERROR] Signing failed. 1>&2
        exit /b 1
    )
    echo [INFO]  Signed successfully.
) else (
    echo [INFO]  WINDOWS_CERTIFICATE_THUMBPRINT not set — skipping signing.
)

rem ---- Step 4: Find installer ----
set INSTALLER=
for %%f in ("%BUNDLE_DIR%\nsis\Claudia_*.exe") do set INSTALLER=%%f
if "%INSTALLER%"=="" (
    for %%f in ("%BUNDLE_DIR%\msi\Claudia_*.msi") do set INSTALLER=%%f
)
if "%INSTALLER%"=="" (
    echo [ERROR] Installer not found in %BUNDLE_DIR% 1>&2
    exit /b 1
)
echo [INFO]  Installer: %INSTALLER%

echo [INFO]  === Done! ===
exit /b 0
