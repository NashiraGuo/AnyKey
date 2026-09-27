# AnyKey Filter Driver - DEVICE-level UpperFilter install (correct position).
#
# Why device-level, not class-level:
#   Class UpperFilters (Control\Class\{GUID}\UpperFilters) puts the filter ABOVE
#   kbdclass -> CONNECT (kbdclass->port, downward) never reaches the filter.
#   Device UpperFilters (Enum\<InstanceId>\UpperFilters, a.k.a. HKR in INF
#   DDInstall.HW) puts the filter ABOVE the port driver but BELOW kbdclass ->
#   CONNECT flows kbdclass -> anykey_flt -> port. This is the kbfiltr position.
#
# SAFETY MODEL (an UpperFilters entry makes the filter a MEMBER of the device
# stack - if the image cannot be loaded the whole stack fails to start, and the
# user is left with no keyboard and no mouse):
#   1. Install_AnyKey_Filter.bat calls this script only when test signing is
#      already ON and Secure Boot is off (_check_test_signing.ps1).
#   2. This script registers a one-shot boot guard BEFORE writing any device key.
#      If the driver fails to load on the next boot, the guard undoes the
#      bindings and reboots into a working machine (_boot_guard.ps1). A missing
#      guard while bindings exist is exactly the lockout we must never create,
#      so the install refuses to continue without it.
#
# Enum keys are writable only by SYSTEM (not even admins). This script
# self-elevates to SYSTEM via a temporary scheduled task if not already SYSTEM.
# All paths are resolved relative to THIS script's directory (scheduled tasks
# run with cwd=System32, so relative paths would break).
#
# ASCII-only to avoid encoding issues in Windows PowerShell 5.1.
# Run as Administrator. Reboot after.

$ErrorActionPreference = 'Stop'
$svc        = 'anykey_flt'                       # change to 'hello_flt' for the minimal driver
$scriptPath = $MyInvocation.MyCommand.Path
$scriptDir  = Split-Path -Parent $scriptPath
# Look for the .sys in a few candidate locations (user may have copied just the script).
$candidates = @(
    (Join-Path $scriptDir "$svc.sys"),
    (Join-Path $scriptDir "Release\$svc.sys")
)
$srcSys    = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
$dstSys    = "$env:WINDIR\System32\drivers\$svc.sys"
$logFile   = Join-Path $scriptDir "_install_anykey_device.log"
$dataDir   = Join-Path $env:ProgramData 'AnyKey'
$guardSrc  = Join-Path $scriptDir '_boot_guard.ps1'
$guardDst  = Join-Path $dataDir 'boot_guard.ps1'
$guardTask = 'AnyKeyBootGuard'

# --- Self-elevate to SYSTEM if not already ---
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
if ($identity.Name -ne 'NT AUTHORITY\SYSTEM') {
    Write-Host "Current user: $($identity.Name). Re-launching as SYSTEM via scheduled task..."
    Write-Host "scriptDir = $scriptDir"
    Write-Host "srcSys    = $srcSys  (exists: $(if ($srcSys) { Test-Path $srcSys } else { 'no path' }))"
    $taskName  = "AnyKeyInstall_$(Get-Random)"
    $action    = New-ScheduledTaskAction -Execute 'powershell.exe' `
                   -Argument "-ExecutionPolicy Bypass -NoProfile -File `"$scriptPath`"" `
                   -WorkingDirectory $scriptDir
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Force | Out-Null
    Start-ScheduledTask -TaskName $taskName
    for ($i = 0; $i -lt 30; $i++) {
        Start-Sleep -Seconds 1
        if ((Get-ScheduledTask -TaskName $taskName).State -ne 'Running') { break }
    }
    $info = Get-ScheduledTaskInfo -TaskName $taskName
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    Write-Host ""
    Write-Host "=== SYSTEM task log ($logFile) ==="
    if (Test-Path $logFile) {
        Get-Content $logFile | ForEach-Object { Write-Host $_ }
    } else {
        Write-Host "(no log produced - task likely failed to start the script)"
    }
    Write-Host ""
    if ($info.LastTaskResult -eq 0) {
        Write-Host "SUCCESS. Reboot to load the filter."
        Write-Host "If the driver fails to load, the boot guard ('$guardTask') will undo"
        Write-Host "the install and reboot automatically."
    } else {
        Write-Host "FAILED (LastResult=0x$('{0:X}' -f $info.LastTaskResult))."
    }
    exit $info.LastTaskResult
}

# --- Running as SYSTEM: log everything to $logFile ---
Start-Transcript -Path $logFile -Force | Out-Null
try {
    Write-Host "Running as SYSTEM. scriptDir=$scriptDir"

    if (-not $srcSys) {
        throw "Driver .sys not found. Looked in: $($candidates -join ' ; ')"
    }
    Copy-Item $srcSys $dstSys -Force
    Write-Host "Copied $svc.sys ($srcSys) -> $dstSys"

    if (-not (Get-Service $svc -ErrorAction SilentlyContinue)) {
        & sc.exe create $svc type= kernel start= demand binPath= $dstSys | Out-Null
    }
    $svcKey = "HKLM:\SYSTEM\CurrentControlSet\Services\$svc"
    New-ItemProperty -Path $svcKey -Name Group -Value 'Keyboard Port' -PropertyType String -Force | Out-Null
    Write-Host "Service $svc created (type=kernel, start=demand, group=Keyboard Port)"

    # ---- Boot guard, registered BEFORE any device key is touched ----
    # The script is copied out of the release folder into ProgramData, because the
    # user may move or delete the extracted folder before the next boot.
    if (-not (Test-Path $guardSrc)) {
        throw "Boot guard not found: $guardSrc - the release package is incomplete."
    }
    if (-not (Test-Path $dataDir)) { New-Item -ItemType Directory -Path $dataDir -Force | Out-Null }
    Copy-Item $guardSrc $guardDst -Force

    Unregister-ScheduledTask -TaskName $guardTask -Confirm:$false -ErrorAction SilentlyContinue
    $guardAction  = New-ScheduledTaskAction -Execute 'powershell.exe' `
                      -Argument "-ExecutionPolicy Bypass -NoProfile -WindowStyle Hidden -File `"$guardDst`""
    $guardTrigger = New-ScheduledTaskTrigger -AtStartup
    $guardSet     = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
                      -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 15) `
                      -MultipleInstances IgnoreNew
    $guardPrin    = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask -TaskName $guardTask -Action $guardAction -Trigger $guardTrigger `
        -Principal $guardPrin -Settings $guardSet -Force | Out-Null
    Write-Host "Boot guard registered ('$guardTask'), script at $guardDst"

    # ---- Bind the filter on every keyboard / mouse device instance ----
    $bound = 0
    foreach ($class in @('Keyboard', 'Mouse')) {
        $devices = Get-PnpDevice -Class $class -ErrorAction SilentlyContinue
        if (-not $devices) { Write-Host "WARNING: no $class device found via Get-PnpDevice"; continue }
        foreach ($dev in $devices) {
            $id  = $dev.InstanceId
            $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$id"
            if (-not (Test-Path $key)) { Write-Host "  [skip] $id (enum key missing)"; continue }
            $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
            if (-not $cur) { $cur = @() }
            if ($cur -contains $svc) {
                Write-Host "  [ok]   $id ($class) already has $svc"
                $bound++
                continue
            }
            $new = @($cur) + $svc
            New-ItemProperty -Path $key -Name UpperFilters -Value $new -PropertyType MultiString -Force | Out-Null
            Write-Host "  [set]  $id ($class)"
            Write-Host "         UpperFilters = $($new -join ', ')"
            $bound++
        }
    }
    Write-Host "Filter bound on $bound device instance(s)."
    Write-Host "INSTALL OK. Reboot to load."

    # KDNET safety: if debug is enabled, lock hostip to 127.0.0.1
    # so a wrong/missing IP never blocks kernel init (and keyboard).
    $debugOn = cmd /c "bcdedit /enum {current} 2>&1" | Select-String "debug.*Yes"
    if ($debugOn) {
        cmd /c 'bcdedit /set {current} hostip 127.0.0.1 2>&1' | Out-Null
        Write-Host "KDNET: debug on, hostip locked to 127.0.0.1 (safe)"
    }

    Stop-Transcript | Out-Null
    exit 0
} catch {
    Write-Host "ERROR: $_"
    Write-Host $_.ScriptStackTrace
    Write-Host ""
    Write-Host "The install did NOT complete. Your keyboard and mouse are still usable."
    Write-Host "The boot guard '$guardTask' is registered, so a reboot is safe either way,"
    Write-Host "but re-run the installer to get a complete installation."
    Stop-Transcript | Out-Null
    exit 1
}
