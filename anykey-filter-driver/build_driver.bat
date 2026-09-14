@echo off
setlocal EnableExtensions
REM ============================================================================
REM  AnyKey filter driver build: compile + test-sign ANYKEY_FLT.SYS
REM
REM  Nothing machine-specific is hardcoded. Everything that differs between
REM  machines is auto-detected, and every detected value can be overridden with
REM  an environment variable:
REM
REM    WKROOT                  Windows Kits 10 root
REM    ANYKEY_WDK_VERSION      e.g. 10.0.26100.0  (default: pinned, else newest)
REM    ANYKEY_KMDF_VERSION     e.g. 1.15          (default: pinned, else newest)
REM    ANYKEY_VCVARS           full path to vcvars64.bat
REM    ANYKEY_SIGN_THUMBPRINT  code-signing certificate thumbprint. When unset,
REM                            the author's test cert is used if present,
REM                            otherwise a fresh test certificate named
REM                            "CN=AnyKey Test Driver" is created in
REM                            Cert:\CurrentUser\My.
REM
REM  Exit codes: 0 = built and signed
REM              1 = setup, compile or link failed
REM              2 = built but signing failed (unsigned .sys left in BIN)
REM ============================================================================

set "PROJDIR=%~dp0"
cd /d "%PROJDIR%"
set "SYS=%PROJDIR%sys"
set "PF86=%ProgramFiles(x86)%"
set "PF64=%ProgramFiles%"
set "PINNED_WDK=10.0.28000.0"
set "PINNED_KMDF=1.15"
set "PINNED_THUMB=5B1F1FD5EEBCE1880DD21D38F503A1C615EE796D"

REM --- 1/5  Windows Kits root ------------------------------------------------
set "WKROOT_SRC=env WKROOT"
if not defined WKROOT if exist "%PF86%\Windows Kits\10\Include" (set "WKROOT=%PF86%\Windows Kits\10" & set "WKROOT_SRC=auto-detected")
if not defined WKROOT if exist "%PF64%\Windows Kits\10\Include" (set "WKROOT=%PF64%\Windows Kits\10" & set "WKROOT_SRC=auto-detected")
if not defined WKROOT (
    echo ERROR: Windows Kits 10 not found.
    echo        Install the WDK, or set WKROOT to your Windows Kits 10 folder.
    exit /b 1
)
if not exist "%WKROOT%\Include" (
    echo ERROR: WKROOT has no Include folder: "%WKROOT%"
    exit /b 1
)

REM --- 2/5  WDK (SDK) version -----------------------------------------------
set "SDKVER="
set "SDKVER_SRC="
if defined ANYKEY_WDK_VERSION (set "SDKVER=%ANYKEY_WDK_VERSION%" & set "SDKVER_SRC=env ANYKEY_WDK_VERSION")
if not defined SDKVER if exist "%WKROOT%\Include\%PINNED_WDK%" (set "SDKVER=%PINNED_WDK%" & set "SDKVER_SRC=pinned %PINNED_WDK%")
if not defined SDKVER for /f "delims=" %%V in ('dir /b /ad /o-n "%WKROOT%\Include" 2^>nul ^| findstr /r /c:"^10\.0\."') do if not defined SDKVER (set "SDKVER=%%V" & set "SDKVER_SRC=auto-detected")
if not defined SDKVER (
    echo ERROR: no WDK 10.0.x found under "%WKROOT%\Include".
    echo        Install the WDK, or set ANYKEY_WDK_VERSION to an installed one.
    exit /b 1
)
if not exist "%WKROOT%\Include\%SDKVER%\km" (
    echo ERROR: WDK kernel headers missing for %SDKVER%.
    echo        Expected "%WKROOT%\Include\%SDKVER%\km".
    echo        Installed versions:
    dir /b /ad "%WKROOT%\Include" 2>nul
    exit /b 1
)

REM --- 3/5  KMDF version ----------------------------------------------------
set "KMDFVER="
set "KMDF_SRC="
if defined ANYKEY_KMDF_VERSION (set "KMDFVER=%ANYKEY_KMDF_VERSION%" & set "KMDF_SRC=env ANYKEY_KMDF_VERSION")
if not defined KMDFVER if exist "%WKROOT%\Include\wdf\kmdf\%PINNED_KMDF%" (set "KMDFVER=%PINNED_KMDF%" & set "KMDF_SRC=pinned %PINNED_KMDF%")
if not defined KMDFVER for /f "delims=" %%V in ('dir /b /ad /o-n "%WKROOT%\Include\wdf\kmdf" 2^>nul ^| findstr /r /c:"^1\."') do if not defined KMDFVER (set "KMDFVER=%%V" & set "KMDF_SRC=auto-detected")
if not defined KMDFVER (
    echo ERROR: KMDF headers not found under "%WKROOT%\Include\wdf\kmdf".
    echo        Install the WDK KMDF package, or set ANYKEY_KMDF_VERSION.
    exit /b 1
)
if not exist "%WKROOT%\Lib\WDF\KMDF\x64\%KMDFVER%\WDFLDR.LIB" (
    echo ERROR: KMDF x64 libraries for %KMDFVER% are missing.
    echo        Expected "%WKROOT%\Lib\WDF\KMDF\x64\%KMDFVER%".
    echo        Set ANYKEY_KMDF_VERSION to a version that ships both headers and libs.
    exit /b 1
)
set "KMDFMAJ="
set "KMDFMIN="
for /f "tokens=1,2 delims=." %%A in ("%KMDFVER%") do (set "KMDFMAJ=%%A" & set "KMDFMIN=%%B")
if not defined KMDFMIN set "KMDFMIN=0"

REM --- 4/5  Visual Studio C++ toolchain -------------------------------------
set "VCVARS="
set "VCVARS_SRC="
if defined ANYKEY_VCVARS (set "VCVARS=%ANYKEY_VCVARS%" & set "VCVARS_SRC=env ANYKEY_VCVARS")
set "VSWHERE=%PF86%\Microsoft Visual Studio\Installer\vswhere.exe"
if not defined VCVARS if exist "%VSWHERE%" for /f "usebackq delims=" %%P in (`"%VSWHERE%" -latest -products * -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath 2^>nul`) do if not defined VCVARS (set "VCVARS=%%P\VC\Auxiliary\Build\vcvars64.bat" & set "VCVARS_SRC=vswhere")
if not defined VCVARS for %%E in (Community Professional Enterprise BuildTools) do if not defined VCVARS if exist "%PF64%\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat" (set "VCVARS=%PF64%\Microsoft Visual Studio\2022\%%E\VC\Auxiliary\Build\vcvars64.bat" & set "VCVARS_SRC=well-known %%E")
if not defined VCVARS (
    echo ERROR: vcvars64.bat not found.
    echo        Install Visual Studio 2022 with the C++ workload, or set
    echo        ANYKEY_VCVARS to the full path of vcvars64.bat.
    exit /b 1
)

REM --- 5/5  signing certificate --------------------------------------------
set "THUMB="
set "THUMB_SRC="
set "CERTHIT="
if defined ANYKEY_SIGN_THUMBPRINT set "THUMB=%ANYKEY_SIGN_THUMBPRINT%"
if defined ANYKEY_SIGN_THUMBPRINT set "THUMB_SRC=env ANYKEY_SIGN_THUMBPRINT"
if defined ANYKEY_SIGN_THUMBPRINT call :cert_exists "%ANYKEY_SIGN_THUMBPRINT%"
if defined ANYKEY_SIGN_THUMBPRINT if not defined CERTHIT (
    echo ERROR: ANYKEY_SIGN_THUMBPRINT=%ANYKEY_SIGN_THUMBPRINT% not found in
    echo        Cert:\CurrentUser\My or Cert:\LocalMachine\My.
    echo        Create one, or unset the variable to let this script create it.
    exit /b 1
)
if not defined THUMB call :cert_exists "%PINNED_THUMB%"
if not defined THUMB if defined CERTHIT (set "THUMB=%PINNED_THUMB%" & set "THUMB_SRC=pinned author cert")
if not defined THUMB (
    echo [cert] No known signing certificate found - creating a test certificate.
    for /f "usebackq delims=" %%T in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "$c = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=AnyKey Test Driver' -CertStoreLocation Cert:\CurrentUser\My -NotAfter (Get-Date).AddYears(10); $c.Thumbprint"`) do set "THUMB=%%T"
    set "THUMB_SRC=created CN=AnyKey Test Driver"
)
if not defined THUMB (
    echo ERROR: could not determine a signing certificate.
    echo        Set ANYKEY_SIGN_THUMBPRINT to a thumbprint in your certificate store.
    exit /b 1
)

echo.
echo === AnyKey filter driver build ===
echo   Windows Kits  : %WKROOT%   [%WKROOT_SRC%]
echo   WDK version   : %SDKVER%   [%SDKVER_SRC%]
echo   KMDF version  : %KMDFVER%   [%KMDF_SRC%]
echo   VS toolchain  : %VCVARS%   [%VCVARS_SRC%]
echo   Signing cert  : %THUMB%   [%THUMB_SRC%]
echo.

if not exist "BIN\X64\RELEASE" mkdir "BIN\X64\RELEASE"
if not exist "RELEASE\X64" mkdir "RELEASE\X64"

REM Hand the resolved thumbprint to the Python wrapper (build/build_driver_release.py)
REM so the certificate is configured in exactly one place.
>"BIN\X64\RELEASE\signing_thumbprint.txt" echo %THUMB%

if not exist "%WKROOT%\bin\%SDKVER%\x64\signtool.exe" (
    echo ERROR: signtool.exe missing under "%WKROOT%\bin\%SDKVER%\x64".
    echo        This Windows Kits install has no WDK signing tools.
    exit /b 1
)

REM --- VS dev env: sets cl/link/PATH and the VC CRT include -----------------
call "%VCVARS%"
if errorlevel 1 goto :fail

REM --- Kernel-mode includes: VC (from vcvars) + km + shared + wdf.
REM     Do NOT add ucrt: it makes ntstrsafe.h pull user-mode _vsnwprintf. ---
set "INCLUDE=%WKROOT%\Include\%SDKVER%\km;%WKROOT%\Include\%SDKVER%\shared;%WKROOT%\Include\wdf\kmdf\%KMDFVER%;%INCLUDE%"

REM --- Compile flags. Paths containing spaces stay quoted; never introduce a
REM     backslash-before-quote (\" ) pattern, it corrupts cmd quote tracking. --
set "CLFLAGS=/c /I"RELEASE\X64" /I"%SYS%" /Zi /nologo /W4 /WX /diagnostics:column /Ox /Os /Oy- /D _WIN64 /D _AMD64_ /D AMD64 /D _WIN32_WINNT=0x0A00 /D WINVER=0x0A00 /D WINNT=1 /D NTDDI_VERSION=0xA000012 /D _NT_TARGET_VERSION_WIN10 /D KMDF_VERSION_MAJOR=%KMDFMAJ% /D KMDF_VERSION_MINOR=%KMDFMIN% /D NTSTRSAFE_LIB /GF /Gm- /Zp8 /GS /guard:cf /Gy /fp:precise /Qspectre /Zc:wchar_t- /Zc:forScope /Zc:inline /GR- /FoRELEASE\X64\ /Fd"RELEASE\X64\VC143.PDB" /external:W4 /Gz /wd4152 /wd4201 /wd4204 /wd4221 /wd4819 /FI"%WKROOT%\Include\%SDKVER%\shared\warning.h" /FC /kernel -cbstring -d2epilogunwind /d1nodatetime /d1import_no_registry /d2AllowCompatibleILVersions /d2Zi+"

echo === Compiling anykey_flt.c ===
cl %CLFLAGS% "%SYS%\anykey_flt.c"
if errorlevel 1 goto :fail

echo === Compiling rawpdo.c ===
cl %CLFLAGS% "%SYS%\rawpdo.c"
if errorlevel 1 goto :fail

echo === Linking anykey_flt.sys ===
link /OUT:"BIN\X64\RELEASE\ANYKEY_FLT.SYS" /VERSION:"10.0" /INCREMENTAL:NO /NOLOGO /WX /SECTION:"INIT,d" ^
 "%WKROOT%\LIB\%SDKVER%\KM\X64\BUFFEROVERFLOWFASTFAILK.LIB" ^
 "%WKROOT%\LIB\%SDKVER%\KM\X64\NTOSKRNL.LIB" ^
 "%WKROOT%\LIB\%SDKVER%\KM\X64\HAL.LIB" ^
 "%WKROOT%\LIB\%SDKVER%\KM\X64\WMILIB.LIB" ^
 "%WKROOT%\LIB\WDF\KMDF\X64\%KMDFVER%\WDFLDR.LIB" ^
 "%WKROOT%\LIB\WDF\KMDF\X64\%KMDFVER%\WDFDRIVERENTRY.LIB" ^
 "%WKROOT%\LIB\%SDKVER%\KM\X64\NTSTRSAFE.LIB" ^
 /NODEFAULTLIB /MANIFEST:NO /DEBUG /PDB:"BIN\X64\RELEASE\ANYKEY_FLT.PDB" /SUBSYSTEM:NATIVE,"10.00" /Driver /OPT:REF /OPT:ICF /ENTRY:"FxDriverEntry" /RELEASE /IMPLIB:"BIN\X64\RELEASE\ANYKEY_FLT.LIB" /MERGE:"_TEXT=.text;_PAGE=PAGE" /MACHINE:X64 /PROFILE /guard:cf /kernel /IGNORE:4198,4010,4037,4039,4065,4070,4078,4087,4089,4221,4108,4088,4218,4218,4235 /osversion:10.0 /pdbcompress /debugtype:pdata ^
 "RELEASE\X64\ANYKEY_FLT.OBJ" "RELEASE\X64\RAWPDO.OBJ"
if errorlevel 1 goto :fail

echo === Signing (test cert) ===
"%WKROOT%\bin\%SDKVER%\x64\signtool.exe" sign /ph /fd "SHA256" /sha1 "%THUMB%" "BIN\X64\RELEASE\ANYKEY_FLT.SYS"
if errorlevel 1 goto :signfail

echo === BUILD OK ===
dir "BIN\X64\RELEASE\ANYKEY_FLT.SYS"
endlocal
exit /b 0

:signfail
echo === BUILD OK but SIGNING FAILED (unsigned .sys is at BIN\X64\RELEASE) ===
echo     Certificate used: %THUMB%
endlocal
exit /b 2

:fail
echo === BUILD FAILED ===
endlocal
exit /b 1

REM ---------------------------------------------------------------------------
REM  :cert_exists <thumbprint>  ->  sets CERTHIT=yes when the thumbprint exists
REM  in a code-signing capable store (CurrentUser\My or LocalMachine\My).
REM ---------------------------------------------------------------------------
:cert_exists
REM Sets CERTHIT=yes when %1 is found in a store signtool can read.
REM NOTE: no ^| inside the quoted -Command string: cmd does not treat ^ as an
REM escape character inside double quotes, so the caret would reach PowerShell
REM and bind to Get-ChildItem's -Filter (unsupported by the cert provider).
set "CERTHIT="
for /f "usebackq delims=" %%H in (`powershell -NoProfile -ExecutionPolicy Bypass -Command "if ((Test-Path ('Cert:\CurrentUser\My\' + '%~1'.ToUpper())) -or (Test-Path ('Cert:\LocalMachine\My\' + '%~1'.ToUpper()))) { 'yes' }"`) do set "CERTHIT=%%H"
exit /b 0
