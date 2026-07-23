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
    "$RepoRoot\mpv_ui\portable_config\script-opts",
    "$RepoRoot\mpv_ui\portable_config\fonts",
    "$RepoRoot\third_party_licenses",
    "$RepoRoot\third_party_licenses\pdfium-win-x64",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses"
)

$requiredFiles = @(
    "$RepoRoot\target\release\mtt-file-manager.exe",
    "$RepoRoot\target\release\mtt-search-service.exe",
    "$RepoRoot\target\release\libmpv-2.dll",
    "$RepoRoot\target\release\pdfium.dll",
    "$RepoRoot\LICENSE",
    "$RepoRoot\NOTICE",
    "$RepoRoot\THIRD_PARTY_NOTICES.md",
    "$RepoRoot\third_party_licenses\README.md",
    "$RepoRoot\third_party_licenses\PROVENANCE.md",
    "$RepoRoot\third_party_licenses\GPL-2.0.txt",
    "$RepoRoot\third_party_licenses\LGPL-2.1.txt",
    "$RepoRoot\third_party_licenses\MPV-COPYRIGHT-NOTICE.txt",
    "$RepoRoot\third_party_licenses\PDFIUM-LICENSE.txt",
    "$RepoRoot\third_party_licenses\PDFIUM-BINARIES-LICENSE.txt",
    "$RepoRoot\third_party_licenses\SOURCE-AVAILABILITY.md",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\LICENSE",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\VERSION",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\args.gn",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\abseil.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\agg23.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\fast_float.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\freetype.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\icu.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\lcms.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\libjpeg_turbo.ijg",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\libjpeg_turbo.md",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\libopenjpeg.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\libpng.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\libtiff.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\llvm-libc.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\pdfium.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\simdutf.txt",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses\zlib.txt",
    "$RepoRoot\third_party_licenses\UNRAR-LICENSE.txt",
    "$RepoRoot\third_party_licenses\MATERIAL-DESIGN-ICONIC-FONT-NOTICE.txt",
    "$RepoRoot\appicon.ico",
    "$RepoRoot\mpv_ui\portable_config\mpv.conf",
    "$RepoRoot\mpv_ui\portable_config\input.conf",
    "$RepoRoot\mpv_ui\portable_config\scripts\autoload.lua",
    "$RepoRoot\mpv_ui\portable_config\scripts\modernH.lua",
    "$RepoRoot\mpv_ui\portable_config\scripts\vsr.lua",
    "$RepoRoot\mpv_ui\portable_config\script-opts\osc.conf",
    "$RepoRoot\mpv_ui\portable_config\fonts\Material-Design-Iconic-Font.ttf"
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

# SEC: Verify integrity of third-party DLLs before packaging.
# Update these hashes when upgrading the corresponding libraries.
$dllHashes = @{
    "$RepoRoot\target\release\pdfium.dll"   = "7167AEE6BB3D2724EE62FD83BBEB8883EDC786A6E1999782857D4952536A0ED3"
    "$RepoRoot\target\release\libmpv-2.dll"  = "8F77950F7D98770B1FFB1D02742C1EE5A17F9C05BCCE0723693188C69CC7C865"
}

$hashFailed = $false
foreach ($entry in $dllHashes.GetEnumerator()) {
    $actual = (Get-FileHash -Path $entry.Key -Algorithm SHA256).Hash
    $relative = $entry.Key.Replace("$RepoRoot\", "")
    if ($actual -ne $entry.Value) {
        Write-Host "  FAIL  $relative" -ForegroundColor Red
        Write-Host "        Expected: $($entry.Value)" -ForegroundColor Red
        Write-Host "        Actual:   $actual" -ForegroundColor Red
        $hashFailed = $true
    } else {
        Write-Host "  HASH  $relative OK" -ForegroundColor Green
    }
}

if ($hashFailed) {
    throw @"
DLL integrity check failed. The DLLs in target\release\ do not match the
expected hashes. If you intentionally upgraded a library, update the hashes
in build_installer.ps1.
"@
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
