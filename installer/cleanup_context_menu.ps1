param(
    [Parameter(Mandatory = $true)]
    [string]$ApplicationPath
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$applicationPath = [System.IO.Path]::GetFullPath($ApplicationPath)
$expectedCommand = '"' + $applicationPath + '" --open-folder "%1"'
$menuKeyPaths = @(
    'Software\Classes\Directory\shell\MTT.FileManager.OpenInMTT',
    'Directory\shell\MTT.FileManager.OpenInMTT'
)
$stateKeyPath = 'Software\MTT-File-Manager\ShellIntegration\ContextMenu'

function Remove-RegistryKeyIfEmpty {
    param(
        [Parameter(Mandatory = $true)]
        [Microsoft.Win32.RegistryKey]$Hive,
        [Parameter(Mandatory = $true)]
        [string]$KeyPath
    )

    $key = $Hive.OpenSubKey($KeyPath, $true)
    if (-not $key) { return $false }
    try {
        if ($key.GetValueNames().Count -ne 0 -or $key.GetSubKeyNames().Count -ne 0) {
            return $false
        }
    } finally {
        $key.Dispose()
    }

    try {
        $Hive.DeleteSubKey($KeyPath, $false)
        return $true
    } catch [ArgumentException] {
        return $false
    }
}

function Remove-MttContextMenuFromHive {
    param(
        [Parameter(Mandatory = $true)]
        [Microsoft.Win32.RegistryKey]$Hive,
        [switch]$IncludeState
    )

    $changed = $false
    foreach ($menuKeyPath in $menuKeyPaths) {
        $commandKeyPath = "$menuKeyPath\command"
        $command = $null
        $commandKey = $Hive.OpenSubKey($commandKeyPath, $false)
        if ($commandKey) {
            try {
                $command = [string]$commandKey.GetValue('', $null)
            } finally {
                $commandKey.Dispose()
            }
        }

        if ([string]::Equals($command, $expectedCommand, [StringComparison]::OrdinalIgnoreCase)) {
            $commandKey = $Hive.OpenSubKey($commandKeyPath, $true)
            if ($commandKey) {
                try {
                    $commandKey.DeleteValue('', $false)
                    $changed = $true
                } finally {
                    $commandKey.Dispose()
                }
            }

            $menuKey = $Hive.OpenSubKey($menuKeyPath, $true)
            if ($menuKey) {
                try {
                    foreach ($valueName in @('', 'Icon', 'Position', 'MultiSelectModel')) {
                        if ($menuKey.GetValueNames() -contains $valueName) {
                            $menuKey.DeleteValue($valueName, $false)
                            $changed = $true
                        }
                    }
                } finally {
                    $menuKey.Dispose()
                }
            }

            foreach ($keyPath in @($commandKeyPath, $menuKeyPath)) {
                if (Remove-RegistryKeyIfEmpty -Hive $Hive -KeyPath $keyPath) {
                    $changed = $true
                }
            }
        }
    }

    if ($IncludeState) {
        $stateKey = $Hive.OpenSubKey($stateKeyPath, $true)
        if ($stateKey) {
            try {
                $registeredPath = [string]$stateKey.GetValue('', $null)
                $matchesApplication = $false
                if ($registeredPath) {
                    try {
                        $matchesApplication = [string]::Equals(
                            [System.IO.Path]::GetFullPath($registeredPath),
                            $applicationPath,
                            [StringComparison]::OrdinalIgnoreCase
                        )
                    } catch [System.ArgumentException] {
                        $matchesApplication = $false
                    }
                }
                if ($matchesApplication) {
                    $stateKey.DeleteValue('', $false)
                    $changed = $true
                }
            } finally {
                $stateKey.Dispose()
            }
        }

        foreach ($keyPath in @(
            $stateKeyPath,
            'Software\MTT-File-Manager\ShellIntegration',
            'Software\MTT-File-Manager'
        )) {
            if (Remove-RegistryKeyIfEmpty -Hive $Hive -KeyPath $keyPath) {
                $changed = $true
            }
        }
    }

    return $changed
}

function Get-ProfileLocalAppData {
    param(
        [Parameter(Mandatory = $true)]
        [Microsoft.Win32.RegistryKey]$Hive,
        [Parameter(Mandatory = $true)]
        [string]$ProfilePath
    )

    $defaultLocalAppData = Join-Path (Join-Path $ProfilePath 'AppData') 'Local'
    $folderKey = $Hive.OpenSubKey(
        'Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders',
        $false
    )
    if (-not $folderKey) { return $defaultLocalAppData }

    try {
        $configuredPath = [string]$folderKey.GetValue(
            'Local AppData',
            '',
            [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames
        )
    } finally {
        $folderKey.Dispose()
    }
    if (-not $configuredPath) { return $defaultLocalAppData }

    $roamingAppData = Join-Path (Join-Path $ProfilePath 'AppData') 'Roaming'
    foreach ($token in @('%USERPROFILE%', '%UserProfile%', '%userprofile%')) {
        $configuredPath = $configuredPath.Replace($token, $ProfilePath)
    }
    foreach ($token in @('%LOCALAPPDATA%', '%LocalAppData%', '%localappdata%')) {
        $configuredPath = $configuredPath.Replace($token, $defaultLocalAppData)
    }
    foreach ($token in @('%APPDATA%', '%AppData%', '%appdata%')) {
        $configuredPath = $configuredPath.Replace($token, $roamingAppData)
    }
    $configuredPath = [Environment]::ExpandEnvironmentVariables($configuredPath)
    if ([System.IO.Path]::IsPathRooted($configuredPath)) {
        return [System.IO.Path]::GetFullPath($configuredPath)
    }
    return $defaultLocalAppData
}

function Invoke-OfflineHiveCleanup {
    param(
        [Parameter(Mandatory = $true)]
        [string]$HivePath,
        [Parameter(Mandatory = $true)]
        [string]$Sid,
        [string]$ProfilePath,
        [switch]$IncludeState
    )

    $temporaryHiveName = 'MTT_ContextCleanup_' + [guid]::NewGuid().ToString('N')
    $temporaryHiveLoaded = $false
    $hive = $null
    try {
        & "$env:SystemRoot\System32\reg.exe" load "HKU\$temporaryHiveName" $HivePath 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            throw "Could not open the registry hive for profile $Sid."
        }
        $temporaryHiveLoaded = $true
        $hive = [Microsoft.Win32.Registry]::Users.OpenSubKey($temporaryHiveName, $true)
        if (-not $hive) {
            throw "Could not access the registry hive for profile $Sid."
        }
        if ($IncludeState -and $ProfilePath) {
            try {
                $script:offlineLocalAppData = Get-ProfileLocalAppData -Hive $hive -ProfilePath $ProfilePath
            } catch {
                $script:offlineLocalAppData = Join-Path (Join-Path $ProfilePath 'AppData') 'Local'
            }
        }
        $changed = Remove-MttContextMenuFromHive -Hive $hive -IncludeState:$IncludeState
        if ($changed) { $script:shellAssociationsChanged = $true }
        return $changed
    } finally {
        if ($hive) { $hive.Dispose() }
        if ($temporaryHiveLoaded) {
            & "$env:SystemRoot\System32\reg.exe" unload "HKU\$temporaryHiveName" 2>&1 | Out-Null
            if ($LASTEXITCODE -ne 0) {
                throw "Could not unload the registry hive for profile $Sid."
            }
        }
    }
}

$usersRoot = [Microsoft.Win32.Registry]::Users
$profileList = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey(
    'SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList',
    $false
)
if (-not $profileList) {
    throw 'Could not enumerate Windows user profiles.'
}

$profilePaths = @{}
try {
    foreach ($sidName in $profileList.GetSubKeyNames()) {
        try {
            $sid = [System.Security.Principal.SecurityIdentifier]::new($sidName).Value
        } catch [System.ArgumentException] {
            continue
        }

        $profile = $profileList.OpenSubKey($sidName, $false)
        if (-not $profile) { continue }
        try {
            $profilePaths[$sid] = [Environment]::ExpandEnvironmentVariables(
                [string]$profile.GetValue('ProfileImagePath', '')
            )
        } finally {
            $profile.Dispose()
        }
    }
} finally {
    $profileList.Dispose()
}

$processedSids = @{}
$processedClassSids = @{}
$profileLocalAppData = @{}
$cleanupErrors = [System.Collections.Generic.List[string]]::new()
$shellAssociationsChanged = $false
$script:shellAssociationsChanged = $false
$script:offlineLocalAppData = $null

foreach ($subKeyName in $usersRoot.GetSubKeyNames()) {
    $isClassesHive = $subKeyName.EndsWith('_Classes', [StringComparison]::OrdinalIgnoreCase)
    $sidName = if ($isClassesHive) {
        $subKeyName.Substring(0, $subKeyName.Length - '_Classes'.Length)
    } else {
        $subKeyName
    }
    try {
        $sid = [System.Security.Principal.SecurityIdentifier]::new($sidName).Value
    } catch [System.ArgumentException] {
        continue
    }
    if (-not $profilePaths.ContainsKey($sid)) { continue }

    if ($isClassesHive) {
        if ($processedClassSids.ContainsKey($sid)) { continue }
        $hive = $null
        try {
            $hive = $usersRoot.OpenSubKey($subKeyName, $true)
            if (-not $hive) {
                $cleanupErrors.Add("Could not access the classes registry hive for profile $sid.")
                continue
            }
            $processedClassSids[$sid] = $true
            if (Remove-MttContextMenuFromHive -Hive $hive) {
                $shellAssociationsChanged = $true
            }
        } catch {
            $cleanupErrors.Add("Could not clean the MTT context-menu registration for profile $sid.")
        } finally {
            if ($hive) { $hive.Dispose() }
        }
        continue
    }

    if ($processedSids.ContainsKey($sid)) { continue }

    $hive = $null
    try {
        $hive = $usersRoot.OpenSubKey($sid, $true)
    } catch {
        $cleanupErrors.Add("Could not access the registry hive for profile $sid.")
        continue
    }
    if (-not $hive) { continue }
    $processedSids[$sid] = $true
    try {
        if (Remove-MttContextMenuFromHive -Hive $hive -IncludeState) {
            $shellAssociationsChanged = $true
        }
        if ($profilePaths[$sid]) {
            try {
                $profileLocalAppData[$sid] = Get-ProfileLocalAppData -Hive $hive -ProfilePath $profilePaths[$sid]
            } catch {
                $profileLocalAppData[$sid] = Join-Path (Join-Path $profilePaths[$sid] 'AppData') 'Local'
            }
        }
    } catch {
        $cleanupErrors.Add("Could not clean the MTT context-menu registration for profile $sid.")
    } finally {
        $hive.Dispose()
    }
}

foreach ($sid in $profilePaths.Keys) {
    $profilePath = $profilePaths[$sid]
    if (-not $profilePath) { continue }

    if (-not $processedSids.ContainsKey($sid)) {
        $profileHivePath = Join-Path $profilePath 'NTUSER.DAT'
        if (Test-Path -LiteralPath $profileHivePath -PathType Leaf) {
            $script:offlineLocalAppData = Join-Path (Join-Path $profilePath 'AppData') 'Local'
            try {
                if (Invoke-OfflineHiveCleanup -HivePath $profileHivePath -Sid $sid -ProfilePath $profilePath -IncludeState) {
                    $shellAssociationsChanged = $true
                }
                $processedSids[$sid] = $true
                $profileLocalAppData[$sid] = $script:offlineLocalAppData
            } catch {
                $cleanupErrors.Add("Could not clean the MTT context-menu registration for profile $sid.")
            }
        }
    } elseif (-not $profileLocalAppData.ContainsKey($sid)) {
        try {
            $profileLocalAppData[$sid] = Join-Path (Join-Path $profilePath 'AppData') 'Local'
        } catch {
            $profileLocalAppData[$sid] = $null
        }
    }

    if ($processedClassSids.ContainsKey($sid)) { continue }
    $localAppData = $profileLocalAppData[$sid]
    if (-not $localAppData) {
        $localAppData = Join-Path (Join-Path $profilePath 'AppData') 'Local'
    }
    $classesHivePath = Join-Path (Join-Path (Join-Path $localAppData 'Microsoft') 'Windows') 'UsrClass.dat'
    if (-not (Test-Path -LiteralPath $classesHivePath -PathType Leaf)) { continue }
    try {
        if (Invoke-OfflineHiveCleanup -HivePath $classesHivePath -Sid $sid) {
            $shellAssociationsChanged = $true
        }
        $processedClassSids[$sid] = $true
    } catch {
        $cleanupErrors.Add("Could not clean the MTT context-menu registration for profile $sid.")
    }
}

if ($shellAssociationsChanged -and -not ('MTT.Shell.AssociationChange' -as [type])) {
    Add-Type -Namespace MTT.Shell -Name AssociationChange -MemberDefinition '[System.Runtime.InteropServices.DllImport("shell32.dll")] public static extern void SHChangeNotify(uint eventId, uint flags, System.IntPtr item1, System.IntPtr item2);'
}
if ($shellAssociationsChanged) {
    [MTT.Shell.AssociationChange]::SHChangeNotify(0x08000000, 0, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
}

if ($cleanupErrors.Count -gt 0) {
    throw ($cleanupErrors -join ' ')
}
