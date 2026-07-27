@echo off
REM ==================================================
REM   Xin Ji Shen Meeting Assistant - Quick Start
REM   One-click startup script for Tauri desktop mode
REM   This script contains NO Chinese characters
REM ==================================================

setlocal enabledelayedexpansion

REM Configuration
set "PORT=55556"
set "URL=http://127.0.0.1:%PORT%/"
set "FRONTEND_DIR=%~dp0frontend"
set "CARGO_BIN=%USERPROFILE%\.cargo\bin"

REM Redirect TEMP/TMP to D drive to avoid C drive disk space exhaustion
REM (C drive has limited space; link.exe and cargo need large temp space during Rust build)
set "TEMP_DIR=%~dp0.tmp"
if not exist "%TEMP_DIR%" mkdir "%TEMP_DIR%"
set "TEMP=%TEMP_DIR%"
set "TMP=%TEMP_DIR%"
echo [INFO] TEMP/TMP redirected to: %TEMP_DIR%

REM Set UTF-8 charset for MSVC compiler
REM (whisper.cpp and llama.cpp source files contain UTF-8 encoded characters like music symbols
REM  which MSVC misinterprets as GBK on Chinese Windows, causing error C3688/C2001)
set "CXXFLAGS=/utf-8"
set "CFLAGS=/utf-8"
echo [INFO] CXXFLAGS/CFLAGS set to /utf-8 for MSVC

REM Redirect CARGO_TARGET_DIR to D drive project folder
REM (C drive has limited space; Rust build artifacts can exceed 10GB.
REM  Using D drive project-local target dir avoids C drive disk full errors.)
set "CARGO_TARGET_DIR=%~dp0build-target"
if not exist "%CARGO_TARGET_DIR%" mkdir "%CARGO_TARGET_DIR%"
echo [INFO] CARGO_TARGET_DIR redirected to: %CARGO_TARGET_DIR%

REM Disable pnpm verify-deps-before-run to prevent SQLite store database errors
REM (pnpm 11+ auto-checks deps before running scripts via a SQLite store index;
REM  if index.db is corrupted/locked, "pnpm tauri:dev" fails with ERR_SQLITE_ERROR.
REM  Deps are already installed by the step below, so this check is unnecessary.)
set "PNPM_CONFIG_VERIFY_DEPS_BEFORE_RUN=false"
echo [INFO] PNPM verify-deps-before-run disabled.

echo ==================================================
echo   Xin Ji Shen Meeting Assistant - Quick Start
echo   Tauri Desktop Mode
echo ==================================================
echo.

REM Add cargo bin to PATH if not already there (for new sessions)
where cargo >nul 2>&1
if %ERRORLEVEL% neq 0 (
    if exist "%CARGO_BIN%\cargo.exe" (
        set "PATH=%CARGO_BIN%;!PATH!"
        echo [INFO] Added cargo bin to PATH for this session.
    )
)

REM Change to frontend directory
cd /d "%FRONTEND_DIR%"
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Frontend directory not found: %FRONTEND_DIR%
    pause
    exit /b 1
)

REM ===== Check Node.js =====
where node >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Node.js not found in PATH.
    echo Please install Node.js from https://nodejs.org/
    pause
    exit /b 1
)

REM ===== Check npm =====
where npm >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [ERROR] npm not found in PATH.
    echo Please reinstall Node.js from https://nodejs.org/
    pause
    exit /b 1
)

echo [INFO] Node.js and npm detected.

REM ===== Check pnpm; auto-install if missing =====
where pnpm >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [WARN] pnpm not found in PATH. Auto-installing pnpm via npm...
    echo [INFO] Running: npm install -g pnpm
    call npm install -g pnpm
    if !ERRORLEVEL! neq 0 (
        echo [ERROR] Failed to auto-install pnpm.
        echo Please install manually by running: npm install -g pnpm
        pause
        exit /b 1
    )
    echo [INFO] pnpm installed successfully.
    where pnpm >nul 2>&1
    if !ERRORLEVEL! neq 0 (
        for /f "delims=" %%i in ('npm config get prefix 2^>nul') do set "NPM_GLOBAL=%%i"
        if exist "!NPM_GLOBAL!\pnpm.cmd" (
            set "PATH=!NPM_GLOBAL!;!PATH!"
        ) else if exist "%APPDATA%\npm\pnpm.cmd" (
            set "PATH=%APPDATA%\npm;!PATH!"
        ) else (
            echo [ERROR] pnpm.cmd not found after installation.
            echo Please close this window and run start.bat again.
            pause
            exit /b 1
        )
    )
)
echo [INFO] pnpm detected.
echo.

REM ===== Check Rust toolchain =====
where cargo >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Rust cargo not found in PATH.
    echo Please install Rust from https://rustup.rs/
    echo Run: winget install --id Rustlang.Rustup
    pause
    exit /b 1
)

where rustc >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [ERROR] Rust rustc not found in PATH.
    echo Please install Rust from https://rustup.rs/
    pause
    exit /b 1
)

echo [INFO] Rust toolchain detected:
for /f "delims=" %%v in ('rustc --version 2^>nul') do echo        %%v
echo.

REM ===== Check libclang.dll (required by whisper-rs bindgen) =====
set "LIBCLANG_DIR=%USERPROFILE%\libclang"
if not exist "%LIBCLANG_DIR%\libclang.dll" (
    REM Try to auto-install via pip (libclang PyPI package ships libclang.dll)
    echo [WARN] libclang.dll not found at %LIBCLANG_DIR%
    echo [INFO] Attempting auto-install via pip...
    where pip >nul 2>&1
    if !ERRORLEVEL! neq 0 (
        echo [ERROR] pip not found. Cannot auto-install libclang.
        echo Please install libclang manually:
        echo   pip install libclang
        echo Then copy libclang.dll from Python site-packages\clang\native\ to %LIBCLANG_DIR%\
        pause
        exit /b 1
    )
    call pip install libclang
    if !ERRORLEVEL! neq 0 (
        echo [ERROR] Failed to install libclang via pip.
        pause
        exit /b 1
    )
    REM Locate libclang.dll in Python site-packages
    set "FOUND_LIBCLANG="
    for /f "delims=" %%p in ('pip show -f libclang 2^>nul ^| findstr /C:"Location:"') do set "SITE_PKG_LINE=%%p"
    if defined SITE_PKG_LINE (
        for /f "tokens=2 delims=:" %%a in ("!SITE_PKG_LINE!") do set "SITE_PKG=%%a"
        set "SITE_PKG=!SITE_PKG: =!"
        if exist "!SITE_PKG!\clang\native\libclang.dll" (
            if not exist "%LIBCLANG_DIR%" mkdir "%LIBCLANG_DIR%"
            copy /Y "!SITE_PKG!\clang\native\libclang.dll" "%LIBCLANG_DIR%\libclang.dll" >nul
            set "FOUND_LIBCLANG=1"
        )
    )
    if not defined FOUND_LIBCLANG (
        echo [ERROR] Could not locate libclang.dll after pip install.
        echo Please manually copy libclang.dll to %LIBCLANG_DIR%\
        pause
        exit /b 1
    )
    echo [INFO] libclang.dll installed to %LIBCLANG_DIR%
)

REM Set LIBCLANG_PATH for current session (bindgen requires this)
set "LIBCLANG_PATH=%LIBCLANG_DIR%"
echo [INFO] LIBCLANG_PATH set to: %LIBCLANG_PATH%
echo.

REM ===== Check cmake (required by whisper-rs-sys to build whisper.cpp) =====
where cmake >nul 2>&1
if %ERRORLEVEL% neq 0 (
    echo [WARN] cmake not found in PATH. Attempting to locate or auto-install...

    REM Try locating cmake.exe via python sysconfig (most reliable method)
    set "CMAKE_FOUND="
    where python >nul 2>&1
    if !ERRORLEVEL! equ 0 (
        for /f "delims=" %%d in ('python -c "import sysconfig; print(sysconfig.get_path('scripts'))" 2^>nul') do (
            set "PY_SCRIPTS_DIR=%%d"
            if exist "%%d\cmake.exe" (
                set "PATH=%%d;!PATH!"
                set "CMAKE_FOUND=1"
            )
        )
    )

    REM If still not found, auto-install via pip
    if not defined CMAKE_FOUND (
        where pip >nul 2>&1
        if !ERRORLEVEL! neq 0 (
            echo [ERROR] pip not found. Cannot auto-install cmake.
            echo Please install cmake from https://cmake.org/download/
            pause
            exit /b 1
        )
        echo [INFO] Auto-installing cmake via pip...
        call pip install cmake
        if !ERRORLEVEL! neq 0 (
            echo [ERROR] Failed to auto-install cmake.
            echo Please install cmake from https://cmake.org/download/
            pause
            exit /b 1
        )
        REM Re-locate via python sysconfig after install
        where python >nul 2>&1
        if !ERRORLEVEL! equ 0 (
            for /f "delims=" %%d in ('python -c "import sysconfig; print(sysconfig.get_path('scripts'))" 2^>nul') do (
                set "PY_SCRIPTS_DIR=%%d"
                if exist "%%d\cmake.exe" (
                    set "PATH=%%d;!PATH!"
                    set "CMAKE_FOUND=1"
                )
            )
        )
    )

    REM Final fallback: search common pip user-scripts locations
    if not defined CMAKE_FOUND (
        for /f "delims=" %%f in ('dir /b /s "%LOCALAPPDATA%\Python\*Scripts\cmake.exe" 2^>nul') do (
            set "CMAKE_DIR=%%~dpf"
            set "CMAKE_DIR=!CMAKE_DIR:~0,-1!"
            set "PATH=!CMAKE_DIR!;!PATH!"
            set "CMAKE_FOUND=1"
        )
    )

    if not defined CMAKE_FOUND (
        echo [ERROR] cmake.exe not found and could not be auto-installed.
        echo Please install cmake from https://cmake.org/download/
        pause
        exit /b 1
    )
    echo [INFO] cmake located and added to PATH for this session.
)
for /f "delims=" %%v in ('cmake --version 2^>nul ^| findstr /C:"cmake version"') do echo [INFO] %%v
echo.

REM ===== Check WebView2 runtime (required by Tauri on Windows) =====
REM Note: Avoid %ProgramFiles(x86)% directly in if-exist because parentheses
REM break cmd parsing inside if blocks. Resolve to a variable first.
set "WEBVIEW2_FOUND=0"
set "PF_X86=%ProgramFiles(x86)%"
set "PF=%ProgramFiles%"
if exist "%PF_X86%\Microsoft\EdgeWebView\Application" set "WEBVIEW2_FOUND=1"
if exist "%PF%\Microsoft\EdgeWebView\Application" set "WEBVIEW2_FOUND=1"
if "%WEBVIEW2_FOUND%"=="0" (
    echo [WARN] WebView2 runtime not found. Tauri requires WebView2 on Windows.
    echo [INFO] Download from: https://developer.microsoft.com/microsoft-edge/webview2/
    echo.
    choice /C YN /M "Continue anyway"
    if errorlevel 2 exit /b 1
)
echo.

REM ===== Install frontend dependencies if missing or broken =====
REM Check both node_modules existence AND a critical file (tauri.js)
REM pnpm uses absolute paths in its virtual store - if the project is moved,
REM node_modules may exist but contain broken symlinks/paths.
REM Note: avoid parentheses in echo text inside if-blocks; they break cmd parsing.
set "NEED_INSTALL=0"
if not exist "node_modules" (
    set "NEED_INSTALL=1"
    echo [INFO] First run detected. node_modules not found.
)
if exist "node_modules" if not exist "node_modules\@tauri-apps\cli\tauri.js" (
    set "NEED_INSTALL=1"
    echo [WARN] node_modules exists but @tauri-apps/cli is missing.
    echo [WARN] Dependencies may be broken - project was moved?
    echo [INFO] Cleaning up broken node_modules...
    rmdir /s /q "node_modules" 2>nul
)

if "!NEED_INSTALL!"=="1" (
    echo [INFO] Installing frontend dependencies...
    echo This may take a few minutes. Please wait...
    echo.
    call pnpm install
    if !ERRORLEVEL! neq 0 (
        echo [ERROR] Failed to install frontend dependencies.
        pause
        exit /b 1
    )
    echo.
    echo [INFO] Frontend dependencies installed successfully.
    echo.
)

REM ===== Clean stale .next cache ONLY when project path has changed =====
REM (Avoids unnecessary recompilation on every startup, which causes
REM  Tauri webview to open before Next.js finishes compiling = blank right panel)
set "PATH_MARKER=%~dp0.next\.project-path"
set "CURRENT_PATH=%~dp0"
set "NEED_CLEAN=0"
if not exist ".next" set "NEED_CLEAN=1"
if exist "%PATH_MARKER%" (
    for /f "delims=" %%p in ('type "%PATH_MARKER%" 2^>nul') do set "STORED_PATH=%%p"
    if /i "!STORED_PATH!" neq "!CURRENT_PATH!" set "NEED_CLEAN=1"
) else (
    set "NEED_CLEAN=1"
)
if "!NEED_CLEAN!"=="1" (
    echo [INFO] Cleaning .next cache ^(path changed or first run^)...
    rmdir /s /q ".next" 2>nul
    echo [INFO] .next cache cleaned.
) else (
    echo [INFO] .next cache is valid, skipping clean ^(faster startup^).
)
REM Save current path marker
echo !CURRENT_PATH!> "%PATH_MARKER%"

REM ===== Kill any existing meetily process (avoid WebView2 lock conflicts) =====
echo [INFO] Checking for leftover meetily.exe processes...
taskkill /F /IM meetily.exe >nul 2>&1
python -c "import subprocess;subprocess.run([chr(116)+chr(97)+chr(115)+chr(107)+chr(107)+chr(105)+chr(108)+chr(108),chr(47)+chr(70),chr(47)+chr(73)+chr(77),chr(26032)+chr(38469)+chr(23457)+chr(20250)+chr(35758)+chr(21161)+chr(25143)+chr(46)+chr(101)+chr(120)+chr(101)],capture_output=True)" >nul 2>&1
if !ERRORLEVEL! equ 0 (
    echo [INFO] Terminated leftover meetily.exe process.
    ping -n 3 127.0.0.1 >nul
) else (
    echo [INFO] No leftover meetily.exe process found.
)

REM ===== Kill leftover WebView2 zombie processes from previous meetily runs =====
REM When meetily crashes, msedgewebview2.exe child processes may remain alive
REM and lock SQLite -shm/-wal files, causing disk I/O error on next startup.
REM WARNING: this kills ALL msedgewebview2 processes. If you run other WebView2
REM apps simultaneously, comment this out.
taskkill /F /IM msedgewebview2.exe >nul 2>&1
if !ERRORLEVEL! equ 0 (
    echo [INFO] Terminated leftover msedgewebview2.exe processes.
    ping -n 3 127.0.0.1 >nul
)

REM ===== Kill any existing process on the port =====
echo [INFO] Checking port %PORT% for existing processes...
set "PORT_IN_USE="
for /f "tokens=5" %%a in ('netstat -aon ^| findstr ":%PORT%.*LISTENING" 2^>nul') do (
    set "PORT_IN_USE=%%a"
)
if defined PORT_IN_USE (
    echo [INFO] Found process !PORT_IN_USE! using port %PORT%. Terminating...
    taskkill /F /PID !PORT_IN_USE! >nul 2>&1
    ping -n 3 127.0.0.1 >nul
) else (
    echo [INFO] Port %PORT% is free.
)

REM ===== Clean stale SQLite journal files - prevent disk I/O error on startup =====
REM If meetily was killed unexpectedly, -shm/-wal files may be left behind and
REM cause disk I/O error code 4618 when SQLite tries to recover.
set "APP_DATA_DIR=%APPDATA%\com.meetily.ai"
if exist "!APP_DATA_DIR!\meeting_minutes.sqlite-shm" (
    echo [INFO] Cleaning stale SQLite shared-memory file...
    del /F /Q "!APP_DATA_DIR!\meeting_minutes.sqlite-shm" >nul 2>&1
)
if exist "!APP_DATA_DIR!\meeting_minutes.sqlite-wal" (
    echo [INFO] Cleaning stale SQLite write-ahead-log file...
    del /F /Q "!APP_DATA_DIR!\meeting_minutes.sqlite-wal" >nul 2>&1
)
echo.

REM ===== Disable Rust incremental compilation to prevent rustc ICE =====
REM Incremental compilation cache can become corrupted when a build is interrupted,
REM causing rustc to panic with "internal compiler error: no entry found for key".
REM Disabling incremental compilation prevents this entirely and saves ~1GB disk space.
REM The trade-off is slightly slower builds ~30-60s, but much more reliable.
set "CARGO_INCREMENTAL=0"
echo [INFO] Rust incremental compilation disabled for reliability.

REM ===== Check for first-time Rust compilation =====
REM CARGO_TARGET_DIR is set to D drive project folder above
set "FIRST_BUILD=1"
if exist "%CARGO_TARGET_DIR%\debug\meetily.exe" set "FIRST_BUILD=0"

REM ===== Build llama-helper sidecar (required by Tauri) =====
set "HELPER_DIR=%~dp0llama-helper"
if not exist "%HELPER_DIR%\Cargo.toml" (
    echo [ERROR] llama-helper directory not found: %HELPER_DIR%
    pause
    exit /b 1
)

REM Detect target triple (e.g. x86_64-pc-windows-msvc)
for /f "tokens=2" %%i in ('rustc -vV 2^>nul ^| findstr "host:"') do set "TARGET_TRIPLE=%%i"
set "SIDECAR_BIN=%FRONTEND_DIR%\src-tauri\binaries\llama-helper-!TARGET_TRIPLE!.exe"

if exist "%SIDECAR_BIN%" (
    echo [INFO] llama-helper sidecar already exists, skipping build.
) else (
    echo [INFO] Building llama-helper sidecar ^(first time only^)...
    echo [INFO] This compiles llama.cpp and may take several minutes.
    pushd "%HELPER_DIR%"
    call cargo build
    if !ERRORLEVEL! neq 0 (
        echo [ERROR] Failed to build llama-helper.
        popd
        pause
        exit /b 1
    )
    popd

    REM Copy binary to sidecar location (CARGO_TARGET_DIR is on D drive)
    set "BINARIES_DIR=%FRONTEND_DIR%\src-tauri\binaries"
    if not exist "!BINARIES_DIR!" mkdir "!BINARIES_DIR!"
    set "SRC_BIN=%CARGO_TARGET_DIR%\debug\llama-helper.exe"
    if exist "!SRC_BIN!" (
        copy /Y "!SRC_BIN!" "!SIDECAR_BIN!" >nul
        echo [INFO] llama-helper sidecar copied to: !SIDECAR_BIN!
    ) else (
        echo [ERROR] llama-helper binary not found at: !SRC_BIN!
        echo [INFO] Check if cargo workspace target dir is elsewhere.
        pause
        exit /b 1
    )
)
echo.

REM ===== Start Tauri dev mode =====
echo ==================================================
if "%FIRST_BUILD%"=="1" (
    echo [INFO] Starting Tauri desktop app in dev mode.
    echo [WARN] FIRST-TIME BUILD DETECTED.
    echo [WARN] Compiling Rust backend may take 10-20 minutes.
    echo [WARN] Subsequent starts will be much faster.
    echo.
    echo [INFO] A Tauri window will open automatically when ready.
    echo [INFO] The Next.js dev server runs on %URL%
    echo.
    echo [INFO] Build logs are shown below. Please be patient...
) else (
    echo [INFO] Starting Tauri desktop app in dev mode.
    echo [INFO] A Tauri window will open automatically when ready.
    echo [INFO] The Next.js dev server runs on %URL%
)
echo ==================================================
echo.

REM ===== Pre-start Next.js dev server and wait for compilation =====
REM Tauri's beforeDevCommand starts Next.js but doesn't wait for compilation
REM to finish before opening the webview, causing ChunkLoadError.
REM Solution: Start Next.js first, wait for HTTP 200, then start Tauri.
REM (beforeDevCommand is set to "" in tauri.conf.json to avoid double-start)

echo [INFO] Starting Next.js dev server in background...
start "Next.js Dev Server" /min cmd /c "call pnpm dev"

REM Poll for HTTP 200 (up to 180 seconds = 60 iterations x 3 seconds)
echo [INFO] Waiting for Next.js dev server to start...
set "NEXT_READY=0"
for /l %%i in (1,1,60) do (
    if "!NEXT_READY!"=="0" (
        ping -n 4 127.0.0.1 >nul
        powershell -Command "try { $r = Invoke-WebRequest -Uri 'http://localhost:%PORT%' -UseBasicParsing -TimeoutSec 5; if ($r.StatusCode -eq 200) { exit 0 } else { exit 1 } } catch { exit 1 }" >nul 2>&1
        if !ERRORLEVEL! equ 0 (
            set "NEXT_READY=1"
        )
    )
)
if "!NEXT_READY!"=="0" (
    echo [ERROR] Next.js dev server failed to become ready within 180 seconds.
    echo [ERROR] Please check the Next.js Dev Server window for errors.
    pause
    exit /b 1
)
echo [INFO] Next.js dev server is running.

REM Pre-warm the page (trigger first compilation and wait for full response)
REM In Next.js dev mode, the first request triggers on-demand compilation.
REM The request is held open until compilation completes, then the response is sent.
REM We use a 300-second timeout to allow for slow first-time compilation.
echo [INFO] Triggering Next.js first compilation...
echo [INFO] This may take 1-3 minutes on first run. Please wait...
powershell -Command "try { $r = Invoke-WebRequest -Uri 'http://localhost:%PORT%/' -UseBasicParsing -TimeoutSec 300; if ($r.StatusCode -eq 200) { exit 0 } else { exit 1 } } catch { exit 1 }"
if !ERRORLEVEL! neq 0 (
    echo [ERROR] Next.js compilation timed out after 300 seconds or failed.
    echo [ERROR] Please check the Next.js Dev Server window for errors.
    echo [ERROR] If compilation is just slow, try running start.bat again.
    pause
    exit /b 1
)
echo [INFO] Next.js page compilation completed successfully.

REM Verify layout.js chunk is available (double-check)
echo [INFO] Verifying layout.js chunk availability...
powershell -Command "try { $r = Invoke-WebRequest -Uri 'http://localhost:%PORT%/_next/static/chunks/app/layout.js' -UseBasicParsing -TimeoutSec 60; if ($r.StatusCode -eq 200) { exit 0 } else { exit 1 } } catch { exit 1 }" >nul 2>&1
if !ERRORLEVEL! neq 0 (
    echo [WARN] layout.js chunk verification timed out, but page loaded. Continuing...
) else (
    echo [INFO] layout.js chunk verified. Next.js is fully ready.
)
echo.
echo [INFO] Starting Tauri...
echo.

REM Run tauri:dev (beforeDevCommand is empty, Tauri uses the running Next.js)
call pnpm tauri:dev

REM If we reach here, tauri dev exited
echo.
echo [INFO] Tauri dev server has stopped.
echo.
pause
exit /b 0
