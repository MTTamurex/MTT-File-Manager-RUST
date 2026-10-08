param(
    [Parameter(Mandatory = $true)]
    [string]$OutputPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$certificatePath = $env:MTT_MSIX_SIGNING_PFX
$certificatePassword = $env:MTT_MSIX_SIGNING_PASSWORD
$publisher = $env:MTT_MSIX_PUBLISHER
if (-not $certificatePath -or -not $certificatePassword -or -not $publisher) {
    Write-Warning 'A trusted MSIX signing certificate, password, and matching publisher are required. The installer will use the classic context-menu fallback.'
    return 10
}
if (-not (Test-Path -LiteralPath $certificatePath -PathType Leaf)) {
    throw 'The MSIX signing certificate file was not found.'
}

function Find-WindowsSdkTool([string]$Name) {
    $fromPath = Get-Command $Name -ErrorAction SilentlyContinue
    if ($fromPath) { return $fromPath.Source }

    $sdkRoot = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\bin'
    if (-not (Test-Path -LiteralPath $sdkRoot -PathType Container)) { return $null }
    $versions = Get-ChildItem -LiteralPath $sdkRoot -Directory -ErrorAction SilentlyContinue |
        Sort-Object Name -Descending
    foreach ($version in $versions) {
        $candidate = Join-Path $version.FullName (Join-Path 'x64' $Name)
        if (Test-Path -LiteralPath $candidate -PathType Leaf) { return $candidate }
    }
    return $null
}

$makeAppx = Find-WindowsSdkTool 'MakeAppx.exe'
$signTool = Find-WindowsSdkTool 'SignTool.exe'
if (-not $makeAppx -or -not $signTool) {
    throw 'The Windows SDK MakeAppx.exe and SignTool.exe are required when MSIX signing is configured.'
}

$securePassword = ConvertTo-SecureString $certificatePassword -AsPlainText -Force
$pfxData = Get-PfxData -FilePath $certificatePath -Password $securePassword
$publisherCertificate = $pfxData.EndEntityCertificates |
    Where-Object { $_.Subject -ceq $publisher } |
    Select-Object -First 1
if (-not $publisherCertificate) {
    throw 'MTT_MSIX_PUBLISHER must exactly match the signing certificate subject.'
}

$stageDirectory = Join-Path $env:TEMP ('mtt-sparse-' + [guid]::NewGuid().ToString('N'))
$certificateImportedByScript = $false
$importedCertificateThumbprint = $publisherCertificate.Thumbprint
$signingCertificate = $null
try {
    New-Item -ItemType Directory -Path $stageDirectory -Force | Out-Null
    $manifestPath = Join-Path $stageDirectory 'AppxManifest.xml'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'sparse\AppxManifest.xml') -Destination $manifestPath

    $manifest = New-Object System.Xml.XmlDocument
    $manifest.PreserveWhitespace = $true
    $manifest.Load($manifestPath)
    $namespaces = New-Object System.Xml.XmlNamespaceManager($manifest.NameTable)
    $namespaces.AddNamespace('appx', 'http://schemas.microsoft.com/appx/manifest/foundation/windows10')
    $identity = $manifest.SelectSingleNode('/appx:Package/appx:Identity', $namespaces)
    if (-not $identity) { throw 'The sparse package manifest has no Identity element.' }

    Push-Location (Split-Path -Parent $PSScriptRoot)
    try {
        $metadata = cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
        if ($LASTEXITCODE -ne 0) { throw 'cargo metadata failed while reading the app version.' }
    } finally {
        Pop-Location
    }
    $appVersion = ($metadata.packages | Where-Object { $_.name -eq 'mtt-file-manager' } | Select-Object -First 1).version
    if (-not $appVersion) { throw 'Could not determine the MTT package version.' }
    $identity.SetAttribute('Version', "$appVersion.0")
    $identity.SetAttribute('Publisher', $publisher)
    $manifest.Save($manifestPath)

    $outputDirectory = Split-Path -Parent $OutputPath
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
    & $makeAppx pack /o /d $stageDirectory /nv /p $OutputPath | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "MakeAppx failed with exit code $LASTEXITCODE." }

    $existingCertificate = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Thumbprint -eq $publisherCertificate.Thumbprint } |
        Select-Object -First 1
    if (-not $existingCertificate) {
        Import-PfxCertificate -FilePath $certificatePath -CertStoreLocation Cert:\CurrentUser\My -Password $securePassword | Out-Null
        $certificateImportedByScript = $true
    }
    $signingCertificate = Get-ChildItem Cert:\CurrentUser\My |
        Where-Object { $_.Thumbprint -eq $publisherCertificate.Thumbprint -and $_.HasPrivateKey } |
        Select-Object -First 1
    if (-not $signingCertificate) { throw 'The imported signing certificate has no private key.' }

    & $signTool sign /fd SHA256 /sha1 $signingCertificate.Thumbprint /s My $OutputPath | Out-Host
    if ($LASTEXITCODE -ne 0) { throw "SignTool failed with exit code $LASTEXITCODE." }
    return 0
} finally {
    if ($certificateImportedByScript -and $importedCertificateThumbprint) {
        Remove-Item -LiteralPath ("Cert:\CurrentUser\My\" + $importedCertificateThumbprint) -Force -ErrorAction SilentlyContinue
    }
    Remove-Item -LiteralPath $stageDirectory -Recurse -Force -ErrorAction SilentlyContinue
}
