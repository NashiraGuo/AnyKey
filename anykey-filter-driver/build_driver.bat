@echo off
setlocal
REM ── AnyKey filter driver build (reconstructed from MSBuild tlog) ──
REM VS2022 Community 14.44 + WDK 10.0.28000.0 + KMDF 1.15

set "PROJDIR=%~dp0"
cd /d "%PROJDIR%"

set "SDKVER=10.0.28000.0"
REM PROGRA~2 = 8.3 short name of "Program Files (x86)", removing that dir's
REM spaces from every %WK% path. (Note: "Windows Kits" still contains a space;
REM quotes in CLFLAGS / link args protect it — do NOT introduce backslash-
REM before-quote (\" ) patterns, they corrupt cmd's quote tracking.)
set "WK=C:\PROGRA~2\Windows Kits\10"
set "SYS=%PROJDIR%sys"

REM --- VS dev env (sets cl/link/PATH + VC CRT include) ---
call "C:\Program Files\Microsoft Visual Studio\2022\Community\VC\Auxiliary\Build\vcvars64.bat"
if errorlevel 1 goto :fail

REM --- Kernel-mode includes: VC(from vcvars) + km + shared + wdf.
REM     Do NOT add ucrt: it makes ntstrsafe.h pull user-mode _vsnwprintf. ---
set "INCLUDE=%WK%\Include\%SDKVER%\km;%WK%\Include\%SDKVER%\shared;%WK%\Include\wdf\kmdf\1.15;%INCLUDE%"

if not exist "RELEASE\X64" mkdir "RELEASE\X64"
if not exist "BIN\X64\RELEASE" mkdir "BIN\X64\RELEASE"

set "CLFLAGS=/c /I"RELEASE\X64" /I"%SYS%" /Zi /nologo /W4 /WX /diagnostics:column /Ox /Os /Oy- /D _WIN64 /D _AMD64_ /D AMD64 /D _WIN32_WINNT=0x0A00 /D WINVER=0x0A00 /D WINNT=1 /D NTDDI_VERSION=0xA000012 /D _NT_TARGET_VERSION_WIN10 /D KMDF_VERSION_MAJOR=1 /D KMDF_VERSION_MINOR=15 /D NTSTRSAFE_LIB /GF /Gm- /Zp8 /GS /guard:cf /Gy /fp:precise /Qspectre /Zc:wchar_t- /Zc:forScope /Zc:inline /GR- /FoRELEASE\X64\ /Fd"RELEASE\X64\VC143.PDB" /external:W4 /Gz /wd4152 /wd4201 /wd4204 /wd4221 /wd4819 /FI"%WK%\Include\%SDKVER%\shared\warning.h" /FC /kernel -cbstring -d2epilogunwind /d1nodatetime /d1import_no_registry /d2AllowCompatibleILVersions /d2Zi+"

echo === Compiling anykey_flt.c ===
cl %CLFLAGS% "%SYS%\anykey_flt.c"
if errorlevel 1 goto :fail

echo === Compiling rawpdo.c ===
cl %CLFLAGS% "%SYS%\rawpdo.c"
if errorlevel 1 goto :fail

echo === Linking anykey_flt.sys ===
link /OUT:"BIN\X64\RELEASE\ANYKEY_FLT.SYS" /VERSION:"10.0" /INCREMENTAL:NO /NOLOGO /WX /SECTION:"INIT,d" ^
 "%WK%\LIB\%SDKVER%\KM\X64\BUFFEROVERFLOWFASTFAILK.LIB" ^
 "%WK%\LIB\%SDKVER%\KM\X64\NTOSKRNL.LIB" ^
 "%WK%\LIB\%SDKVER%\KM\X64\HAL.LIB" ^
 "%WK%\LIB\%SDKVER%\KM\X64\WMILIB.LIB" ^
 "%WK%\LIB\WDF\KMDF\X64\1.15\WDFLDR.LIB" ^
 "%WK%\LIB\WDF\KMDF\X64\1.15\WDFDRIVERENTRY.LIB" ^
 "%WK%\LIB\%SDKVER%\KM\X64\NTSTRSAFE.LIB" ^
 /NODEFAULTLIB /MANIFEST:NO /DEBUG /PDB:"BIN\X64\RELEASE\ANYKEY_FLT.PDB" /SUBSYSTEM:NATIVE,"10.00" /Driver /OPT:REF /OPT:ICF /ENTRY:"FxDriverEntry" /RELEASE /IMPLIB:"BIN\X64\RELEASE\ANYKEY_FLT.LIB" /MERGE:"_TEXT=.text;_PAGE=PAGE" /MACHINE:X64 /PROFILE /guard:cf /kernel /IGNORE:4198,4010,4037,4039,4065,4070,4078,4087,4089,4221,4108,4088,4218,4218,4235 /osversion:10.0 /pdbcompress /debugtype:pdata ^
 "RELEASE\X64\ANYKEY_FLT.OBJ" "RELEASE\X64\RAWPDO.OBJ"
if errorlevel 1 goto :fail

echo === Signing (test cert) ===
"%WK%\bin\%SDKVER%\x64\signtool.exe" sign /ph /fd "SHA256" /sha1 "5B1F1FD5EEBCE1880DD21D38F503A1C615EE796D" "BIN\X64\RELEASE\ANYKEY_FLT.SYS"
if errorlevel 1 goto :signfail

echo === BUILD OK ===
dir "BIN\X64\RELEASE\ANYKEY_FLT.SYS"
endlocal
exit /b 0

:signfail
echo === BUILD OK but SIGNING FAILED (unsigned .sys is at BIN\X64\RELEASE) ===
endlocal
exit /b 2

:fail
echo === BUILD FAILED ===
endlocal
exit /b 1
