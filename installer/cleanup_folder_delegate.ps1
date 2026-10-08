function Test-RegistryStringEquals($Record, [string]$Expected) {
    return $Record -and
        $Record.Kind -in @([int][Microsoft.Win32.RegistryValueKind]::String, [int][Microsoft.Win32.RegistryValueKind]::ExpandString) -and
        [string]::Equals([string]$Record.Value, $Expected, [StringComparison]::OrdinalIgnoreCase)
}

function Normalize-WindowsPath([string]$Path) {
    $fullPath = [System.IO.Path]::GetFullPath($Path)
    if ($fullPath.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        return '\\' + $fullPath.Substring(8)
    }
    if ($fullPath.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {
        return $fullPath.Substring(4)
    }
    return $fullPath
}

function Test-SameInstallPath([string]$Left, [string]$Right) {
    if (-not $Left -or -not $Right) { return $false }
    return [string]::Equals((Normalize-WindowsPath $Left), (Normalize-WindowsPath $Right), [StringComparison]::OrdinalIgnoreCase)
}

function Get-FolderCommandExecutablePath($Record) {
    if (-not $Record -or $Record.Kind -notin @([int][Microsoft.Win32.RegistryValueKind]::String, [int][Microsoft.Win32.RegistryValueKind]::ExpandString)) { return $null }
    $command = ([string]$Record.Value).TrimStart()
    if ($command.StartsWith('"')) {
        $end = $command.IndexOf('"', 1)
        if ($end -gt 1) { return $command.Substring(1, $end - 1) }
        return $null
    }
    $space = $command.IndexOf(' ')
    if ($space -ge 0) { return $command.Substring(0, $space) }
    return $command
}

function Test-FolderCommandUsesExecutable($Record, [string]$ExpectedExecutable) {
    $commandExecutable = Get-FolderCommandExecutablePath $Record
    return $commandExecutable -and (Test-SameInstallPath $commandExecutable $ExpectedExecutable)
}

function Test-FolderCommandLineTargetsPath([string]$Command, [string]$ExpectedExecutable) {
    $record = [pscustomobject]@{ Kind = [int][Microsoft.Win32.RegistryValueKind]::String; Value = $Command }
    return Test-FolderCommandUsesExecutable $record $ExpectedExecutable
}

function Test-RegistryPathRecord($Record, [string]$ExpectedPath) {
    return $Record -and
        $Record.Kind -in @([int][Microsoft.Win32.RegistryValueKind]::String, [int][Microsoft.Win32.RegistryValueKind]::ExpandString) -and
        (Test-SameInstallPath ([string]$Record.Value) $ExpectedPath)
}

function Notify-ShellAssociationChanged {
    if (-not ('MTT.Shell.AssociationChange' -as [type])) {
        Add-Type -Namespace MTT.Shell -Name AssociationChange -MemberDefinition '[System.Runtime.InteropServices.DllImport("shell32.dll")] public static extern void SHChangeNotify(uint eventId, uint flags, System.IntPtr item1, System.IntPtr item2);'
    }
    [MTT.Shell.AssociationChange]::SHChangeNotify(0x08000000, 0, [System.IntPtr]::Zero, [System.IntPtr]::Zero)
}

function Restore-FolderDelegateSnapshot {
    param(
        [Microsoft.Win32.RegistryKey]$Root,
        [object]$Snapshot,
        [string]$ApplicationDirectory,
        [string]$RegistryClassesRoot
    )

    $applicationExe = Join-Path $ApplicationDirectory 'mtt-file-manager.exe'
    $serverExe = Join-Path $ApplicationDirectory 'mtt-shell-command.exe'
    $legacyServerDll = Join-Path $ApplicationDirectory 'mtt_explorer_command.dll'
    $clsid = '{2e6a41d7-54b3-4f0e-b8c2-9d17a3e5c6b8}'
    $legacyClsid = '{7b9f6e73-c8a1-4c2d-9f18-26b47e5f9c31}'
    $folderCommandPath = "$RegistryClassesRoot\Folder\shell\open\command"
    $clsidPath = "$RegistryClassesRoot\CLSID\$clsid"
    $localServerPath = "$clsidPath\LocalServer32"
    $legacyClsidPath = "$RegistryClassesRoot\CLSID\$legacyClsid"
    $legacyInprocPath = "$legacyClsidPath\InprocServer32"
    $currentFolderDefault = Get-RegistryValueRecord $Root $folderCommandPath ''
    $currentFolderDelegate = Get-RegistryValueRecord $Root $folderCommandPath 'DelegateExecute'

    if ($null -eq $Snapshot) {
        $currentServerCommand = Get-RegistryValueRecord $Root $localServerPath ''
        $currentLegacyServer = Get-RegistryValueRecord $Root $legacyInprocPath ''
        if ((Test-FolderCommandUsesExecutable $currentFolderDefault $applicationExe) -or
            (Test-RegistryStringEquals $currentFolderDelegate $clsid) -or
            (Test-FolderCommandUsesExecutable $currentServerCommand $serverExe) -or
            (Test-RegistryStringEquals $currentFolderDelegate $legacyClsid) -or
            (Test-RegistryPathRecord $currentLegacyServer $legacyServerDll)) {
            throw 'An MTT Folder COM handler is active but its restoration snapshot is missing.'
        }
        return
    }

    if ($Snapshot.PSObject.Properties['installed_server_path']) {
        $installedLegacyServer = [string]$Snapshot.installed_server_path
        $currentLegacyServer = Get-RegistryValueRecord $Root $legacyInprocPath ''
        $currentLegacyThreading = Get-RegistryValueRecord $Root $legacyInprocPath 'ThreadingModel'
        if (-not (Test-SameInstallPath $installedLegacyServer $legacyServerDll)) {
            if ((Test-RegistryStringEquals $currentFolderDelegate $legacyClsid) -or
                (Test-RegistryPathRecord $currentLegacyServer $installedLegacyServer)) {
                throw 'The legacy Folder COM snapshot belongs to another installation but is still active.'
            }
            return
        }
        if ([string]$Snapshot.installed_clsid -ne $legacyClsid -or
            -not (Test-FolderCommandLineTargetsPath ([string]$Snapshot.installed_folder_command) $applicationExe)) {
            throw 'The saved legacy Folder COM snapshot does not match this installation.'
        }

        if (Test-RegistryStringEquals $currentFolderDefault ([string]$Snapshot.installed_folder_command)) {
            Set-RegistryValueFromSnapshot $Root $folderCommandPath '' $Snapshot.default_command
        }
        if (Test-RegistryStringEquals $currentFolderDelegate ([string]$Snapshot.installed_clsid)) {
            Set-RegistryValueFromSnapshot $Root $folderCommandPath 'DelegateExecute' $Snapshot.delegate_execute
        }
        $remainingFolderDefault = Get-RegistryValueRecord $Root $folderCommandPath ''
        $remainingFolderDelegate = Get-RegistryValueRecord $Root $folderCommandPath 'DelegateExecute'
        if ((Test-FolderCommandUsesExecutable $remainingFolderDefault $applicationExe) -or
            (Test-RegistryStringEquals $remainingFolderDelegate $legacyClsid)) {
            throw 'The legacy Folder handler still references MTT; its COM registration was retained.'
        }

        if (Test-RegistryPathRecord $currentLegacyServer $installedLegacyServer) {
            Set-RegistryValueFromSnapshot $Root $legacyInprocPath '' $Snapshot.inproc_server
            if (Test-RegistryStringEquals $currentLegacyThreading 'Apartment') {
                Set-RegistryValueFromSnapshot $Root $legacyInprocPath 'ThreadingModel' $Snapshot.threading_model
            }
        }
        $remainingLegacyServer = Get-RegistryValueRecord $Root $legacyInprocPath ''
        if (Test-RegistryPathRecord $remainingLegacyServer $installedLegacyServer) {
            throw 'The legacy Folder COM registration still points to the MTT DLL; it was retained.'
        }

        if (-not $Snapshot.inproc_key_existed) {
            Remove-EmptySubKey $Root $legacyClsidPath 'InprocServer32'
        }
        if (-not $Snapshot.clsid_key_existed) {
            Remove-EmptySubKey $Root "$RegistryClassesRoot\CLSID" $legacyClsid
        }
        if (-not $Snapshot.command_key_existed) {
            Remove-EmptySubKey $Root "$RegistryClassesRoot\Folder\shell\open" 'command'
        }
        if (-not $Snapshot.open_key_existed) {
            Remove-EmptySubKey $Root "$RegistryClassesRoot\Folder\shell" 'open'
        }
        if (-not $Snapshot.shell_key_existed) {
            Remove-EmptySubKey $Root "$RegistryClassesRoot\Folder" 'shell'
        }
        if (-not $Snapshot.folder_key_existed) {
            Remove-EmptySubKey $Root $RegistryClassesRoot 'Folder'
        }
        if (-not $Snapshot.clsid_root_key_existed) {
            Remove-EmptySubKey $Root $RegistryClassesRoot 'CLSID'
        }
        if (-not $Snapshot.classes_root_key_existed) {
            Remove-EmptySubKey $Root 'Software' 'Classes'
        }
        Notify-ShellAssociationChanged
        return
    }

    $installedServerCommand = if ($Snapshot.PSObject.Properties['installed_server_command']) { [string]$Snapshot.installed_server_command } else { $null }
    if (-not $installedServerCommand) {
        throw 'The saved Folder COM snapshot does not match this installation.'
    }
    if (-not (Test-FolderCommandLineTargetsPath $installedServerCommand $serverExe)) {
        $currentServerCommand = Get-RegistryValueRecord $Root $localServerPath ''
        if ((Test-RegistryStringEquals $currentFolderDelegate $clsid) -or
            (Test-FolderCommandUsesExecutable $currentServerCommand $serverExe)) {
            throw 'The Folder COM snapshot belongs to another installation but is still active.'
        }
        return
    }

    if ([string]$Snapshot.installed_clsid -ne $clsid -or
        -not (Test-FolderCommandLineTargetsPath ([string]$Snapshot.installed_folder_command) $applicationExe)) {
        throw 'The saved Folder COM snapshot does not match this installation.'
    }

    $currentServerCommand = Get-RegistryValueRecord $Root $localServerPath ''
    if (Test-RegistryStringEquals $currentFolderDefault ([string]$Snapshot.installed_folder_command)) {
        Set-RegistryValueFromSnapshot $Root $folderCommandPath '' $Snapshot.default_command
    }
    if (Test-RegistryStringEquals $currentFolderDelegate ([string]$Snapshot.installed_clsid)) {
        Set-RegistryValueFromSnapshot $Root $folderCommandPath 'DelegateExecute' $Snapshot.delegate_execute
    }

    $remainingFolderDefault = Get-RegistryValueRecord $Root $folderCommandPath ''
    $remainingFolderDelegate = Get-RegistryValueRecord $Root $folderCommandPath 'DelegateExecute'
    if ((Test-FolderCommandUsesExecutable $remainingFolderDefault $applicationExe) -or
        (Test-RegistryStringEquals $remainingFolderDelegate $clsid) -or
        (Test-RegistryStringEquals $remainingFolderDelegate $legacyClsid)) {
        throw 'The Folder handler still references MTT; its COM registration was retained.'
    }

    if (Test-RegistryStringEquals $currentServerCommand $installedServerCommand) {
        Set-RegistryValueFromSnapshot $Root $localServerPath '' $Snapshot.local_server
    }
    $remainingServerCommand = Get-RegistryValueRecord $Root $localServerPath ''
    if (Test-FolderCommandUsesExecutable $remainingServerCommand $serverExe) {
        throw 'The Folder COM registration still points to the MTT server; it was retained.'
    }

    if (-not $Snapshot.local_server_key_existed) {
        Remove-EmptySubKey $Root $clsidPath 'LocalServer32'
    }
    if (-not $Snapshot.clsid_key_existed) {
        Remove-EmptySubKey $Root "$RegistryClassesRoot\CLSID" $clsid
    }
    if (-not $Snapshot.clsid_root_key_existed) {
        Remove-EmptySubKey $Root $RegistryClassesRoot 'CLSID'
    }
    if (-not $Snapshot.command_key_existed) {
        Remove-EmptySubKey $Root "$RegistryClassesRoot\Folder\shell\open" 'command'
    }
    if (-not $Snapshot.open_key_existed) {
        Remove-EmptySubKey $Root "$RegistryClassesRoot\Folder\shell" 'open'
    }
    if (-not $Snapshot.shell_key_existed) {
        Remove-EmptySubKey $Root "$RegistryClassesRoot\Folder" 'shell'
    }
    if (-not $Snapshot.folder_key_existed) {
        Remove-EmptySubKey $Root $RegistryClassesRoot 'Folder'
    }
    if (-not $Snapshot.classes_root_key_existed) {
        Remove-EmptySubKey $Root 'Software' 'Classes'
    }

    Notify-ShellAssociationChanged
}
