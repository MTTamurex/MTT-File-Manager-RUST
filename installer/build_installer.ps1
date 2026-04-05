<#
.SYNOPSIS
    Build MTT File Manager release + installer.
.DESCRIPTION
    1. Builds the Rust project in release mode
    2. Compiles the Inno Setup installer (.iss → Setup .exe)
    Requires: Inno Setup 6 (ISCC.exe in PATH or default install location)
.PARAMETER SkipBuild
    Skip the cargo build step (use existing target\release binary).
.EXAMPLE
    .\build_installer.ps1
    .\build_installer.ps1 -SkipBuild
#>
param(
    [switch]$SkipBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $RepoRoot) { $RepoRoot = $PSScriptRoot }

Write-Host "`n=== MTT File Manager Installer Build ===" -ForegroundColor Cyan

# ── Step 1: Cargo build ────────────────────────────────────────────────
if (-not $SkipBuild) {
    Write-Host "`n[1/3] Building release binary..." -ForegroundColor Yellow
    Push-Location $RepoRoot
    try {
        cargo build --release --workspace
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
} else {
    Write-Host "`n[1/3] Skipping cargo build (-SkipBuild)" -ForegroundColor DarkGray
}

# ── Step 2: Validate required files and directories ───────────────────
Write-Host "`n[2/3] Validating required files and directories..." -ForegroundColor Yellow

$requiredDirectories = @(
    "$RepoRoot\mpv_ui\portable_config\scripts",
    "$RepoRoot\mpv_ui\portable_config\script-opts"
)

$requiredFiles = @(
    "$RepoRoot\target\release\mtt-file-manager.exe",
    "$RepoRoot\target\release\mtt-search-service.exe",
    "$RepoRoot\target\release\libmpv-2.dll",
    "$RepoRoot\target\release\pdfium.dll",
    "$RepoRoot\appicon.ico",
    "$RepoRoot\mpv_ui\portable_config\mpv.conf",
    "$RepoRoot\mpv_ui\portable_config\scripts\autoload.lua",
    "$RepoRoot\mpv_ui\portable_config\scripts\modernH.lua",
    "$RepoRoot\mpv_ui\portable_config\scripts\vsr.lua",
    "$RepoRoot\mpv_ui\portable_config\script-opts\osc.conf"
)

foreach ($dir in $requiredDirectories) {
    if (-not (Test-Path $dir -PathType Container)) {
        throw "Required directory not found: $dir"
    }

    $relative = $dir.Replace("$RepoRoot\", "")
    Write-Host "  OK  $relative/" -ForegroundColor Green
}

foreach ($file in $requiredFiles) {
    if (-not (Test-Path $file -PathType Leaf)) {
        throw "Required file not found: $file"
    }
    $size = (Get-Item $file).Length
    $relative = $file.Replace("$RepoRoot\", "")
    Write-Host "  OK  $relative ($([math]::Round($size / 1MB, 1)) MB)" -ForegroundColor Green
}

# ── Step 3: Run Inno Setup compiler ──────────────────────────────────
Write-Host "`n[3/3] Compiling installer..." -ForegroundColor Yellow

$isccFromPath = Get-Command "ISCC.exe" -ErrorAction SilentlyContinue
$isccCandidates = @(
    $(if ($isccFromPath) { $isccFromPath.Source }),
    "${env:ProgramFiles(x86)}\Inno Setup 6\ISCC.exe",
    "${env:ProgramFiles}\Inno Setup 6\ISCC.exe",
    "${env:LOCALAPPDATA}\Programs\Inno Setup 6\ISCC.exe"
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1

if (-not $isccCandidates) {
    throw @"
Inno Setup 6 (ISCC.exe) not found.
Install it via:  winget install JRSoftware.InnoSetup
Or download:     https://jrsoftware.org/isdl.php
"@
}

Write-Host "  Using: $isccCandidates" -ForegroundColor DarkGray

$issFile = "$RepoRoot\installer\setup.iss"
& $isccCandidates $issFile
if ($LASTEXITCODE -ne 0) { throw "ISCC.exe failed (exit code $LASTEXITCODE)" }

# ── Done ──────────────────────────────────────────────────────────────
$outputDir = "$RepoRoot\installer\output"
$installer = Get-ChildItem "$outputDir\*.exe" | Sort-Object LastWriteTime -Descending | Select-Object -First 1

Write-Host "`n=== Build complete ===" -ForegroundColor Green
Write-Host "Installer: $($installer.FullName)" -ForegroundColor Cyan
Write-Host "Size:      $([math]::Round($installer.Length / 1MB, 1)) MB" -ForegroundColor Cyan
