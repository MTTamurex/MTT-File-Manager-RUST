param(
    [string]$CompilerPath,
    [string]$WorkDirectory = (Join-Path ([System.IO.Path]::GetTempPath()) (
        'opencode\win-e-restore-' + [guid]::NewGuid().ToString('N')))
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $CompilerPath) {
    $compiler = Get-Command ISCC.exe -ErrorAction SilentlyContinue
    if ($compiler) {
        $CompilerPath = $compiler.Source
    } else {
        $candidates = @(
            (Join-Path $env:LOCALAPPDATA 'Programs\Inno Setup 6\ISCC.exe'),
            (Join-Path $env:ProgramFiles 'Inno Setup 6\ISCC.exe'),
            (Join-Path ${env:ProgramFiles(x86)} 'Inno Setup 6\ISCC.exe')
        )
        $CompilerPath = $candidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
    }
}
if (-not $CompilerPath) {
    throw 'Inno Setup 6 compiler not found; supply -CompilerPath.'
}

# Use a fresh directory: an old success marker must never mask a failed run.
if (Test-Path -LiteralPath $WorkDirectory) {
    throw 'WorkDirectory must not already exist.'
}
[void](New-Item -ItemType Directory -Path $WorkDirectory)

foreach ($fixture in @('win_e_restore_compile.iss', 'win_e_restore_tests.iss')) {
    & $CompilerPath /Qp "/O$WorkDirectory" (Join-Path $PSScriptRoot $fixture)
    if ($LASTEXITCODE -ne 0) {
        throw "Inno Setup compilation failed for $fixture."
    }
}
& $CompilerPath /Qp "/O$WorkDirectory" /DWinERestore64 /Fwin-e-restore-compile64 `
    (Join-Path $PSScriptRoot 'win_e_restore_compile.iss')
if ($LASTEXITCODE -ne 0) {
    throw 'Inno Setup compilation failed for the 64-bit registry view fixture.'
}

foreach ($test in @(
    @{ Name = 'win-e-restore-compile'; Expected = 'native adapter read checks passed' },
    @{ Name = 'win-e-restore-compile64'; Expected = 'native adapter read checks passed' },
    @{ Name = 'win-e-restore-tests'; Expected = '28 cases passed' }
)) {
    $resultPath = Join-Path $WorkDirectory ($test.Name + '.txt')
    $logPath = Join-Path $WorkDirectory ($test.Name + '.log')
    $process = Start-Process -FilePath (Join-Path $WorkDirectory ($test.Name + '.exe')) `
        -ArgumentList @('/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/SP-',
            ('/TestResult="' + $resultPath + '"'), ('/LOG="' + $logPath + '"')) `
        -Wait -PassThru
    # InitializeSetup returns False intentionally to prevent installation (exit 1).
    if ($process.ExitCode -ne 1 -or -not (Test-Path -LiteralPath $resultPath)) {
        throw ('Win+E test failed: ' + $test.Name + '; inspect its log in WorkDirectory.')
    }
    $result = (Get-Content -LiteralPath $resultPath -Raw).Trim()
    if ($result -ne $test.Expected) {
        throw "Unexpected regression test result: $result"
    }
    Write-Output $result
}
