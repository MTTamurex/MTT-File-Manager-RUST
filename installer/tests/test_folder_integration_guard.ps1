param(
    [string]$WorkDirectory = (Join-Path $env:LOCALAPPDATA (
        'Temp\opencode\folder-integration-guard-' + [guid]::NewGuid().ToString('N')))
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if (Test-Path -LiteralPath $WorkDirectory) { throw 'WorkDirectory must not already exist.' }
[void](New-Item -ItemType Directory -Path $WorkDirectory)
$guard = Join-Path (Split-Path -Parent $PSScriptRoot) 'restore_folder_integration.ps1'
$application = Join-Path $WorkDirectory 'mtt-file-manager.exe'
$legacy = Join-Path $WorkDirectory 'legacy.exe'

# Harmless stand-ins only: neither executable references MTT or the registry.
Add-Type -OutputAssembly $application -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.IO;
public class MaintenanceFixture {
    public static int Main(string[] args) {
        if (args.Length != 1 || args[0] != "--restore-folder-integration") return 9;
        File.WriteAllText(Environment.GetEnvironmentVariable("MTT_GUARD_TEST_MARKER"), "restored");
        return 0;
    }
}
'@
Add-Type -OutputAssembly $legacy -OutputType ConsoleApplication -TypeDefinition @'
using System;
using System.IO;
public class LegacyFixture {
    public static int Main(string[] args) {
        File.WriteAllText(Environment.GetEnvironmentVariable("MTT_GUARD_TEST_MARKER"), "unwanted-ui");
        return 0;
    }
}
'@

$expectedHash = (Get-FileHash -Algorithm SHA256 -LiteralPath $application).Hash.ToLowerInvariant()
$previousMarker = $env:MTT_GUARD_TEST_MARKER
try {
    $env:MTT_GUARD_TEST_MARKER = Join-Path $WorkDirectory 'matched.txt'
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $guard `
        -ApplicationPath $application -ExpectedSha256 $expectedHash
    if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $env:MTT_GUARD_TEST_MARKER)) {
        throw 'Compatible executable did not run maintenance.'
    }
    if ((Get-Content -Raw -LiteralPath $env:MTT_GUARD_TEST_MARKER) -ne 'restored') {
        throw 'Unexpected maintenance arguments.'
    }

    # Model the actual downgrade: replace the executable but retain the earlier
    # uninstall action's hash. The old binary must never be launched.
    Copy-Item -LiteralPath $legacy -Destination $application -Force
    $env:MTT_GUARD_TEST_MARKER = Join-Path $WorkDirectory 'downgraded.txt'
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $guard `
        -ApplicationPath $application -ExpectedSha256 $expectedHash
    if ($LASTEXITCODE -ne 0 -or (Test-Path -LiteralPath $env:MTT_GUARD_TEST_MARKER)) {
        throw 'An incompatible executable was launched after downgrade.'
    }

    $env:MTT_GUARD_TEST_MARKER = Join-Path $WorkDirectory 'missing.txt'
    & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $guard `
        -ApplicationPath (Join-Path $WorkDirectory 'missing.exe') -ExpectedSha256 $expectedHash
    if ($LASTEXITCODE -ne 0 -or (Test-Path -LiteralPath $env:MTT_GUARD_TEST_MARKER)) {
        throw 'Missing executable was not skipped safely.'
    }
    Write-Output '3 folder integration guard cases passed; no application or registry settings were changed.'
} finally {
    $env:MTT_GUARD_TEST_MARKER = $previousMarker
}
