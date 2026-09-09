$ErrorActionPreference = 'Stop'
$proj = 'c:\Users\proje\WorkBuddy\20260419144520\AnyKey\anykey-filter-driver'
Set-Location $proj
$SYS = Join-Path $proj 'sys'
$WK  = 'C:\Program Files (x86)\Windows Kits\10'
$SDK = '10.0.28000.0'

# --- VS dev environment (cl/link/PATH + VC CRT include/lib) ---
& 'C:\Program Files\Microsoft Visual Studio\2022\Community\Common7\Tools\Launch-VsDevShell.ps1' -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
Set-Location $proj

# --- Kernel-mode include set: VC intrinsics + WDK km/shared + WDF only.
#     Deliberately EXCLUDE ucrt/um: including ucrt makes ntstrsafe.h pull the
#     user-mode _vsnwprintf inline (__stdio_common_vswprintf) -> LNK2019. ---
$vcinc = Join-Path $env:VCToolsInstallDir 'include'
$env:INCLUDE = "$vcinc;$WK\Include\$SDK\km;$WK\Include\$SDK\shared;$WK\Include\wdf\kmdf\1.15"

New-Item -ItemType Directory -Force -Path 'RELEASE\X64' | Out-Null
New-Item -ItemType Directory -Force -Path 'BIN\X64\RELEASE' | Out-Null

$cl = @(
 '/c','/IRELEASE\X64\',"/I$SYS",'/Zi','/nologo','/W4','/WX','/diagnostics:column',
 '/Ox','/Os','/Oy-','/D','_WIN64','/D','_AMD64_','/D','AMD64','/D','_WIN32_WINNT=0x0A00',
 '/D','WINVER=0x0A00','/D','WINNT=1','/D','NTDDI_VERSION=0xA000012','/D','_NT_TARGET_VERSION_WIN10',
 '/D','KMDF_VERSION_MAJOR=1','/D','KMDF_VERSION_MINOR=15','/D','NTSTRSAFE_LIB','/GF','/Gm-','/Zp8','/GS','/guard:cf',
 '/Gy','/fp:precise','/Qspectre','/Zc:wchar_t-','/Zc:forScope','/Zc:inline','/GR-',
 '/FoRELEASE\X64\','/FdRELEASE\X64\VC143.PDB','/external:W4','/Gz','/wd4152','/wd4201','/wd4204',
 '/wd4221','/wd4819',"/FI$WK\Include\$SDK\shared\warning.h",'/FC','/kernel',
 '-cbstring','-d2epilogunwind','/d1nodatetime','/d1import_no_registry',
 '/d2AllowCompatibleILVersions','/d2Zi+'
)

Write-Output '=== Compiling anykey_flt.c ==='
& cl.exe @cl "$SYS\anykey_flt.c"
if ($LASTEXITCODE -ne 0) { Write-Output "CL FAILED $LASTEXITCODE"; exit 1 }

Write-Output '=== Compiling rawpdo.c ==='
& cl.exe @cl "$SYS\rawpdo.c"
if ($LASTEXITCODE -ne 0) { Write-Output "CL FAILED $LASTEXITCODE"; exit 1 }

$link = @(
 '/OUT:BIN\X64\RELEASE\ANYKEY_FLT.SYS','/VERSION:10.0','/INCREMENTAL:NO','/NOLOGO','/WX','/SECTION:INIT,d',
 "$WK\LIB\$SDK\KM\X64\BUFFEROVERFLOWFASTFAILK.LIB",
 "$WK\LIB\$SDK\KM\X64\NTOSKRNL.LIB",
 "$WK\LIB\$SDK\KM\X64\HAL.LIB",
 "$WK\LIB\$SDK\KM\X64\WMILIB.LIB",
 "$WK\LIB\WDF\KMDF\X64\1.15\WDFLDR.LIB",
 "$WK\LIB\WDF\KMDF\X64\1.15\WDFDRIVERENTRY.LIB",
 "$WK\LIB\$SDK\KM\X64\NTSTRSAFE.LIB",
 '/NODEFAULTLIB','/MANIFEST:NO','/DEBUG','/PDB:BIN\X64\RELEASE\ANYKEY_FLT.PDB',
 '/SUBSYSTEM:NATIVE,10.00','/Driver','/OPT:REF','/OPT:ICF','/ENTRY:FxDriverEntry','/RELEASE',
 '/IMPLIB:BIN\X64\RELEASE\ANYKEY_FLT.LIB','/MERGE:_TEXT=.text;_PAGE=PAGE','/MACHINE:X64','/PROFILE',
 '/guard:cf','/kernel','/IGNORE:4198,4010,4037,4039,4065,4070,4078,4087,4089,4221,4108,4088,4218,4218,4235',
 '/osversion:10.0','/pdbcompress','/debugtype:pdata',
 'RELEASE\X64\ANYKEY_FLT.OBJ','RELEASE\X64\RAWPDO.OBJ'
)

Write-Output '=== Linking anykey_flt.sys ==='
& link.exe @link
if ($LASTEXITCODE -ne 0) { Write-Output "LINK FAILED $LASTEXITCODE"; exit 1 }

Write-Output '=== Signing (test cert) ==='
& "$WK\bin\$SDK\x64\signtool.exe" sign /ph /fd SHA256 /sha1 5B1F1FD5EEBCE1880DD21D38F503A1C615EE796D 'BIN\X64\RELEASE\ANYKEY_FLT.SYS'
if ($LASTEXITCODE -ne 0) { Write-Output "SIGN FAILED $LASTEXITCODE (unsigned .sys still produced)"; exit 2 }

Write-Output '=== BUILD OK ==='
Get-Item 'BIN\X64\RELEASE\ANYKEY_FLT.SYS' | Select-Object FullName,Length,LastWriteTime | Format-List
