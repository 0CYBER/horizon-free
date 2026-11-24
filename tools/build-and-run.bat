@echo off
setlocal EnableExtensions DisableDelayedExpansion

pushd "%~dp0.." || exit /b 1
set "checkout=%CD%"
set "version="
for /f "usebackq delims=" %%V in (`python.exe -c "import json; print(json.load(open('research-lock.json', encoding='utf-8'))['minecraft']['version'])"`) do set "version=%%V"
if not defined version (
    popd
    exit /b 1
)
cargo.exe build --locked --profile fast -p horizon-game --bin horizon-client --target-dir "%checkout%\target"
set "status=%errorlevel%"
if not "%status%"=="0" (
    popd
    exit /b %status%
)

rem Keep the checkout pushed until the game exits: for a UNC checkout, pushd maps a
rem temporary drive that %checkout% points at, and popd would remove it.
set "scratch=%TEMP%\horizon-dev-%RANDOM%-%RANDOM%"
mkdir "%scratch%\game" "%scratch%\runtime" || (
    popd
    exit /b 1
)
echo Development run: "%scratch%"
set "HORIZON_HOME=%scratch%\home"
set "HORIZON_LAUNCHER_HOME=%scratch%\launcher"
set "HORIZON_LAUNCHER_LOCATION_FILE=%scratch%\launcher-location.json"
set "HORIZON_LAUNCHER_TRASH=%scratch%\trash"
set "HORIZON_LAUNCHER_CLIPBOARD=%scratch%\clipboard"
set "HORIZON_LAUNCHER_DESKTOP=%scratch%\desktop"
set "HORIZON_LAUNCHER_NO_SELF_UPDATE=1"
set "HORIZON_NO_AUDIO=1"
set "HORIZON_TELEMETRY_URL=off"
set "HORIZON_SESSION_FILE="
set "XDG_DATA_HOME=%scratch%\data"
set "XDG_CONFIG_HOME=%scratch%\config"
set "XDG_CACHE_HOME=%scratch%\cache"
set "XDG_RUNTIME_DIR=%scratch%\runtime"

pushd "%scratch%\game" || (
    popd
    exit /b 1
)
"%checkout%\target\fast\horizon-client.exe" --resources "%checkout%\.local\horizon-client\%version%" --game-dir "%scratch%\game" %*
set "status=%errorlevel%"
popd
popd
exit /b %status%
