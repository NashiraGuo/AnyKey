$ErrorActionPreference = "Stop"
Set-Location "c:\Users\proje\WorkBuddy\20260419144520\AnyKey\anykey-engine"

# Kill stale cargo processes
Get-Process -Name cargo,rustc -ErrorAction SilentlyContinue | Stop-Process -Force

# Remove stale lock
Remove-Item -Force "target\debug\.cargo-build-lock" -ErrorAction SilentlyContinue

Write-Host "=== BUILD ==="
cargo build
if ($LASTEXITCODE -ne 0) { exit 1 }

Write-Host "=== TEST ==="
cargo test
