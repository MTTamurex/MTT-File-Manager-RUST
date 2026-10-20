param(
    [Parameter(Mandatory = $true)]
    [string]$ApplicationPath,
    [Parameter(Mandatory = $true)]
    [ValidatePattern('^[a-fA-F0-9]{64}$')]
    [string]$ExpectedSha256
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not (Test-Path -LiteralPath $ApplicationPath -PathType Leaf)) { exit 0 }

# Inno preserves UninstallRun entries across upgrades/downgrades. An older
# executable may interpret the maintenance flag as a request to open the UI.
# Bind this action to the exact executable that registered it, not its version.
$stream = $null
$sha256 = $null
try {
    # Keep the verified executable protected against write/delete through exit.
    $stream = [System.IO.File]::Open($ApplicationPath, [System.IO.FileMode]::Open,
        [System.IO.FileAccess]::Read, [System.IO.FileShare]::Read)
    $sha256 = [System.Security.Cryptography.SHA256]::Create()
    $actualHash = [BitConverter]::ToString($sha256.ComputeHash($stream)).Replace('-', '')
    if ($actualHash -ine $ExpectedSha256) {
        Write-Warning 'The executable changed since installation; skipping incompatible maintenance. Native installer restoration still applies.'
        exit 0
    }
    $process = Start-Process -FilePath $ApplicationPath -ArgumentList '--restore-folder-integration' `
        -WindowStyle Hidden -Wait -PassThru
    exit $process.ExitCode
} catch {
    Write-Warning 'Folder integration maintenance could not complete. Native installer restoration still applies.'
    exit 1
} finally {
    if ($sha256) { $sha256.Dispose() }
    if ($stream) { $stream.Dispose() }
}
