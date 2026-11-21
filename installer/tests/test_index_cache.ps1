param(
    [string]$CompilerPath = '',
    [string]$WorkDirectory = (Join-Path $env:LOCALAPPDATA (
        'Temp\opencode\index-cache-' + [guid]::NewGuid().ToString('N')))
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

function Invoke-Fixture([string]$Executable, [string]$Directory, [switch]$Uninstall) {
    $arguments = @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-')
    if (-not $Uninstall) { $arguments += "/DIR=`"$Directory`"" }
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) { throw "Fixture failed with exit code $($process.ExitCode)." }
}

try {
    $source = Get-Content -LiteralPath (Join-Path $installerRoot 'setup.iss') -Raw
    $installHandler = [regex]::Match(
        $source, '(?ms)^procedure CurStepChanged.*?(?=^procedure CurUninstallStepChanged)'
    ).Value
    if (-not $installHandler) { throw 'Install-step handler not found.' }
    if ($installHandler -match '\bDeleteProgramDataCache\b') {
        throw 'Install and upgrade must preserve the existing search index cache.'
    }
    $uninstallHandler = [regex]::Match(
        $source, '(?ms)^procedure CurUninstallStepChanged.*?(?=^\[CustomMessages\])'
    ).Value
    if ($uninstallHandler -notmatch '(?s)CurUninstallStep = usPostUninstall.*DeleteProgramDataCache') {
        throw 'Complete uninstallation must continue removing the search index cache.'
    }
    $cleanup = [regex]::Match(
        $source, '(?ms)^function DeleteProgramDataCache.*?(?=^function IsSecureInstallDirectory)'
    ).Value
    if (-not $cleanup) { throw 'Cache cleanup function not found.' }

    # Exercise the production install-stage body and cleanup function, but never
    # access product services, registry keys, installation folders, or real cache.
    $installStage = $installHandler.Substring(
        0, $installHandler.IndexOf('  // Persist the language')
    ) + 'end;'
    $cleanup = $cleanup.Replace('ProgramDataCacheDir', "ExpandConstant('{app}\cache')")
    $fixture = @"
[Setup]
AppId=IndexCacheFixture
AppName=Index cache fixture
AppVersion=1.0
DefaultDirName={src}\app
PrivilegesRequired=lowest
CreateUninstallRegKey=no
UsePreviousAppDir=no
UsePreviousLanguage=no
DisableDirPage=yes
DisableProgramGroupPage=yes
OutputBaseFilename=cache-fixture
Compression=none

[Files]
Source: "fixture.txt"; DestDir: "{app}"; Flags: ignoreversion

[CustomMessages]
SecureInstallDirectoryError=Unexpected fixture path failure
SearchServiceStopTimeout=Unexpected fixture stop failure

[Code]
function IsSecureInstallDirectory: Boolean;
begin
  Result := True;
end;

function StopSearchServiceIfRunning(var WasRunning: Boolean): Boolean;
begin
  WasRunning := False;
  Result := True;
end;

function StartSearchServiceAfterFailedStop: Boolean;
begin
  Result := True;
end;

$cleanup
$installStage

procedure CurUninstallStepChanged(CurStep: TUninstallStep);
begin
  if CurStep = usPostUninstall then
    if not DeleteProgramDataCache then
      RaiseException('Fixture cache cleanup failed');
end;
"@
    $fixturePath = Join-Path $WorkDirectory 'cache-fixture.iss'
    Set-Content -LiteralPath $fixturePath -Value $fixture -Encoding UTF8
    Set-Content -LiteralPath (Join-Path $WorkDirectory 'fixture.txt') -Value 'fixture'
    $output = & $CompilerPath '/Q' "/O$WorkDirectory" $fixturePath 2>&1
    if ($LASTEXITCODE -ne 0) { throw "Fixture compilation failed: $($output -join [Environment]::NewLine)" }

    $app = Join-Path $WorkDirectory 'app'
    $cache = Join-Path $app 'cache'
    $executable = Join-Path $WorkDirectory 'cache-fixture.exe'
    Invoke-Fixture $executable $app
    if (Test-Path -LiteralPath $cache) { throw 'Fresh installer unexpectedly created cache.' }
    [void](New-Item -ItemType Directory -Path $cache)
    $artifacts = @('search_index.db', 'search_index.db-wal', 'search_index.db-shm',
        'index_R.bin', 'hmac_key.bin')
    foreach ($name in $artifacts) {
        [IO.File]::WriteAllBytes((Join-Path $cache $name), [byte[]](0, 1, 127, 255))
    }
    foreach ($iteration in 1..2) {
        Invoke-Fixture $executable $app
        foreach ($name in $artifacts) {
            $bytes = [IO.File]::ReadAllBytes((Join-Path $cache $name))
            if ([Convert]::ToBase64String($bytes) -ne 'AAF//w==') {
                throw "Reinstallation changed a cache artifact: $name"
            }
        }
    }
    Invoke-Fixture (Join-Path $app 'unins000.exe') $app -Uninstall
    if (Test-Path -LiteralPath $cache) { throw 'Uninstallation retained the cache.' }
    Write-Output 'Fresh install, repeated upgrade preservation, and complete cache removal passed.'
} finally {
    Remove-Item -LiteralPath $WorkDirectory -Recurse -Force
}
