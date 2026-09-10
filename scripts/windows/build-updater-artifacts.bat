@echo off
setlocal EnableDelayedExpansion

rem ============================================================
rem Claudia — Build Updater Artifacts (Windows)
rem ============================================================
rem This script mimics the macOS version: it builds the app,
rem verifies that the executable and signature are generated,
rem and prints out the files to upload.

set SCRIPT_DIR=%~dp0
set PROJECT_ROOT=%SCRIPT_DIR%..\..

echo [INFO] Building Windows x64 (x86_64-pc-windows-msvc)...
call "%SCRIPT_DIR%build-and-sign.bat" x64
if errorlevel 1 (
    echo [ERROR] Build failed. 1>&2
    exit /b 1
)

set BUNDLE_DIR=%PROJECT_ROOT%\target\x86_64-pc-windows-msvc\release\bundle\nsis

set EXE=
set SIG=
for %%f in ("%BUNDLE_DIR%\SessionDock_*.exe") do set EXE=%%f
for %%f in ("%BUNDLE_DIR%\SessionDock_*.exe.sig") do set SIG=%%f

if "!EXE!"=="" (
    echo [ERROR] Missing artifact: .exe not found in %BUNDLE_DIR% 1>&2
    exit /b 1
)
if "!SIG!"=="" (
    echo [ERROR] Missing artifact: .exe.sig not found in %BUNDLE_DIR% 1>&2
    exit /b 1
)

echo [OK] !EXE!
echo [OK] !SIG!

echo.
echo [INFO] All Windows updater artifacts ready:
echo   !EXE!
echo   !SIG!

exit /b 0
