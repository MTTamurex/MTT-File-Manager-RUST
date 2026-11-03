param(
    [switch]$NoBuild
)

$ErrorActionPreference = "Stop"

# Historical behavior: -NoBuild skips the full service suite, not compilation.
$steps = @()
if (-not $NoBuild) {
    $steps += ,@("test", "-p", "mtt-search-service", "--locked")
}
$steps += ,@("test", "-p", "mtt-search-protocol", "--locked")
$steps += ,@("test", "-p", "mtt-search-service", "--locked", "ipc_server::")
$steps += ,@("test", "-p", "mtt-search-service", "--locked", "ipc_authorization::")
$steps += ,@("test", "-p", "mtt-search-service", "--locked", "security_policy::tests")

# Only ordinary Cargo commands are invoked; no encoded or nested PowerShell.
Push-Location $PSScriptRoot
try {
    Write-Host "=== MTT Security Verification Suite ===" -ForegroundColor Cyan
    for ($index = 0; $index -lt $steps.Count; $index++) {
        $cargoArgs = $steps[$index]
        Write-Host "[$($index + 1)/$($steps.Count)] cargo $($cargoArgs -join ' ')" -ForegroundColor Yellow
        & cargo @cargoArgs
        if ($LASTEXITCODE -ne 0) {
            throw "Cargo failed (exit code $LASTEXITCODE): $($cargoArgs -join ' ')"
        }
    }
    Write-Host "Selected package tests passed. Installed SCM/LocalSystem compatibility is not validated by this suite." -ForegroundColor Green
} finally {
    Pop-Location
}
