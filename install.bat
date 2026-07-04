@echo off
setlocal enableextensions enabledelayedexpansion

:: install.bat -- installs or updates osu-collect from the latest GitHub release
:: requires: Windows 10+ (PowerShell 5.1+ and curl.exe built-in, both preinstalled)

set "REPO=uwuclxdy/osu-collect"
set "API_URL=https://api.github.com/repos/%REPO%/releases/latest"
set "ASSET_NAME=osu-collect-windows-x64.exe"
set "INSTALL_DIR=%LOCALAPPDATA%\Programs\osu-collect"
set "INSTALL_BIN=%INSTALL_DIR%\osu-collect.exe"
set "TMPDIR=%TEMP%\osu-collect-install-%RANDOM%"

:: -- fetch release metadata (one request, with retry) ------------------------

echo ==^> fetching latest release info...

:: fetch once and parse locally -- curl --retry rides out the transient 5xx /
:: timeout the GitHub API occasionally returns; three separate live calls did not
set "RELJSON=%TEMP%\osu-collect-release-%RANDOM%.json"
curl.exe -fsSL --retry 5 -A "osu-collect-installer" -o "%RELJSON%" "%API_URL%"
if errorlevel 1 (
  echo error: could not reach the GitHub API ^(try again in a minute^)
  del /f /q "%RELJSON%" 2>nul
  exit /b 1
)

:: NULL sentinels keep all three fields non-empty so for /f does not collapse the
:: "|" delimiters when an asset is missing. The sha256 comes from GitHub's inline
:: per-asset "digest" ("sha256:<hex>"), so no separate checksum request is needed.
for /f "tokens=1,2,3 delims=|" %%a in ('powershell -NoProfile -Command ^
  "$r = Get-Content -LiteralPath '%RELJSON%' -Raw | ConvertFrom-Json; " ^
  "$t = $r.tag_name; if (-not $t) { $t = 'NULL' }; " ^
  "$a = $r.assets | Where-Object { $_.name -eq '%ASSET_NAME%' }; " ^
  "$d = $a.browser_download_url; if (-not $d) { $d = 'NULL' }; " ^
  "$h = $a.digest; if ($h -match '^sha256:[0-9a-fA-F]{64}$') { $h = $h.Substring(7).ToLower() } else { $h = 'NULL' }; " ^
  "Write-Output ($t + '|' + $d + '|' + $h)"') do (
  set "LATEST_TAG=%%a"
  set "DOWNLOAD_URL=%%b"
  set "REMOTE_HASH=%%c"
)
del /f /q "%RELJSON%" 2>nul

if "%LATEST_TAG%"=="" (
  echo error: could not read release info
  exit /b 1
)
if "%LATEST_TAG%"=="NULL" (
  echo error: latest release has no tag
  exit /b 1
)
if "%DOWNLOAD_URL%"=="NULL" (
  echo error: asset '%ASSET_NAME%' not found in release %LATEST_TAG%
  exit /b 1
)
if "%REMOTE_HASH%"=="NULL" (
  echo error: asset '%ASSET_NAME%' has no sha256 digest in release %LATEST_TAG%
  exit /b 1
)

echo ==^> latest release: %LATEST_TAG%

:: -- create temp dir ----------------------------------------------------------

mkdir "%TMPDIR%" 2>nul
if errorlevel 1 (
  echo error: could not create temp directory %TMPDIR%
  exit /b 1
)

set "TMP_BIN=%TMPDIR%\%ASSET_NAME%"

:: REMOTE_HASH already came from the release JSON's per-asset digest above.

:: -- idempotency check --------------------------------------------------------

set "CURRENT_HASH="
if exist "%INSTALL_BIN%" (
  for /f "delims=" %%H in ('powershell -NoProfile -Command ^
    "(Get-FileHash \"%INSTALL_BIN%\" -Algorithm SHA256).Hash.ToLower()"') do (
    set "CURRENT_HASH=%%H"
  )
)

if defined CURRENT_HASH (
  set "REMOTE_LOWER="
  for /f "delims=" %%H in ('powershell -NoProfile -Command ^
    "'%REMOTE_HASH%'.ToLower()"') do set "REMOTE_LOWER=%%H"

  if "!CURRENT_HASH!"=="!REMOTE_LOWER!" (
    echo ==^> already up to date (%LATEST_TAG%^)
    rmdir /s /q "%TMPDIR%" 2>nul
    goto :shortcuts
  )
)

:: -- download binary ----------------------------------------------------------

echo ==^> downloading osu-collect %LATEST_TAG%...
curl.exe -fsSL --retry 3 -o "%TMP_BIN%" "%DOWNLOAD_URL%"
if errorlevel 1 (
  echo error: download failed
  rmdir /s /q "%TMPDIR%" 2>nul
  exit /b 1
)

:: -- verify sha256 ------------------------------------------------------------

echo ==^> verifying checksum...
for /f "delims=" %%H in ('powershell -NoProfile -Command ^
  "(Get-FileHash \"%TMP_BIN%\" -Algorithm SHA256).Hash.ToLower()"') do (
  set "ACTUAL_HASH=%%H"
)

set "REMOTE_LOWER="
for /f "delims=" %%H in ('powershell -NoProfile -Command ^
  "'%REMOTE_HASH%'.ToLower()"') do set "REMOTE_LOWER=%%H"

if not "!ACTUAL_HASH!"=="!REMOTE_LOWER!" (
  echo error: sha256 mismatch
  echo   expected: !REMOTE_LOWER!
  echo   actual:   !ACTUAL_HASH!
  del /f /q "%TMP_BIN%" 2>nul
  rmdir /s /q "%TMPDIR%" 2>nul
  exit /b 1
)

:: -- install binary -----------------------------------------------------------

if not exist "%INSTALL_DIR%" (
  mkdir "%INSTALL_DIR%"
  if errorlevel 1 (
    echo error: could not create install directory %INSTALL_DIR%
    rmdir /s /q "%TMPDIR%" 2>nul
    exit /b 1
  )
)

copy /y "%TMP_BIN%" "%INSTALL_BIN%" >nul
if errorlevel 1 (
  echo error: could not copy binary to %INSTALL_BIN%
  rmdir /s /q "%TMPDIR%" 2>nul
  exit /b 1
)
echo ==^> installed to %INSTALL_BIN%

:: -- add to user PATH if needed -----------------------------------------------

:: read current user PATH from registry (not the process PATH) to avoid duplicates
for /f "tokens=2*" %%A in (
  'reg query "HKCU\Environment" /v Path 2^>nul'
) do set "USER_PATH=%%B"

:: check if install dir is already in user path
echo !USER_PATH! | findstr /i /c:"%INSTALL_DIR%" >nul 2>&1
if errorlevel 1 (
  if defined USER_PATH (
    setx PATH "!USER_PATH!;%INSTALL_DIR%" >nul
  ) else (
    setx PATH "%INSTALL_DIR%" >nul
  )
  if errorlevel 1 (
    echo error: could not update user PATH
  ) else (
    echo ==^> added %INSTALL_DIR% to user PATH
    echo     restart your terminal for PATH changes to take effect
  )
) else (
  echo ==^> %INSTALL_DIR% already in user PATH
)

:: -- shortcuts ----------------------------------------------------------------

:shortcuts
echo ==^> creating shortcuts...
:: if Windows Terminal is present, launch through it so the app opens in wt instead
:: of the legacy console; the wt.exe alias is a 0-byte stub with no icon, so resolve
:: the real WindowsTerminal.exe from the package and set it as the shortcut icon
powershell -NoProfile -Command ^
  "$ws = New-Object -ComObject WScript.Shell; " ^
  "$q = [char]34; " ^
  "$bang = [char]33; " ^
  "$name = 'osu' + $bang + 'collect.lnk'; " ^
  "$wt = [System.IO.Path]::Combine($env:LOCALAPPDATA, 'Microsoft\WindowsApps\wt.exe'); " ^
  "$useWt = Test-Path $wt; " ^
  "$icon = ''; " ^
  "$pkg = Get-AppxPackage -Name Microsoft.WindowsTerminal | Select-Object -First 1; " ^
  "if ($pkg) { $wtExe = Join-Path $pkg.InstallLocation 'WindowsTerminal.exe'; if (Test-Path $wtExe) { $icon = $wtExe + ',0'; } } " ^
  "$paths = @( " ^
  "  [System.Environment]::GetFolderPath('Desktop'), " ^
  "  [System.IO.Path]::Combine($env:APPDATA, 'Microsoft\Windows\Start Menu\Programs') " ^
  "); " ^
  "foreach ($dir in $paths) { " ^
  "  Remove-Item -LiteralPath ([System.IO.Path]::Combine($dir, 'osu-collect.lnk')) -Force -ErrorAction SilentlyContinue; " ^
  "  $lnk = $ws.CreateShortcut([System.IO.Path]::Combine($dir, $name)); " ^
  "  if ($useWt) { " ^
  "    $lnk.TargetPath = $wt; " ^
  "    $lnk.Arguments = $q + '%INSTALL_BIN%' + $q; " ^
  "  } else { " ^
  "    $lnk.TargetPath = '%INSTALL_BIN%'; " ^
  "  } " ^
  "  if ($icon) { $lnk.IconLocation = $icon; } " ^
  "  $lnk.WorkingDirectory = $env:USERPROFILE; " ^
  "  $lnk.Description = 'download osu' + $bang + ' collections (TUI)'; " ^
  "  $lnk.Save() " ^
  "}"
if errorlevel 1 (
  echo error: could not create shortcuts
) else (
  echo ==^> shortcuts created in Desktop and Start Menu
)

:: -- register uninstaller -----------------------------------------------------

echo ==^> registering uninstaller...

:: 1. extract the uninstaller payload (plain PowerShell at the bottom of this file)
::    into the install dir. kept separate from the registry writes below so a
::    throw here cannot abort registration
powershell -NoProfile -Command ^
  "try { " ^
  "  $marker = '# ::OSU-COLLECT' + '-UNINSTALLER::'; " ^
  "  $txt = Get-Content -LiteralPath '%~f0' -Raw; " ^
  "  $i = $txt.IndexOf($marker); " ^
  "  if ($i -ge 0) { Set-Content -LiteralPath '%INSTALL_DIR%\uninstall.ps1' -Value $txt.Substring($i + $marker.Length).TrimStart() -Encoding ASCII; } " ^
  "} catch { exit 1 }"

if not exist "%INSTALL_DIR%\uninstall.ps1" (
  echo error: could not write uninstall.ps1 -- skipping uninstaller registration
  goto :after_register
)

:: 2. register under HKCU with reg.exe (native, reliable). delayed expansion is
::    off in this block so the literal "!" in the display name is not stripped
setlocal disabledelayedexpansion
set "UNKEY=HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\osu-collect"
set "UNVER=%LATEST_TAG%"
if "%UNVER:~0,1%"=="v" set "UNVER=%UNVER:~1%"
reg add "%UNKEY%" /v DisplayName          /t REG_SZ   /d "osu!collect" /f >nul
reg add "%UNKEY%" /v DisplayVersion        /t REG_SZ   /d "%UNVER%" /f >nul
reg add "%UNKEY%" /v DisplayIcon           /t REG_SZ   /d "%INSTALL_BIN%" /f >nul
reg add "%UNKEY%" /v Publisher             /t REG_SZ   /d "uwuclxdy" /f >nul
reg add "%UNKEY%" /v InstallLocation       /t REG_SZ   /d "%INSTALL_DIR%" /f >nul
reg add "%UNKEY%" /v URLInfoAbout          /t REG_SZ   /d "https://github.com/uwuclxdy/osu-collect" /f >nul
reg add "%UNKEY%" /v UninstallString       /t REG_SZ   /d "powershell -NoProfile -ExecutionPolicy Bypass -File \"%INSTALL_DIR%\uninstall.ps1\"" /f >nul
reg add "%UNKEY%" /v QuietUninstallString  /t REG_SZ   /d "powershell -NoProfile -ExecutionPolicy Bypass -File \"%INSTALL_DIR%\uninstall.ps1\"" /f >nul
reg add "%UNKEY%" /v NoModify              /t REG_DWORD /d 1 /f >nul
reg add "%UNKEY%" /v NoRepair              /t REG_DWORD /d 1 /f >nul
endlocal

reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\osu-collect" /v UninstallString >nul 2>&1
if errorlevel 1 (
  echo error: could not register uninstaller in the Installed apps list
) else (
  echo ==^> uninstaller registered; see Settings - Apps - Installed apps
)

:after_register

:: -- cleanup ------------------------------------------------------------------

rmdir /s /q "%TMPDIR%" 2>nul
echo ==^> done -- open a new terminal and run 'osu-collect' to start
endlocal
exit /b 0

:: Everything below runs only when extracted to uninstall.ps1 (PowerShell), never
:: by cmd -- it sits past `exit /b 0`. The installer copies it verbatim into the
:: install dir and registers it as the Windows uninstaller.
# ::OSU-COLLECT-UNINSTALLER::
$ErrorActionPreference = 'SilentlyContinue'
Write-Host ''
Write-Host 'Uninstalling osu!collect ...'
$installDir = Join-Path $env:LOCALAPPDATA 'Programs\osu-collect'
$desktop    = [Environment]::GetFolderPath('Desktop')
$startMenu  = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$shortcuts  = @(
    Join-Path $desktop   'osu!collect.lnk'
    Join-Path $startMenu 'osu!collect.lnk'
    Join-Path $desktop   'osu-collect.lnk'
    Join-Path $startMenu 'osu-collect.lnk'
)
foreach ($lnk in $shortcuts) { Remove-Item -LiteralPath $lnk -Force }

# strip the install dir from the user PATH
$userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if ($userPath) {
    $kept = ($userPath -split ';') | Where-Object { $_ -and ($_ -ne $installDir) }
    [Environment]::SetEnvironmentVariable('Path', ($kept -join ';'), 'User')
}

# drop the "Installed apps" registry entry
Remove-Item -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\osu-collect' -Recurse -Force

Write-Host 'osu!collect has been uninstalled.'
Start-Sleep -Seconds 2

# remove the install dir (incl. this script) from a detached process so the file
# isn't locked while it deletes itself; the delay lets this script exit first
Start-Process cmd -WindowStyle Hidden -ArgumentList '/c', ('timeout /t 3 > nul & rmdir /s /q "' + $installDir + '"')
