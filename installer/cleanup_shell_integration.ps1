param(
    [Parameter(Mandatory = $true)]
    [string]$ApplicationDirectory,
    [Parameter(Mandatory = $true)]
    [string]$PackageName
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$applicationExe = Join-Path $ApplicationDirectory 'mtt-file-manager.exe'
$registryClassesRoot = 'Software\Classes'
$integrationKeyPath = 'Software\MTT-File-Manager\ShellIntegration'
$contextMenuKeyName = 'MTT.FileManager.OpenInMTT'
$utf16 = [System.Text.Encoding]::Unicode

function Get-RegistryValueRecord([Microsoft.Win32.RegistryKey]$Root, [string]$Path, [string]$Name) {
    $key = $Root.OpenSubKey($Path, $false)
    if (-not $key) { return $null }
    try {
        if ($key.GetValueNames() -notcontains $Name) { return $null }
        return [pscustomobject]@{
            Kind = [int]$key.GetValueKind($Name)
            Value = $key.GetValue($Name, $null, [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
        }
    } finally {
        $key.Dispose()
    }
}

function Convert-SnapshotValue($Snapshot) {
    if ($null -eq $Snapshot) { return $null }
    $bytes = [byte[]]@($Snapshot.bytes)
    switch ([int]$Snapshot.value_type) {
        1 { $kind = [Microsoft.Win32.RegistryValueKind]::String; $value = $utf16.GetString($bytes).TrimEnd([char]0) }
        2 { $kind = [Microsoft.Win32.RegistryValueKind]::ExpandString; $value = $utf16.GetString($bytes).TrimEnd([char]0) }
        3 { $kind = [Microsoft.Win32.RegistryValueKind]::Binary; $value = $bytes }
        4 { $kind = [Microsoft.Win32.RegistryValueKind]::DWord; $value = [BitConverter]::ToInt32($bytes, 0) }
        7 {
            $kind = [Microsoft.Win32.RegistryValueKind]::MultiString
            $raw = $utf16.GetString($bytes)
            $value = [string[]]$raw.Split([char[]]@([char]0), [System.StringSplitOptions]::RemoveEmptyEntries)
        }
        11 { $kind = [Microsoft.Win32.RegistryValueKind]::QWord; $value = [BitConverter]::ToInt64($bytes, 0) }
        default { throw 'The saved Shell handler uses an unsupported registry value type.' }
    }
    [pscustomobject]@{ Kind = $kind; Value = $value }
}

function Test-ByteArraysEqual([byte[]]$Left, [byte[]]$Right) {
    if ($Left.Length -ne $Right.Length) { return $false }
    for ($index = 0; $index -lt $Left.Length; $index++) {
        if ($Left[$index] -ne $Right[$index]) { return $false }
    }
    return $true
}

function Test-RegistryValueMatchesSnapshot(
    [Microsoft.Win32.RegistryKey]$Root,
    [string]$Path,
    [string]$Name,
    $Snapshot
) {
    $current = Get-RegistryValueRecord $Root $Path $Name
    if ($null -eq $Snapshot) { return $null -eq $current }
    if ($null -eq $current -or $current.Kind -ne [int]$Snapshot.value_type) { return $false }

    $expected = Convert-SnapshotValue $Snapshot
    switch ($expected.Kind) {
        ([Microsoft.Win32.RegistryValueKind]::Binary) {
            return Test-ByteArraysEqual ([byte[]]$current.Value) ([byte[]]$expected.Value)
        }
        ([Microsoft.Win32.RegistryValueKind]::DWord) {
            return Test-ByteArraysEqual ([BitConverter]::GetBytes([int]$current.Value)) ([BitConverter]::GetBytes([int]$expected.Value))
        }
        ([Microsoft.Win32.RegistryValueKind]::QWord) {
            return Test-ByteArraysEqual ([BitConverter]::GetBytes([long]$current.Value)) ([BitConverter]::GetBytes([long]$expected.Value))
        }
        ([Microsoft.Win32.RegistryValueKind]::MultiString) {
            return [string]::Join([char]0, [string[]]$current.Value) -ceq [string]::Join([char]0, [string[]]$expected.Value)
        }
        default {
            return [string]::Equals([string]$current.Value, [string]$expected.Value, [StringComparison]::Ordinal)
        }
    }
}

function Set-RegistryValueFromSnapshot(
    [Microsoft.Win32.RegistryKey]$Root,
    [string]$Path,
    [string]$Name,
    $Snapshot
) {
    $key = $Root.CreateSubKey($Path, $true)
    if (-not $key) { throw 'Could not open a Shell registry key for restoration.' }
    try {
        if ($null -eq $Snapshot) {
            if ($key.GetValueNames() -contains $Name) { $key.DeleteValue($Name, $false) }
            return
        }
        $value = Convert-SnapshotValue $Snapshot
        $key.SetValue($Name, $value.Value, $value.Kind)
    } finally {
        $key.Dispose()
    }
}

function Remove-EmptySubKey(
    [Microsoft.Win32.RegistryKey]$Root,
    [string]$ParentPath,
    [string]$ChildName
) {
    $parent = $Root.OpenSubKey($ParentPath, $true)
    if (-not $parent) { return }
    try {
        $child = $parent.OpenSubKey($ChildName, $true)
        if (-not $child) { return }
        try {
            if ($child.GetValueNames().Count -eq 0 -and $child.GetSubKeyNames().Count -eq 0) {
                $parent.DeleteSubKey($ChildName, $false)
            }
        } finally {
            $child.Dispose()
        }
    } finally {
        $parent.Dispose()
    }
}

function Remove-EmptyHandlerAncestors(
    [Microsoft.Win32.RegistryKey]$Root,
    [string]$ClassName,
    $Verb
) {
    $classPath = "$registryClassesRoot\$ClassName"
    if (-not $Verb.command_key_existed) { Remove-EmptySubKey $Root "$classPath\shell\open" 'command' }
    if (-not $Verb.open_key_existed) { Remove-EmptySubKey $Root "$classPath\shell" 'open' }
    if (-not $Verb.shell_key_existed) { Remove-EmptySubKey $Root $classPath 'shell' }
}

function Remove-ContextMenuEntry([Microsoft.Win32.RegistryKey]$Root, [string]$ClassName) {
    $menuPath = "$registryClassesRoot\$ClassName\shell\$contextMenuKeyName"
    $commandPath = "$menuPath\command"
    $entries = @(
        [pscustomobject]@{ Path = $commandPath; Name = '' },
        [pscustomobject]@{ Path = $menuPath; Name = '' },
        [pscustomobject]@{ Path = $menuPath; Name = 'Icon' },
        [pscustomobject]@{ Path = $menuPath; Name = 'Position' }
    )
    foreach ($entry in $entries) {
        $key = $Root.OpenSubKey($entry.Path, $true)
        if ($key) {
            try {
                if ($key.GetValueNames() -contains $entry.Name) { $key.DeleteValue($entry.Name, $false) }
            } finally {
                $key.Dispose()
            }
        }
    }
    Remove-EmptySubKey $Root $menuPath 'command'
    Remove-EmptySubKey $Root "$registryClassesRoot\$ClassName\shell" $contextMenuKeyName
}

. (Join-Path $PSScriptRoot 'cleanup_folder_delegate.ps1')

function Cleanup-UserHive([Microsoft.Win32.RegistryKey]$Root) {
    $snapshotRecord = Get-RegistryValueRecord $Root $integrationKeyPath 'DefaultHandlerSnapshot'
    $snapshot = $null
    if ($snapshotRecord) {
        if ($snapshotRecord.Kind -ne [int][Microsoft.Win32.RegistryValueKind]::String) {
            throw 'The saved Shell handler snapshot has an unexpected registry type.'
        }
        $snapshot = $snapshotRecord.Value | ConvertFrom-Json
        if ([int]$snapshot.version -notin @(1, 2)) { throw 'The saved Shell handler snapshot version is unsupported.' }
    }

    $expectedDefaultCommand = '"' + $applicationExe + '" "%1"'
    $snapshotBelongsToThisInstall = $false
    if ($snapshot) {
        foreach ($installedCommand in @($snapshot.installed_commands)) {
            if (Test-FolderCommandLineTargetsPath ([string]$installedCommand) $applicationExe) {
                $snapshotBelongsToThisInstall = $true
                break
            }
        }
    }
    $verbsToRestore = @()
    foreach ($className in @('Directory', 'Drive')) {
        $commandPath = "$registryClassesRoot\$className\shell\open\command"
        $currentCommand = Get-RegistryValueRecord $Root $commandPath ''
        $currentDelegate = Get-RegistryValueRecord $Root $commandPath 'DelegateExecute'
        $currentText = if ($currentCommand -and $currentCommand.Kind -in @([int][Microsoft.Win32.RegistryValueKind]::String, [int][Microsoft.Win32.RegistryValueKind]::ExpandString)) { [string]$currentCommand.Value } else { $null }

        if ($null -eq $snapshot) {
            if ($currentText -and $currentText.IndexOf($applicationExe, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw 'An MTT folder handler is active but its restoration snapshot is missing.'
            }
            continue
        }
        if (-not $snapshotBelongsToThisInstall) {
            if ($currentText -and $currentText.IndexOf($applicationExe, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw 'An MTT folder handler is active but its restoration snapshot does not belong to this installation.'
            }
            continue
        }

        $verb = @($snapshot.verbs | Where-Object { $_.class_name -eq $className }) | Select-Object -First 1
        if (-not $verb) { throw 'The saved Shell handler snapshot is incomplete.' }
        if ($currentText -and $currentText.IndexOf($applicationExe, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            if ($currentText -cne $expectedDefaultCommand -or $null -ne $currentDelegate) {
                throw 'A folder handler still points to MTT but differs from its saved registration.'
            }
            $previous = Convert-SnapshotValue $verb.default_command
            if ($previous -and $previous.Value -is [string] -and
                $previous.Value.IndexOf($applicationExe, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
                throw 'The previous folder handler also points to MTT; uninstall was stopped to avoid leaving a broken handler.'
            }
            $verbsToRestore += $verb
        } elseif ((Test-RegistryValueMatchesSnapshot $Root $commandPath '' $verb.default_command) -and
                  (Test-RegistryValueMatchesSnapshot $Root $commandPath 'DelegateExecute' $verb.delegate_execute)) {
            continue
        }
    }

    foreach ($verb in $verbsToRestore) {
        $className = [string]$verb.class_name
        $commandPath = "$registryClassesRoot\$className\shell\open\command"
        Set-RegistryValueFromSnapshot $Root $commandPath '' $verb.default_command
        Set-RegistryValueFromSnapshot $Root $commandPath 'DelegateExecute' $verb.delegate_execute
        Remove-EmptyHandlerAncestors $Root $className $verb
    }

    $folderSnapshot = $null
    if ($snapshotBelongsToThisInstall -and $snapshot -and $snapshot.PSObject.Properties['folder_delegate']) {
        $folderSnapshot = $snapshot.folder_delegate
    }
    Restore-FolderDelegateSnapshot -Root $Root -Snapshot $folderSnapshot -ApplicationDirectory $ApplicationDirectory -RegistryClassesRoot $registryClassesRoot

    $menuState = Get-RegistryValueRecord $Root "$integrationKeyPath\ContextMenu" ''
    $menuExe = if ($menuState -and $menuState.Kind -eq [int][Microsoft.Win32.RegistryValueKind]::String) { [string]$menuState.Value } else { $null }
    $menuMarkerBelongsToThisInstall = $menuExe -and $menuExe.Equals($applicationExe, [StringComparison]::OrdinalIgnoreCase)
    foreach ($className in @('Directory', 'Drive')) {
        $menuCommandPath = "$registryClassesRoot\$className\shell\$contextMenuKeyName\command"
        $currentMenuCommand = Get-RegistryValueRecord $Root $menuCommandPath ''
        $currentMenuText = if ($currentMenuCommand -and $currentMenuCommand.Kind -in @([int][Microsoft.Win32.RegistryValueKind]::String, [int][Microsoft.Win32.RegistryValueKind]::ExpandString)) { [string]$currentMenuCommand.Value } else { $null }
        if ($currentMenuText -ceq $expectedDefaultCommand) {
            Remove-ContextMenuEntry $Root $className
        } elseif ($currentMenuText -and $currentMenuText.IndexOf($applicationExe, [StringComparison]::OrdinalIgnoreCase) -ge 0) {
            throw 'A context-menu command still points to MTT but differs from its saved registration.'
        }
    }

    $integrationKey = $Root.OpenSubKey($integrationKeyPath, $true)
    if ($integrationKey) {
        try {
            if ($snapshotBelongsToThisInstall -and $integrationKey.GetValueNames() -contains 'DefaultHandlerSnapshot') {
                $integrationKey.DeleteValue('DefaultHandlerSnapshot', $false)
            }
            if ($menuMarkerBelongsToThisInstall -and $integrationKey.GetSubKeyNames() -contains 'ContextMenu') {
                $contextKey = $integrationKey.OpenSubKey('ContextMenu', $true)
                if ($contextKey) {
                    try {
                        if ($contextKey.GetValueNames() -contains '') { $contextKey.DeleteValue('', $false) }
                    } finally {
                        $contextKey.Dispose()
                    }
                }
            }
        } finally {
            $integrationKey.Dispose()
        }
    }
    if ($menuMarkerBelongsToThisInstall) {
        Remove-EmptySubKey $Root $integrationKeyPath 'ContextMenu'
    }
    Remove-EmptySubKey $Root 'Software\MTT-File-Manager' 'ShellIntegration'
}

function Get-UserProfiles {
    $profileList = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey('SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList')
    if (-not $profileList) { throw 'Could not enumerate Windows user profiles.' }
    try {
        foreach ($sid in $profileList.GetSubKeyNames()) {
            if ($sid -in @('S-1-5-18', 'S-1-5-19', 'S-1-5-20') -or $sid.EndsWith('.bak')) { continue }
            $profile = $profileList.OpenSubKey($sid)
            if (-not $profile) { continue }
            try {
                $profilePath = [Environment]::ExpandEnvironmentVariables([string]$profile.GetValue('ProfileImagePath', ''))
                if ($profilePath) {
                    [pscustomobject]@{ Sid = $sid; Path = $profilePath }
                }
            } finally {
                $profile.Dispose()
            }
        }
    } finally {
        $profileList.Dispose()
    }
}

try {
    $usersRoot = [Microsoft.Win32.Registry]::Users
    $regExe = Join-Path $env:SystemRoot 'System32\reg.exe'
    foreach ($profile in Get-UserProfiles) {
        $rootName = $profile.Sid
        $hive = $usersRoot.OpenSubKey($rootName, $true)
        $loadedByScript = $false
        if (-not $hive) {
            $hiveFile = Join-Path $profile.Path 'NTUSER.DAT'
            if (-not (Test-Path -LiteralPath $hiveFile -PathType Leaf)) { continue }
            $rootName = 'MTT_Cleanup_' + [guid]::NewGuid().ToString('N')
            & $regExe load "HKU\$rootName" $hiveFile | Out-Null
            if ($LASTEXITCODE -ne 0) { throw 'Could not load a Windows user profile for Shell cleanup.' }
            $loadedByScript = $true
            $hive = $usersRoot.OpenSubKey($rootName, $true)
            if (-not $hive) { throw 'Could not access a loaded Windows user profile.' }
        }
        try {
            Cleanup-UserHive $hive
        } finally {
            $hive.Dispose()
            if ($loadedByScript) {
                & $regExe unload "HKU\$rootName" | Out-Null
                if ($LASTEXITCODE -ne 0) { throw 'Could not unload a Windows user profile after Shell cleanup.' }
            }
        }
    }

    & (Join-Path $PSScriptRoot 'unregister_sparse_package.ps1') -PackageName $PackageName -AllUsers
} catch {
    Write-Error $_
    exit 1
}
