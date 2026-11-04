param(
    [string]$CompilerPath = '',
    [string]$WorkDirectory = (Join-Path $env:LOCALAPPDATA (
        'Temp\opencode\retired-cleanup-' + [guid]::NewGuid().ToString('N')))
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$installerRoot = Split-Path -Parent $PSScriptRoot
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

function New-Fixture([string]$Name, [string]$Version, [string]$DeleteSection, [string]$RunEntry) {
    # No application registration, elevation, services, shortcuts, or MTT code.
    $source = @"
[Setup]
AppId=RetiredCleanupFixture
AppName=Retired cleanup fixture
AppVersion=$Version
DefaultDirName={src}\app
PrivilegesRequired=lowest
CreateUninstallRegKey=no
UninstallLogMode=append
UsePreviousAppDir=no
UsePreviousLanguage=no
DisableDirPage=yes
DisableProgramGroupPage=yes
OutputBaseFilename=$Name
Compression=none

$DeleteSection

[Files]
Source: "fixture.txt"; DestDir: "{app}"; DestName: "keep.txt"; Flags: ignoreversion

[UninstallRun]
$RunEntry
"@
    $path = Join-Path $WorkDirectory "$Name.iss"
    Set-Content -LiteralPath $path -Value $source -Encoding UTF8
    $output = & $CompilerPath '/Q' "/O$WorkDirectory" $path 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Fixture compilation failed: $($output -join [Environment]::NewLine)" }
    return (Join-Path $WorkDirectory "$Name.exe")
}

function Invoke-Fixture([string]$Executable, [string]$Directory, [string]$LogName, [switch]$Uninstall) {
    $log = Join-Path $WorkDirectory "$LogName.log"
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-', "/LOG=`"$log`"")
    if (-not $Uninstall) { $arguments += "/DIR=`"$Directory`"" }
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Fixture failed: $LogName (exit $($process.ExitCode)); see $log" }
    return $log
}

try {
    $setup = Get-Content -LiteralPath (Join-Path $installerRoot 'setup.iss') -Raw
    $deleteSection = [regex]::Match(
        $setup, '(?ms)^\[InstallDelete\]\s*\r?\n.*?(?=^\[|\z)'
    ).Value
    if (-not $deleteSection) { throw 'InstallDelete section not found.' }
    $override = [regex]::Match(
        $setup, '(?m)^Filename:.*RunOnceId: "UnregisterMttSparsePackage".*$'
    ).Value.Trim()
    if (-not $override) { throw 'Retired uninstall command override not found.' }

    Set-Content -LiteralPath (Join-Path $WorkDirectory 'fixture.txt') -Value 'keep' -Encoding ASCII
    $oldEntry = 'Filename: "{sys}\cmd.exe"; Parameters: "/d /c echo obsolete-command-ran>""{app}\obsolete-command.txt"""; RunOnceId: "UnregisterMttSparsePackage"; Flags: runhidden waituntilterminated'
    $old = New-Fixture 'old' '1.0' '' $oldEntry
    $current = New-Fixture 'current' '2.0' $deleteSection $override

    # Fresh installation: all retired files are absent, including after retry.
    $fresh = Join-Path $WorkDirectory 'fresh'
    $null = Invoke-Fixture $current $fresh 'fresh'
    $null = Invoke-Fixture $current $fresh 'fresh-repeat'
    if ((Get-Content -LiteralPath (Join-Path $fresh 'keep.txt') -Raw).Trim() -ne 'keep') {
        throw 'Fresh installation did not preserve its application file.'
    }
    $null = Invoke-Fixture (Join-Path $fresh 'unins000.exe') $fresh 'fresh-uninstall' -Uninstall
    Write-Output 'Absent retired files do not block installation, repetition, or uninstallation.'

    # Upgrade: preserve the older uninstall log while replacing its command.
    $upgrade = Join-Path $WorkDirectory 'upgrade'
    $null = Invoke-Fixture $old $upgrade 'old-install'
    $retired = @('mtt-updater.exe', 'mtt_explorer_command.dll',
        'mtt-file-manager.identity.msix', 'unregister_sparse_package.ps1')
    foreach ($name in $retired) {
        Set-Content -LiteralPath (Join-Path $upgrade $name) -Value 'obsolete fixture' -Encoding ASCII
    }
    $null = Invoke-Fixture $current $upgrade 'upgrade'
    foreach ($name in $retired) {
        if (Test-Path -LiteralPath (Join-Path $upgrade $name)) {
            throw "Retired file survived upgrade: $name"
        }
    }
    if ((Get-Content -LiteralPath (Join-Path $upgrade 'keep.txt') -Raw).Trim() -ne 'keep') {
        throw 'Upgrade changed an unrelated application file.'
    }
    $log = Invoke-Fixture (Join-Path $upgrade 'unins000.exe') $upgrade 'upgrade-uninstall' -Uninstall
    if (Test-Path -LiteralPath (Join-Path $upgrade 'obsolete-command.txt')) {
        throw 'Uninstallation executed the obsolete command from an older log.'
    }
    if ((Get-Content -LiteralPath $log -Raw) -notmatch 'Skipping RunOnceId "UnregisterMttSparsePackage"') {
        throw 'Uninstallation did not demonstrate suppression of the older command.'
    }
    Write-Output 'Upgrade removes only retired files and suppresses the obsolete uninstall command.'
} finally {
    Remove-Item -LiteralPath $WorkDirectory -Recurse -Force
}
