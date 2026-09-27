# AnyKey - Test signing gate. Must pass BEFORE any device key is touched.
#
# WHY THIS EXISTS
#   Writing "UpperFilters = anykey_flt" onto a keyboard/mouse device key declares
#   the filter as a MEMBER of that device's driver stack. If the image cannot be
#   loaded (unsigned driver while test signing is not really in effect), the whole
#   stack fails to start and the device dies - the user is left with no keyboard
#   and no mouse, and cannot even run the uninstaller.
#
#   Reading the boot configuration is NOT proof that test signing is in effect
#   (with Secure Boot on, the value can say Yes and still be ignored), so this
#   gate also refuses to install while Secure Boot is enabled.
#
# TWO-PASS INSTALL
#   Test signing only takes effect after a reboot, so when it has to be enabled
#   we stop right here and ask the user to reboot and run the installer again.
#
# EXIT CODES
#    0  test signing ON and Secure Boot off  -> safe to install
#   10  test signing was OFF, now enabled    -> REBOOT, then run the installer again
#    1  cannot make test signing effective   -> abort, install nothing
#
# ASCII only (Windows PowerShell 5.1).

$ErrorActionPreference = 'Continue'

function Test-SecureBootEnabled {
    # $true / $false, or $null when the firmware does not report Secure Boot
    # (legacy BIOS boot).
    try { return [bool](Confirm-SecureBootUEFI) } catch { return $null }
}

function Test-TestSigningOn {
    # Read the CURRENT boot entry only. A bare "bcdedit /enum" lists every entry
    # on the machine (dual boot, leftovers), so any stale Yes would match.
    $out = cmd /c "bcdedit /enum {current} 2>&1" | Out-String
    return ($out -match 'testsigning\s+Yes')
}

$sb = Test-SecureBootEnabled
if ($sb -eq $true) {
    Write-Host "  Secure Boot is ENABLED."
    Write-Host ""
    Write-Host "  While Secure Boot is on, Windows ignores test signing, so the AnyKey"
    Write-Host "  driver could not be loaded. Turn Secure Boot off in UEFI firmware"
    Write-Host "  first (README, install section step 1), then run this installer."
    exit 1
}
if ($null -eq $sb) {
    Write-Host "  Secure Boot is not reported by this firmware (legacy BIOS) - check skipped."
} else {
    Write-Host "  Secure Boot is off."
}

if (Test-TestSigningOn) {
    Write-Host "  Test signing is ON."
    exit 0
}

Write-Host "  Test signing is OFF. Enabling it now (needs a reboot to take effect)..."
cmd /c "bcdedit /set {current} testsigning on" 2>&1 | Write-Host
if ($LASTEXITCODE -ne 0) {
    Write-Host ""
    Write-Host "  ERROR: 'bcdedit /set {current} testsigning on' failed (exit $LASTEXITCODE)."
    Write-Host "  The usual cause is Secure Boot being enabled, or a policy that"
    Write-Host "  protects the boot configuration."
    exit 1
}
Write-Host "  Test signing has been enabled in the boot configuration."
exit 10
