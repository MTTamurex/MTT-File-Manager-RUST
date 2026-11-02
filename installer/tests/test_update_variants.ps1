param(
    [string]$CompilerPath = '',
    [string]$WorkDirectory = (Join-Path $env:LOCALAPPDATA (
        'Temp\opencode\update-variants-' + [guid]::NewGuid().ToString('N')))
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$installerRoot = Split-Path -Parent $PSScriptRoot
$repoRoot = Split-Path -Parent $installerRoot
if (-not $CompilerPath) {
    $fromPath = Get-Command 'ISCC.exe' -ErrorAction SilentlyContinue
    $CompilerPath = @(
        $(if ($fromPath) { $fromPath.Source }),
        "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
        "$env:ProgramFiles\Inno Setup 6\ISCC.exe",
        "$env:LOCALAPPDATA\Programs\Inno Setup 6\ISCC.exe"
    ) | Where-Object { $_ -and (Test-Path -LiteralPath $_) } | Select-Object -First 1
}
if (-not $CompilerPath) { throw 'Inno Setup compiler not found.' }
if (Test-Path -LiteralPath $WorkDirectory) { throw 'WorkDirectory must not already exist.' }
[void](New-Item -ItemType Directory -Path $WorkDirectory)

function Invoke-CompileCase([string]$Name, [string[]]$Defines) {
    $arguments = @('/O-', "/DBuildOutputDir=$WorkDirectory") + $Defines
    $previousPreference = $ErrorActionPreference
    try {
        # Windows PowerShell wraps expected native stderr as ErrorRecords.
        $ErrorActionPreference = 'Continue'
        $output = & $CompilerPath @arguments (Join-Path $installerRoot 'setup.iss') 2>&1
        $exitCode = $LASTEXITCODE
    } finally {
        $ErrorActionPreference = $previousPreference
    }
    if ($exitCode -ne 0) {
        throw "$Name failed: $($output -join [Environment]::NewLine)"
    }
    Write-Output "$Name passed."
}

try {
    # Inno Setup validates these file inputs but output/installation is disabled.
    foreach ($name in @('mtt-file-manager.exe', 'mtt-search-service.exe', 'pdfium.dll', 'libmpv-2.dll')) {
        Set-Content -LiteralPath (Join-Path $WorkDirectory $name) -Value 'fixture'
    }
    Invoke-CompileCase 'No-update installer excludes a missing helper' @('/DNoOnlineUpdates=1')
    Invoke-CompileCase 'Browser-update installer excludes a missing helper' @()
    $installerSource = Get-Content -LiteralPath (Join-Path $installerRoot 'setup.iss') -Raw
    if ($installerSource -match '(?im)^Source:.*mtt-updater\.exe') {
        throw 'The browser-update installer must not package an update helper.'
    }

    Push-Location $repoRoot
    try {
        $json = cargo metadata --format-version 1 --no-deps --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed.' }
        $package = ($json | ConvertFrom-Json).packages | Where-Object name -eq 'mtt-file-manager'
        if ($package.features.default -notcontains 'online-updates') { throw 'Standard build must enable online-updates.' }
        $helper = $package.targets | Where-Object name -eq 'mtt-updater'
        if ($helper) { throw 'The browser-update build must not define an updater binary.' }
        foreach ($name in @('reqwest', 'semver', 'egui_commonmark')) {
            $dependency = $package.dependencies | Where-Object name -eq $name
            if (-not $dependency.optional) { throw "$name must be optional." }
        }
        Write-Output 'Cargo feature and browser-only update configuration passed.'

        $tree = cargo tree -p mtt-file-manager --depth 1 --prefix none --edges normal --locked --no-default-features --features notify-watcher
        if ($LASTEXITCODE -ne 0) { throw 'No-update dependency resolution failed.' }
        if ($tree -match '^(reqwest|semver|egui_commonmark) v') {
            throw 'No-update build still enables an update-only direct dependency.'
        }
        if (-not ($tree -match '^notify v')) { throw 'No-update build must retain filesystem watching.' }
        Write-Output 'No-update dependency selection passed.'
    } finally {
        Pop-Location
    }
} finally {
    Remove-Item -LiteralPath $WorkDirectory -Recurse -Force
}
