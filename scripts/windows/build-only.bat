@echo off
setlocal EnableDelayedExpansion

rem ============================================================
rem Claudia — Build Only Script (Windows, no sign)
rem ============================================================
rem Usage:
rem   scripts\windows\build-only.bat              # default: x64
rem   scripts\windows\build-only.bat x64
rem   scripts\windows\build-only.bat arm64
rem   scripts\windows\build-only.bat --devtools   # enable DevTools (F12)
rem ============================================================

set SCRIPT_DIR=%~dp0
set PROJECT_DIR=%SCRIPT_DIR%..\..
set BINARIES_DIR=%PROJECT_DIR%\src-tauri\binaries
set CARGO_TOML=%PROJECT_DIR%\src-tauri\Cargo.toml

rem ---- Check required env vars ----
set TAURI_SIGNING_PRIVATE_KEY=dW50cnVzdGVkIGNvbW1lbnQ6IHJzaWduIGVuY3J5cHRlZCBzZWNyZXQga2V5ClJXUlRZMEl5Ymxvazh2a3I2QmxsNCtFOWRvaThWRU9ZS3pUVUU3TG4wWDFZWmN1bkoxZ0FBQkFBQUFBQUFBQUFBQUlBQUFBQUEyNTl5WXRxcmkwZGtZQ0ZPTS90ZjV2TUlURkxYUm5zcHVEV0dWNTA5cXcyK3FoeUZEQjlsWFl0T0U4Q1JGNHY4RlZOT0NnQnZPOXh2d0tWZHVxK2FFVHBWTm1FTktYaS9sRVZ3dWIwRE8ySzlCR1dEK3RtOC9JVHBPenFUTnJJVVNRc2VrQ05Eb0E9Cg==
set TAURI_SIGNING_PRIVATE_KEY_PASSWORD=claudia

rem ---- Parse arguments ----
set ARCH=x64
set DEVTOOLS=0
for %%A in (%*) do (
    if /i "%%A"=="--devtools" (
        set DEVTOOLS=1
    ) else (
        set ARCH=%%A
    )
)

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

echo [INFO]  === Claudia Build Only (no sign) ===
echo [INFO]  Target: %TARGET%
if "%DEVTOOLS%"=="1" echo [INFO]  DevTools: ENABLED

rem ---- Step 0: Patch Cargo.toml if --devtools ----
if "%DEVTOOLS%"=="1" (
    powershell -NoProfile -Command "$q=[char]34; (Get-Content '%CARGO_TOML%') -replace ('(tauri = \{[^}]*features = \[' + $q + ')'), ('$1devtools' + $q + ', ' + $q) | Set-Content '%CARGO_TOML%'"
    if errorlevel 1 (
        echo [ERROR] Failed to patch Cargo.toml 1>&2
        exit /b 1
    )
    echo [INFO]  devtools feature added to Cargo.toml
)

rem ---- Step 1: Clean old bundle cache ----
echo [INFO]  Cleaning old bundle cache for %TARGET%...
if exist "%BUNDLE_DIR%" rmdir /s /q "%BUNDLE_DIR%"
echo [INFO]  Bundle cache cleaned.

rem ---- Step 2: Build claudia-proxy ----
echo [INFO]  Building claudia-proxy for %TARGET%...
if not exist "%BINARIES_DIR%" mkdir "%BINARIES_DIR%"
pushd "%PROJECT_DIR%"
cargo build -p claudia-proxy --release --target %TARGET%
if errorlevel 1 (
    echo [ERROR] claudia-proxy build failed. 1>&2
    popd
    goto :restore
)
popd
copy /y "%PROJECT_DIR%\target\%TARGET%\release\claudia-proxy.exe" ^
        "%BINARIES_DIR%\claudia-proxy-%TARGET%.exe" >nul
echo [INFO]  claudia-proxy binary placed at binaries\claudia-proxy-%TARGET%.exe

rem ---- Step 3: Build Tauri app ----
echo [INFO]  Building Claudia with Tauri...
pushd "%PROJECT_DIR%"
set CLAUDIA_SKIP_PROXY_BUILD=1
npm run tauri build -- --target %TARGET%
if errorlevel 1 (
    echo [ERROR] Tauri build failed. 1>&2
    popd
    goto :restore
)
echo [INFO]  Build completed.
popd

rem ---- Step 4: Find installer ----
set INSTALLER=
for %%f in ("%BUNDLE_DIR%\nsis\Claudia_*.exe") do set INSTALLER=%%f
if "%INSTALLER%"=="" (
    for %%f in ("%BUNDLE_DIR%\msi\Claudia_*.msi") do set INSTALLER=%%f
)
if "%INSTALLER%"=="" (
    echo [ERROR] Installer not found in %BUNDLE_DIR% 1>&2
    goto :restore
)
echo [INFO]  Installer: %INSTALLER%

echo [INFO]  === Done! ===

:restore
rem ---- Restore Cargo.toml if --devtools was used ----
if "%DEVTOOLS%"=="1" (
    powershell -NoProfile -Command "$q=[char]34; (Get-Content '%CARGO_TOML%') -replace ($q + 'devtools' + $q + ', '), '' | Set-Content '%CARGO_TOML%'"
    echo [INFO]  devtools feature removed from Cargo.toml
)

if "%INSTALLER%"=="" exit /b 1
exit /b 0
