# AnyKey Filter Driver - DEVICE-level UpperFilter install (correct position).
#
# Why device-level, not class-level:
#   Class UpperFilters (Control\Class\{GUID}\UpperFilters) puts the filter ABOVE
#   kbdclass -> CONNECT (kbdclass->port, downward) never reaches the filter.
#   Device UpperFilters (Enum\<InstanceId>\UpperFilters, a.k.a. HKR in INF
#   DDInstall.HW) puts the filter ABOVE the port driver but BELOW kbdclass ->
#   CONNECT flows kbdclass -> anykey_flt -> port. This is the kbfiltr position.
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
$srcSys  = $candidates | Where-Object { Test-Path $_ } | Select-Object -First 1
$dstSys  = "$env:WINDIR\System32\drivers\$svc.sys"
$logFile = Join-Path $scriptDir "_install_anykey_device.log"

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

    $keyboards = Get-PnpDevice -Class Keyboard -ErrorAction SilentlyContinue
    if (-not $keyboards) { Write-Host "WARNING: No keyboard devices found via Get-PnpDevice" }
    foreach ($kbd in $keyboards) {
        $id  = $kbd.InstanceId
        $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$id"
        if (-not (Test-Path $key)) { Write-Host "  [skip] $id (enum key missing)"; continue }
        $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
        if (-not $cur) { $cur = @() }
        if ($cur -contains $svc) { Write-Host "  [ok]   $id already has $svc"; continue }
        $new = @($cur) + $svc
        New-ItemProperty -Path $key -Name UpperFilters -Value $new -PropertyType MultiString -Force | Out-Null
        Write-Host "  [set]  $id (Keyboard)"
        Write-Host "         UpperFilters = $($new -join ', ')"
    }

    # Mouse devices (v0.2)
    $mice = Get-PnpDevice -Class Mouse -ErrorAction SilentlyContinue
    if (-not $mice) { Write-Host "WARNING: No mouse devices found via Get-PnpDevice" }
    foreach ($mouse in $mice) {
        $id  = $mouse.InstanceId
        $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$id"
        if (-not (Test-Path $key)) { Write-Host "  [skip] $id (enum key missing)"; continue }
        $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
        if (-not $cur) { $cur = @() }
        if ($cur -contains $svc) { Write-Host "  [ok]   $id already has $svc"; continue }
        $new = @($cur) + $svc
        New-ItemProperty -Path $key -Name UpperFilters -Value $new -PropertyType MultiString -Force | Out-Null
        Write-Host "  [set]  $id (Mouse)"
        Write-Host "         UpperFilters = $($new -join ', ')"
    }
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
    Stop-Transcript | Out-Null
    exit 1
}
