function Test-FolderDelegateString($Record, [string]$Expected) {
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
    $serverPath = Join-Path $ApplicationDirectory 'mtt_explorer_command.dll'
    $clsid = '{7b9f6e73-c8a1-4c2d-9f18-26b47e5f9c31}'
    $folderCommandPath = "$RegistryClassesRoot\Folder\shell\open\command"
    $clsidPath = "$RegistryClassesRoot\CLSID\$clsid"
    $inprocPath = "$clsidPath\InprocServer32"
    $currentFolderDefault = Get-RegistryValueRecord $Root $folderCommandPath ''
    $currentFolderDelegate = Get-RegistryValueRecord $Root $folderCommandPath 'DelegateExecute'
    $currentServer = Get-RegistryValueRecord $Root $inprocPath ''
    $currentThreading = Get-RegistryValueRecord $Root $inprocPath 'ThreadingModel'

    if ($null -eq $Snapshot) {
        if ((Test-FolderCommandUsesExecutable $currentFolderDefault $applicationExe) -or
            (Test-FolderDelegateString $currentFolderDelegate $clsid) -or
            (Test-RegistryPathRecord $currentServer $serverPath)) {
            throw 'An MTT Folder COM handler is active but its restoration snapshot is missing.'
        }
        return
    }

    if (-not (Test-SameInstallPath ([string]$Snapshot.installed_server_path) $serverPath)) {
        if ((Test-FolderDelegateString $currentFolderDelegate $clsid) -or
            (Test-RegistryPathRecord $currentServer ([string]$Snapshot.installed_server_path))) {
            throw 'The Folder COM snapshot belongs to another installation but is still active.'
        }
        return
    }

    if ([string]$Snapshot.installed_clsid -ne $clsid -or
        -not (Test-FolderCommandLineTargetsPath ([string]$Snapshot.installed_folder_command) $applicationExe)) {
        throw 'The saved Folder COM snapshot does not match this installation.'
    }

    $folderDefaultOwned = Test-FolderDelegateString $currentFolderDefault ([string]$Snapshot.installed_folder_command)
    $folderDelegateOwned = Test-FolderDelegateString $currentFolderDelegate ([string]$Snapshot.installed_clsid)
    if ($folderDefaultOwned) {
        Set-RegistryValueFromSnapshot $Root $folderCommandPath '' $Snapshot.default_command
    }
    if ($folderDelegateOwned) {
        Set-RegistryValueFromSnapshot $Root $folderCommandPath 'DelegateExecute' $Snapshot.delegate_execute
    }

    $remainingFolderDefault = Get-RegistryValueRecord $Root $folderCommandPath ''
    $remainingFolderDelegate = Get-RegistryValueRecord $Root $folderCommandPath 'DelegateExecute'
    if ((Test-FolderCommandUsesExecutable $remainingFolderDefault $applicationExe) -or
        (Test-FolderDelegateString $remainingFolderDelegate $clsid)) {
        throw 'The Folder handler still references MTT; its COM registration was retained.'
    }

    $serverOwned = Test-RegistryPathRecord $currentServer $serverPath
    $threadingOwned = Test-FolderDelegateString $currentThreading 'Apartment'
    if ($serverOwned) {
        Set-RegistryValueFromSnapshot $Root $inprocPath '' $Snapshot.inproc_server
    }
    if ($threadingOwned) {
        Set-RegistryValueFromSnapshot $Root $inprocPath 'ThreadingModel' $Snapshot.threading_model
    }
    $remainingServer = Get-RegistryValueRecord $Root $inprocPath ''
    if (Test-RegistryPathRecord $remainingServer $serverPath) {
        throw 'The Folder COM registration still points to the MTT DLL; it was retained.'
    }

    if (-not $Snapshot.inproc_key_existed) {
        Remove-EmptySubKey $Root $clsidPath 'InprocServer32'
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
