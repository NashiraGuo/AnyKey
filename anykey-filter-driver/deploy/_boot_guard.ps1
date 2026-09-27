# AnyKey - one-shot boot guard (failure recovery).
#
# WHY THIS EXISTS
#   Writing "UpperFilters = anykey_flt" onto keyboard/mouse device keys makes the
#   filter a MEMBER of that device's driver stack. If the image cannot be loaded
#   on the next boot (unsigned driver while test signing is not really in effect,
#   HVCI, WDAC, ...), the whole stack fails to start and the user ends up with NO
#   keyboard and NO mouse - so they cannot even run the uninstaller.
#
#   This script is registered by _install_anykey_device.ps1 as a one-shot SYSTEM
#   task ("AnyKeyBootGuard", trigger AtStartup) BEFORE any device key is written.
#   It runs once after the next boot:
#       driver loaded fine  -> unregister itself, exit
#       lockout detected    -> undo the bindings, remove service + .sys, log,
#                              then reboot so the user gets a working PC back
#   It unregisters itself in every case, so it can never fire twice. If the
#   service is already gone it does nothing but clean up, so a stale task cannot
#   reboot a machine for no reason.
#
# ASCII only. Registered with -WindowStyle Hidden (no console window).

$svc      = 'anykey_flt'
$taskName = 'AnyKeyBootGuard'
$dstSys   = "$env:WINDIR\System32\drivers\$svc.sys"
$dataDir  = Join-Path $env:ProgramData 'AnyKey'
$logFile  = Join-Path $dataDir 'boot_guard.log'
$waitSec  = 45          # let PnP finish enumerating before judging

if (-not (Test-Path $dataDir)) { New-Item -ItemType Directory -Path $dataDir -Force | Out-Null }

function Write-Log([string]$msg) {
    $line = "{0}  {1}" -f (Get-Date).ToString('yyyy-MM-dd HH:mm:ss'), $msg
    Add-Content -Path $logFile -Value $line -Encoding UTF8 -ErrorAction SilentlyContinue
}

function Remove-GuardTask {
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue
}

Write-Log "guard start (waiting ${waitSec}s for PnP to settle)"
Start-Sleep -Seconds $waitSec

# ---- 1. Current state -----------------------------------------------------
$svcObj    = Get-Service $svc -ErrorAction SilentlyContinue
$svcExists = [bool]$svcObj
$driverUp  = ($svcExists -and $svcObj.Status -eq 'Running')

$bound = @()    # instances whose UpperFilters still list our service
$bad   = @()    # of those, instances that are present and not OK
foreach ($class in @('Keyboard', 'Mouse')) {
    $devices = Get-PnpDevice -Class $class -ErrorAction SilentlyContinue
    if (-not $devices) { continue }
    foreach ($dev in $devices) {
        $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$($dev.InstanceId)"
        if (-not (Test-Path $key)) { continue }
        $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
        if (-not $cur -or -not ($cur -contains $svc)) { continue }
        $bound += $dev.InstanceId
        if ($dev.Present -and $dev.Status -ne 'OK') { $bad += $dev.InstanceId }
    }
}

Write-Log ("state: service_exists={0} driver_running={1} bound={2} bad={3}" -f `
           $svcExists, $driverUp, $bound.Count, $bad.Count)

# ---- 2. Decide ------------------------------------------------------------
if (-not $svcExists -and $bound.Count -eq 0) {
    Write-Log "nothing installed - nothing to do"
    Remove-GuardTask
    exit 0
}
if ($driverUp -and $bad.Count -eq 0) {
    Write-Log "OK: $svc is running and every filtered device started. Guard done."
    Remove-GuardTask
    exit 0
}

if (-not $driverUp) {
    Write-Log "LOCKOUT: $svc is not running while bindings exist - the filter could not be loaded."
} else {
    Write-Log "LOCKOUT: filtered device(s) failed to start: $($bad -join ' ; ')"
}

# ---- 3. Roll back --------------------------------------------------------
# Bindings FIRST: never leave an UpperFilters entry pointing at a missing service.
$changed = $false
foreach ($id in $bound) {
    $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$id"
    if (-not (Test-Path $key)) { continue }
    $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
    if (-not $cur -or -not ($cur -contains $svc)) { continue }
    $rest = @($cur | Where-Object { $_ -ne $svc })
    if ($rest.Count -eq 0) {
        Remove-ItemProperty -Path $key -Name UpperFilters -Force -ErrorAction SilentlyContinue
    } else {
        New-ItemProperty -Path $key -Name UpperFilters -Value $rest -PropertyType MultiString -Force | Out-Null
    }
    Write-Log "  removed $svc from $id"
    $changed = $true
}

if ($svcExists) {
    cmd /c "sc.exe stop $svc 2>&1"   | Out-Null
    cmd /c "sc.exe delete $svc 2>&1" | Out-Null
    Write-Log "  service $svc stopped and deleted"
    $changed = $true
}
if (Test-Path $dstSys) {
    try {
        Remove-Item $dstSys -Force -ErrorAction Stop
        Write-Log "  removed $dstSys"
    } catch {
        Move-Item $dstSys "$dstSys.delete_me" -Force -ErrorAction SilentlyContinue
        Write-Log "  $dstSys in use - renamed; it disappears after the reboot"
    }
    $changed = $true
}

Remove-GuardTask
Write-Log "rollback done (changed=$changed), guard unregistered"

if ($changed) {
    Write-Log "rebooting so the device stacks come back without the filter"
    Restart-Computer -Force
}
exit 0
