param(
    [Parameter(Mandatory = $true)]
    [string]$PackageName,
    [switch]$AllUsers
)

$ErrorActionPreference = 'Stop'
if ($AllUsers) {
    $packages = Get-AppxPackage -AllUsers -Name $PackageName -ErrorAction SilentlyContinue
    foreach ($package in $packages) {
        Remove-AppxPackage -Package $package.PackageFullName -AllUsers -ErrorAction Stop
    }
} else {
    $package = Get-AppxPackage -Name $PackageName -ErrorAction SilentlyContinue
    if ($package) {
        $package | Remove-AppxPackage -ErrorAction Stop
    }
}
