param(
    [Parameter(Mandatory = $true)]
    [string]$PackagePath,
    [Parameter(Mandatory = $true)]
    [string]$ExternalLocation,
    [Parameter(Mandatory = $true)]
    [string]$PackageName,
    [Parameter(Mandatory = $true)]
    [string]$PackageVersion
)

$ErrorActionPreference = 'Stop'
if ([Environment]::OSVersion.Version.Build -lt 22000 -or $env:PROCESSOR_ARCHITECTURE -ne 'AMD64') {
    exit 0
}

$existing = Get-AppxPackage -Name $PackageName -ErrorAction SilentlyContinue |
    Select-Object -First 1
if ($existing) {
    $existingVersion = [version]$existing.Version
    $requestedVersion = [version]$PackageVersion
    if ($existingVersion -eq $requestedVersion) { exit 0 }
    if ($existingVersion -gt $requestedVersion) {
        throw 'A newer MTT Explorer package is already registered for this account.'
    }
}

Add-AppxPackage -Path $PackagePath -ExternalLocation $ExternalLocation -ErrorAction Stop
