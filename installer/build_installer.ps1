<#
.SYNOPSIS
    Build MTT File Manager release + installer.
.DESCRIPTION
    1. Builds the Rust project in release mode
    2. Compiles the Inno Setup installer (.iss → Setup .exe)
    Requires: Inno Setup 6 (ISCC.exe in PATH or default install location)
.PARAMETER SkipBuild
    Reuse a verified build of the selected variant from its dedicated target directory.
.PARAMETER Variant
    Standard (default) checks for updates and opens the release page in a browser.
    NoUpdate excludes update checks; Both builds each edition.
.EXAMPLE
    .\build_installer.ps1
    .\build_installer.ps1 -SkipBuild
    .\build_installer.ps1 -Variant NoUpdate
    .\build_installer.ps1 -Variant Both
#>
param(
    [switch]$SkipBuild,
    [ValidateSet('Standard', 'NoUpdate', 'Both')]
    [string]$Variant = 'Standard'
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$RepoRoot = Split-Path -Parent $PSScriptRoot
if (-not $RepoRoot) { $RepoRoot = $PSScriptRoot }

if ($Variant -eq 'Both') {
    & $PSCommandPath -Variant Standard -SkipBuild:$SkipBuild
    & $PSCommandPath -Variant NoUpdate -SkipBuild:$SkipBuild
    return
}

if ($SkipBuild -and $env:MTT_MSIX_SIGNING_PFX -and $env:MTT_MSIX_PUBLISHER) {
    throw 'SkipBuild cannot be used when creating a signed sparse package; rebuild the executable with the configured publisher.'
}

$variantDirectory = if ($Variant -eq 'NoUpdate') { 'installer-no-update' } else { 'installer-browser-updates' }
$buildTargetDir = Join-Path $RepoRoot "target\$variantDirectory"
$buildOutputDir = Join-Path $buildTargetDir 'release'
$buildManifestPath = Join-Path $buildOutputDir 'installer-build.json'
$binaries = @('mtt-file-manager.exe', 'mtt-search-service.exe', 'mtt_explorer_command.dll')

Write-Host "`n=== MTT File Manager Installer Build ===" -ForegroundColor Cyan
Write-Host "Variant: $Variant" -ForegroundColor Cyan

# ── Step 1: Cargo build ────────────────────────────────────────────────
if (-not $SkipBuild) {
    Write-Host "`n[1/3] Building release binary..." -ForegroundColor Yellow
    Push-Location $RepoRoot
    try {
        $cargoArguments = @('build', '--release', '--workspace', '--locked', '--target-dir', $buildTargetDir)
        if ($Variant -eq 'NoUpdate') {
            $cargoArguments += @('--no-default-features', '--features', 'notify-watcher')
        }
        cargo @cargoArguments
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed (exit code $LASTEXITCODE)" }
    } finally {
        Pop-Location
    }
    $hashes = @{}
    foreach ($binary in $binaries) {
        $hashes[$binary] = (Get-FileHash -LiteralPath (Join-Path $buildOutputDir $binary) -Algorithm SHA256).Hash
    }
    @{ Variant = $Variant; Binaries = $hashes } | ConvertTo-Json | Set-Content -LiteralPath $buildManifestPath -Encoding UTF8
} else {
    if (-not (Test-Path -LiteralPath $buildManifestPath -PathType Leaf)) {
        throw 'No verified build exists for this variant. Run without SkipBuild first.'
    }
    $manifest = Get-Content -LiteralPath $buildManifestPath -Raw | ConvertFrom-Json
    if ($manifest.Variant -ne $Variant) { throw 'The existing build is for a different variant.' }
    foreach ($binary in $binaries) {
        $actual = (Get-FileHash -LiteralPath (Join-Path $buildOutputDir $binary) -Algorithm SHA256).Hash
        if ($manifest.Binaries.$binary -ne $actual) { throw "The existing build changed: $binary. Run without SkipBuild." }
    }
    Write-Host "`n[1/3] Skipping cargo build (-SkipBuild)" -ForegroundColor DarkGray
}

# Runtime DLLs are shared inputs, but each variant packages its own staged copy.
foreach ($runtime in @('pdfium.dll', 'libmpv-2.dll')) {
    $source = Join-Path $RepoRoot "target\release\$runtime"
    $destination = Join-Path $buildOutputDir $runtime
    if (-not (Test-Path -LiteralPath $destination -PathType Leaf)) {
        Copy-Item -LiteralPath $source -Destination $destination
    }
}

$modernMenuPackageReady = $false
$modernMenuPackagePath = Join-Path $buildOutputDir 'mtt-file-manager.identity.msix'
$packageBuildResult = & "$RepoRoot\installer\build_sparse_package.ps1" -OutputPath $modernMenuPackagePath
if ($packageBuildResult -eq 0 -and (Test-Path -LiteralPath $modernMenuPackagePath -PathType Leaf)) {
    $modernMenuPackageReady = $true
} elseif ($packageBuildResult -eq 10) {
    Write-Host "Modern Explorer menu is not signed; the installer will use the classic context-menu fallback." -ForegroundColor Yellow
} else {
    throw "Sparse package build returned an unexpected status: $packageBuildResult"
}

# ── Step 2: Validate required files and directories ───────────────────
Write-Host "`n[2/3] Validating required files and directories..." -ForegroundColor Yellow

$requiredDirectories = @(
    "$RepoRoot\mpv_ui\portable_config\scripts",
    "$RepoRoot\mpv_ui\portable_config\script-opts",
    "$RepoRoot\mpv_ui\portable_config\fonts",
    "$RepoRoot\installer\sparse",
    "$RepoRoot\third_party_licenses",
    "$RepoRoot\third_party_licenses\pdfium-win-x64",
    "$RepoRoot\third_party_licenses\pdfium-win-x64\licenses"
)

$requiredFiles = @(
    "$buildOutputDir\mtt-file-manager.exe",
    "$buildOutputDir\mtt_explorer_command.dll",
    "$buildOutputDir\mtt-search-service.exe",
    "$buildOutputDir\libmpv-2.dll",
    "$buildOutputDir\pdfium.dll",
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
    "$RepoRoot\third_party_licenses\BETTEROFFICE-NOTICE.txt",
    "$RepoRoot\third_party_licenses\YRS-LICENSE.txt",
    "$RepoRoot\third_party_licenses\DASHMAP-LICENSE.txt",
    "$RepoRoot\third_party_licenses\QUICK-XML-LICENSE.txt",
    "$RepoRoot\third_party_licenses\ZIP-8-LICENSE.txt",
    "$RepoRoot\third_party_licenses\TINY-SKIA-LICENSE.txt",
    "$RepoRoot\third_party_licenses\ZLIB-RS-LICENSE.txt",
    "$RepoRoot\third_party_licenses\MATERIAL-DESIGN-ICONIC-FONT-NOTICE.txt",
    "$RepoRoot\appicon.ico",
    "$RepoRoot\appicon.png",
    "$RepoRoot\installer\build_sparse_package.ps1",
    "$RepoRoot\installer\register_sparse_package.ps1",
    "$RepoRoot\installer\unregister_sparse_package.ps1",
    "$RepoRoot\installer\cleanup_context_menu.ps1",
    "$RepoRoot\installer\restore_folder_integration.ps1",
    "$RepoRoot\installer\sparse\AppxManifest.xml",
    "$RepoRoot\mpv_ui\portable_config\mpv.conf",
    "$RepoRoot\mpv_ui\portable_config\input.conf",
    "$RepoRoot\mpv_ui\portable_config\scripts\autoload.lua",
    "$RepoRoot\mpv_ui\portable_config\scripts\modernH.lua",
    "$RepoRoot\mpv_ui\portable_config\scripts\vsr.lua",
    "$RepoRoot\mpv_ui\portable_config\script-opts\osc.conf",
    "$RepoRoot\mpv_ui\portable_config\fonts\Material-Design-Iconic-Font.ttf",
    "$RepoRoot\installer\languages\ChineseSimplified.isl",
    "$RepoRoot\installer\languages\Russian.isl",
    "$RepoRoot\installer\languages\Greek.isl"
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
    "$buildOutputDir\pdfium.dll"   = "7167AEE6BB3D2724EE62FD83BBEB8883EDC786A6E1999782857D4952536A0ED3"
    "$buildOutputDir\libmpv-2.dll"  = "AE7B7B6B3ECCC7AEEFF3CBEF30FD175F97B27F1B815F89D0BD0CCA926C8C2D99"
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
$compilerArguments = @("/DBuildOutputDir=$buildOutputDir")
if ($modernMenuPackageReady) { $compilerArguments += '/DModernMenuPackageEnabled=1' }
if ($Variant -eq 'NoUpdate') { $compilerArguments += '/DNoOnlineUpdates=1' }
& $isccCandidates @compilerArguments $issFile
if ($LASTEXITCODE -ne 0) { throw "ISCC.exe failed (exit code $LASTEXITCODE)" }

# ── Done ──────────────────────────────────────────────────────────────
$outputDir = "$RepoRoot\installer\output"
$versionLine = Select-String -LiteralPath $issFile -Pattern '^#define MyAppVersion\s+"([^"]+)"'
$version = $versionLine.Matches[0].Groups[1].Value
$suffix = if ($Variant -eq 'NoUpdate') { '-no-update' } else { '' }
$installer = Get-Item -LiteralPath (Join-Path $outputDir "MTT-File-Manager-Setup-$version$suffix.exe")

Write-Host "`n=== Build complete ===" -ForegroundColor Green
Write-Host "Installer: $($installer.FullName)" -ForegroundColor Cyan
Write-Host "Size:      $([math]::Round($installer.Length / 1MB, 1)) MB" -ForegroundColor Cyan
