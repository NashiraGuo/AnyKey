# AnyKey Filter Driver - Robust uninstall (pure ASCII).
# 1. Removes UpperFilters from all keyboard Enum keys (SYSTEM)
# 2. Stops + deletes the kernel service
# 3. Removes .sys (flags for delete-on-reboot if in use)
# 4. Optionally removes INF from pnputil driver store
# 5. Verifies cleanup
# Self-elevates to SYSTEM. Run as Admin.

$ErrorActionPreference = 'Stop'
$svc        = 'anykey_flt'
$scriptPath = $MyInvocation.MyCommand.Path
$scriptDir  = Split-Path -Parent $scriptPath
$dstSys     = "$env:WINDIR\System32\drivers\$svc.sys"
$logFile    = Join-Path $scriptDir "_uninstall.log"

$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent()
if ($identity.Name -ne 'NT AUTHORITY\SYSTEM') {
    Write-Host "Current user: $($identity.Name). Elevating to SYSTEM..."
    $taskName = "AnyKeyUninstall_$(Get-Random)"
    $action   = New-ScheduledTaskAction -Execute 'powershell.exe' `
                  -Argument "-ExecutionPolicy Bypass -NoProfile -File `"$scriptPath`"" `
                  -WorkingDirectory $scriptDir
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask -TaskName $taskName -Action $action -Principal $principal -Force | Out-Null
    Start-ScheduledTask -TaskName $taskName
    for ($i = 0; $i -lt 30; $i++) { Start-Sleep 1; if ((Get-ScheduledTask -TaskName $taskName).State -ne 'Running') { break } }
    $info = Get-ScheduledTaskInfo -TaskName $taskName
    Unregister-ScheduledTask -TaskName $taskName -Confirm:$false
    Write-Host ""
    if (Test-Path $logFile) { Get-Content $logFile | ForEach-Object { Write-Host $_ } }
    Write-Host ""
    if ($info.LastTaskResult -eq 0) { Write-Host "UNINSTALL SUCCESS. Reboot to complete." }
    else { Write-Host "UNINSTALL FAILED (LastResult=0x$('{0:X}' -f $info.LastTaskResult))." }
    exit $info.LastTaskResult
}

Start-Transcript -Path $logFile -Force | Out-Null
try {
    Write-Host "Running as SYSTEM. scriptDir=$scriptDir"

    # 1. Remove UpperFilters from all keyboard AND mouse devices
    $cleaned = 0
    $deviceClasses = @('Keyboard', 'Mouse')
    foreach ($class in $deviceClasses) {
        $devices = Get-PnpDevice -Class $class -ErrorAction SilentlyContinue
        if (-not $devices) { Write-Host "  No $class devices found"; continue }
        foreach ($dev in $devices) {
            $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$($dev.InstanceId)"
            if (-not (Test-Path $key)) { continue }
            $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
            if (-not $cur -or $cur -notcontains $svc) { continue }
            $remaining = @($cur | Where-Object { $_ -ne $svc })
            if ($remaining.Count -eq 0) {
                Remove-ItemProperty -Path $key -Name UpperFilters -Force -ErrorAction SilentlyContinue
            } else {
                New-ItemProperty -Path $key -Name UpperFilters -Value $remaining -PropertyType MultiString -Force | Out-Null
            }
            Write-Host "  removed $svc from $($dev.InstanceId) ($class)"
            $cleaned++
        }
    }
    Write-Host "Cleaned UpperFilters from $cleaned device(s)"

    # 2. Stop + delete service (use cmd /c to avoid PowerShell sc alias)
    $svcObj = Get-Service $svc -ErrorAction SilentlyContinue
    if ($svcObj) {
        if ($svcObj.Status -ne 'Stopped') {
            cmd /c "sc.exe stop $svc 2>&1" | Out-Null
            Write-Host "Service $svc stopped"
        }
        cmd /c "sc.exe delete $svc 2>&1" | Out-Null
        Write-Host "Service $svc deleted"
    } else {
        Write-Host "Service $svc not found (already removed)"
    }

    # 3. Remove .sys
    if (Test-Path $dstSys) {
        try {
            Remove-Item $dstSys -Force -ErrorAction Stop
            Write-Host "Removed $dstSys"
        } catch {
            # File in use: schedule delete on next reboot
            Move-Item $dstSys "$dstSys.delete_me" -Force -ErrorAction SilentlyContinue
            cmd /c "move /Y `"$dstSys.delete_me`" `"$dstSys.old`"" 2>&1 | Out-Null
            Write-Host "WARN: $dstSys in use - rename to .old; will be gone after reboot"
        }
    }

    # 4. Remove INF from driver store (precise: enumerate and match by original INF name)
    $infNames = @('anykey_flt.inf', 'anykey_flt_mouse.inf')
    $enum = cmd /c "pnputil /enum-drivers 2>&1"
    $published = $null
    foreach ($line in $enum) {
        if ($line -match 'Published Name:\s*(\S+\.inf)') {
            $published = $Matches[1]
        } elseif ($published -and $line -match 'Original Name:\s*(\S+\.inf)') {
            if ($infNames -contains $Matches[1]) {
                $result = cmd /c "pnputil /delete-driver $published 2>&1"
                if ($result -match 'deleted successfully') {
                    Write-Host "Removed $published ($($Matches[1])) from driver store"
                } else {
                    Write-Host "WARN: failed to remove $published ($($Matches[1]))"
                }
            }
            $published = $null
        }
    }

    # 5. Verify (both keyboard and mouse)
    $remainingDevs = 0
    foreach ($class in @('Keyboard', 'Mouse')) {
        $devices = Get-PnpDevice -Class $class -ErrorAction SilentlyContinue
        if (-not $devices) { continue }
        foreach ($dev in $devices) {
            $key = "HKLM:\SYSTEM\CurrentControlSet\Enum\$($dev.InstanceId)"
            $cur = (Get-ItemProperty -Path $key -Name UpperFilters -ErrorAction SilentlyContinue).UpperFilters
            if ($cur -and ($cur -contains $svc)) { $remainingDevs++ }
        }
    }
    if ($remainingDevs -gt 0) {
        Write-Host "WARN: $remainingDevs device(s) still reference $svc in UpperFilters"
    } else {
        Write-Host "VERIFY: no devices reference $svc - clean"
    }

    Write-Host "UNINSTALL COMPLETE. Reboot to finalize."
    exit 0
} catch {
    Write-Host "ERROR: $_"
    Write-Host $_.ScriptStackTrace
    exit 1
} finally {
    Stop-Transcript | Out-Null
}
