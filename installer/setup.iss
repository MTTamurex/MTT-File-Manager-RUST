; ==========================================================================
; MTT File Manager - Inno Setup Script
; ==========================================================================
; Requires: Inno Setup 6+ (https://jrsoftware.org/isinfo.php)
; Build:    ISCC.exe installer\setup.iss
; ==========================================================================

#define MyAppName      "MTT File Manager"
#define MyAppVersion   "0.2.8"
#define MyAppPublisher "MTT"
#define MyAppExeName   "mtt-file-manager.exe"
#define MySearchSvc    "mtt-search-service.exe"
#define MySearchName   "MTTFileManagerSearch"
#define MyAppURL       "https://github.com/MTT-File-Manager-RUST"

; Source root is the repository root (one level above this .iss file)
#define SrcRoot        ".."

[Setup]
AppId={{E3A9F1B2-7C4D-4E5F-8A1B-2C3D4E5F6A7B}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
; Output installer to installer\output\
OutputDir={#SrcRoot}\installer\output
OutputBaseFilename=MTT-File-Manager-Setup-{#MyAppVersion}
SetupIconFile={#SrcRoot}\appicon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2/ultra64
SolidCompression=yes
LZMAUseSeparateProcess=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
PrivilegesRequired=admin
MinVersion=10.0
DisableProgramGroupPage=yes
DisableDirPage=yes

[Languages]
Name: "english";    MessagesFile: "compiler:Default.isl"
Name: "portuguese";  MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "chinese";     MessagesFile: "languages\ChineseSimplified.isl"
Name: "russian";     MessagesFile: "languages\Russian.isl"
Name: "greek";       MessagesFile: "languages\Greek.isl"

[Tasks]
Name: "desktopicon";  Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

#ifndef ModernMenuPackageEnabled
[InstallDelete]
; Do not leave an older modern-menu extension behind when this build can only
; install the classic static-verb fallback.
Type: files; Name: "{app}\mtt_explorer_command.dll"
Type: files; Name: "{app}\mtt-file-manager.identity.msix"
#endif

[Files]
; Main executable
Source: "{#SrcRoot}\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

; License and notice files
Source: "{#SrcRoot}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SrcRoot}\NOTICE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SrcRoot}\THIRD_PARTY_NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SrcRoot}\third_party_licenses\*"; DestDir: "{app}\third_party_licenses"; Flags: ignoreversion recursesubdirs

; libmpv runtime
Source: "{#SrcRoot}\target\release\libmpv-2.dll"; DestDir: "{app}"; Flags: ignoreversion

; Pdfium runtime
Source: "{#SrcRoot}\target\release\pdfium.dll"; DestDir: "{app}"; Flags: ignoreversion

; Search service
Source: "{#SrcRoot}\target\release\{#MySearchSvc}"; DestDir: "{app}"; Flags: ignoreversion

; Per-user context-menu cleanup and sparse-package unregistration
Source: "{#SrcRoot}\installer\cleanup_context_menu.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SrcRoot}\installer\unregister_sparse_package.ps1"; DestDir: "{app}"; Flags: ignoreversion

#ifdef ModernMenuPackageEnabled
; Windows 11 compact context-menu extension (signed sparse package)
Source: "{#SrcRoot}\target\release\mtt_explorer_command.dll"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SrcRoot}\appicon.png"; DestDir: "{app}\Assets"; Flags: ignoreversion
Source: "{#SrcRoot}\target\release\mtt-file-manager.identity.msix"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SrcRoot}\installer\register_sparse_package.ps1"; DestDir: "{app}"; Flags: ignoreversion
#endif

; mpv portable config (scripts, settings)
Source: "{#SrcRoot}\mpv_ui\portable_config\mpv.conf";            DestDir: "{app}\mpv_ui\portable_config"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\input.conf";          DestDir: "{app}\mpv_ui\portable_config"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\scripts\autoload.lua"; DestDir: "{app}\mpv_ui\portable_config\scripts"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\scripts\modernH.lua";  DestDir: "{app}\mpv_ui\portable_config\scripts"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\scripts\vsr.lua";      DestDir: "{app}\mpv_ui\portable_config\scripts"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\script-opts\*";       DestDir: "{app}\mpv_ui\portable_config\script-opts"; Flags: ignoreversion recursesubdirs
Source: "{#SrcRoot}\mpv_ui\portable_config\fonts\*";              DestDir: "{app}\mpv_ui\portable_config\fonts"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\{#MyAppName}";         Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}";   Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; WorkingDir: "{app}"

[Run]
#ifdef ModernMenuPackageEnabled
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\register_sparse_package.ps1"" -PackagePath ""{app}\mtt-file-manager.identity.msix"" -ExternalLocation ""{app}"" -PackageName ""MTT.FileManager"" -PackageVersion ""{#MyAppVersion}.0"""; Flags: runhidden waituntilterminated runasoriginaluser
#else
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\unregister_sparse_package.ps1"" -PackageName ""MTT.FileManager"" -AllUsers"; Flags: runhidden waituntilterminated
#endif
; (Re)install and start the search indexer Windows service.
; The service was already stopped in CurStepChanged(ssInstall) before files
; were copied, so "install" here is idempotent for fresh installs and safe
; for upgrades.
Filename: "{app}\{#MySearchSvc}"; Parameters: "install"; StatusMsg: "{cm:InstallSearchService}"; Flags: runhidden waituntilterminated
Filename: "{sys}\sc.exe"; Parameters: "start {#MySearchName}"; StatusMsg: "{cm:StartSearchService}"; Flags: runhidden waituntilterminated

[UninstallRun]
; Use the app's transactional restoration, including canonical Open backups,
; while the executable is still present. This does not launch the UI.
Filename: "{app}\{#MyAppExeName}"; Parameters: "--restore-folder-integration"; RunOnceId: "RestoreMttFolderIntegration"; Flags: runhidden waituntilterminated
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\cleanup_context_menu.ps1"" -ApplicationPath ""{app}\{#MyAppExeName}"""; RunOnceId: "CleanupMttContextMenu"; Flags: runhidden waituntilterminated
Filename: "{sys}\WindowsPowerShell\v1.0\powershell.exe"; Parameters: "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File ""{app}\unregister_sparse_package.ps1"" -PackageName ""MTT.FileManager"" -AllUsers"; RunOnceId: "UnregisterMttSparsePackage"; Flags: runhidden waituntilterminated
; Stop and remove the search service before files are deleted
Filename: "{sys}\sc.exe"; Parameters: "stop {#MySearchName}"; RunOnceId: "StopSearchService"; Flags: runhidden waituntilterminated
Filename: "{app}\{#MySearchSvc}"; Parameters: "uninstall"; RunOnceId: "UninstallSearchService"; Flags: runhidden waituntilterminated

[Code]
#include "vcredist.iss"

const
  ProgramDataCacheDir = 'C:\ProgramData\MTT-File-Manager';
  DefaultFolderOpenCommandKey = 'Software\Classes\Folder\shell\open\command';
  DefaultFolderExploreCommandKey = 'Software\Classes\Folder\shell\explore\command';
  ExplorerDelegateExecute = '{11dbb47c-a525-400b-9e80-a54615a090c0}';

function MTTFolderCommand: String;
begin
  Result := '"' + ExpandConstant('{app}\{#MyAppExeName}') + '" --open-folder "%1"';
end;

function MTTWinECommand: String;
begin
  Result := '"' + ExpandConstant('{app}\{#MyAppExeName}') + '"';
end;

#include "win_e_restore.iss"

procedure RestoreExplorerDefaultManagerIfOwned;
var
  CurrentCommand: String;
begin
  if RegQueryStringValue(HKCU, DefaultFolderOpenCommandKey, '', CurrentCommand) and
    (CompareText(CurrentCommand, MTTFolderCommand) = 0) then
  begin
    RegWriteStringValue(HKCU, 'Software\Classes\Folder\shell\open',
      'MultiSelectModel', 'Document');
    RegWriteExpandStringValue(HKCU, DefaultFolderOpenCommandKey, '',
      '%SystemRoot%\Explorer.exe');
    RegWriteStringValue(HKCU, DefaultFolderOpenCommandKey,
      'DelegateExecute', ExplorerDelegateExecute);
  end;

  if RegQueryStringValue(HKCU, DefaultFolderExploreCommandKey, '', CurrentCommand) and
    (CompareText(CurrentCommand, MTTFolderCommand) = 0) then
  begin
    RegWriteDWordValue(HKCU, 'Software\Classes\Folder\shell\explore',
      'LaunchExplorerFlags', $18);
    RegWriteStringValue(HKCU, 'Software\Classes\Folder\shell\explore',
      'MultiSelectModel', 'Document');
    RegWriteStringValue(HKCU, 'Software\Classes\Folder\shell\explore',
      'ProgrammaticAccessOnly', '');
    RegDeleteValue(HKCU, DefaultFolderExploreCommandKey, '');
    RegWriteStringValue(HKCU, DefaultFolderExploreCommandKey,
      'DelegateExecute', ExplorerDelegateExecute);
  end;

  if not RestoreWinEIfOwned then
    Log('Could not restore Win+E completely; remaining recovery data was retained.');
end;

function DeleteProgramDataCache: Boolean;
var
  Attempts: Integer;
begin
  Result := not DirExists(ProgramDataCacheDir);
  Attempts := 0;
  while (not Result) and (Attempts < 60) do
  begin
    Log('Deleting MTT ProgramData cache directory: ' + ProgramDataCacheDir);
    DelTree(ProgramDataCacheDir, True, True, True);
    Result := not DirExists(ProgramDataCacheDir);
    if not Result then
      Sleep(500);
    Attempts := Attempts + 1;
  end;

  if not Result then
    Log('Failed to delete MTT ProgramData cache directory after retries: ' + ProgramDataCacheDir);
end;

function IsSecureInstallDirectory: Boolean;
begin
  Result := CompareText(
    ExpandConstant('{app}'),
    ExpandConstant('{autopf}\{#MyAppName}')) = 0;
end;

function StartSearchServiceAfterFailedCleanup: Boolean;
var
  ResultCode: Integer;
  Attempts: Integer;
begin
  Result := False;
  Attempts := 0;
  while (not Result) and (Attempts < 60) do
  begin
    ResultCode := -1;
    if Exec(ExpandConstant('{sys}\sc.exe'), 'start {#MySearchName}', '', SW_HIDE,
      ewWaitUntilTerminated, ResultCode) and (ResultCode = 0) then
      Result := True
    else
      Sleep(500);
    Attempts := Attempts + 1;
  end;
end;

// Query the numeric ServiceControllerStatus through the trusted system
// PowerShell so setup does not depend on localized `sc query` output.
function IsSearchServiceStopped: Boolean;
var
  ResultCode: Integer;
begin
  ResultCode := -1;
  Result := Exec(
    ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -Command "$s=Get-Service -Name ''{#MySearchName}'' ' +
    '-ErrorAction SilentlyContinue; if ($null -eq $s -or [int]$s.Status -eq 1) ' +
    '{ exit 0 } else { exit 1 }"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

function StopSearchServiceIfRunning(var WasRunning: Boolean): Boolean;
var
  ResultCode: Integer;
  Attempts: Integer;
begin
  WasRunning := not IsSearchServiceStopped;
  if not WasRunning then
  begin
    Result := True;
    Exit;
  end;

  ResultCode := -1;
  Exec(ExpandConstant('{sys}\sc.exe'), 'stop {#MySearchName}', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode);

  Result := False;
  Attempts := 0;
  while (not Result) and (Attempts < 60) do
  begin
    Result := IsSearchServiceStopped;
    if not Result then
      Sleep(500);
    Attempts := Attempts + 1;
  end;
end;

// Before Inno Setup copies any files, stop the running service so that:
//   1. The .exe is not locked and can be replaced.
//   2. The service cannot be killed mid-write leaving .bin.tmp on disk.
procedure CurStepChanged(CurStep: TSetupStep);
var
  ServiceWasRunning: Boolean;
begin
  if CurStep = ssInstall then
  begin
    if not IsSecureInstallDirectory then
      RaiseException(CustomMessage('SecureInstallDirectoryError'));

    if not StopSearchServiceIfRunning(ServiceWasRunning) then
    begin
      if ServiceWasRunning then
        StartSearchServiceAfterFailedCleanup;
      RaiseException(CustomMessage('SearchServiceStopTimeout'));
    end;

    // Remove cache on upgrades and any cache pre-created before a fresh
    // install. Only this regenerable ProgramData directory is affected.
    if not DeleteProgramDataCache then
    begin
      if ServiceWasRunning and not StartSearchServiceAfterFailedCleanup then
        Log('Failed to restart the previous search service after cache cleanup failure.');
      RaiseException(CustomMessage('CacheCleanupError'));
    end;
  end;

  // Persist the language selected during installation so the app can
  // start in the same language on first launch. The app reads this key
  // from HKLM (read-only, no admin required) and persists it to SQLite.
  // Cleanup happens in CurUninstallStepChanged.
  if CurStep = ssPostInstall then
  begin
    if ActiveLanguage = 'english' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'en')
    else if ActiveLanguage = 'portuguese' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'pt-BR')
    else if ActiveLanguage = 'chinese' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'zh-CN')
    else if ActiveLanguage = 'russian' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'ru')
    else if ActiveLanguage = 'greek' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'el');
    RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallGeneration',
      GetDateTimeString('yyyy-mm-dd hh:nn:ss.zzz', '-', ':'));
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    RestoreExplorerDefaultManagerIfOwned;

  if CurUninstallStep = usPostUninstall then
  begin
    if not DeleteProgramDataCache then
      Log('Search index cache could not be fully removed during uninstall.');
    RegDeleteValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage');
    RegDeleteKeyIncludingSubkeys(HKLM, 'SOFTWARE\MTT-File-Manager');
  end;
end;

[CustomMessages]
#include "vcredist_messages.iss"
english.InstallSearchService=Installing search service...
english.StartSearchService=Starting search service...
english.SecureInstallDirectoryError=MTT File Manager must be installed in its protected Program Files directory.
english.SearchServiceStopTimeout=The search service did not stop within 30 seconds. Setup was stopped before updating any files.
english.CacheCleanupError=Unable to securely remove the previous search index cache. Setup was stopped before installing the update.

portuguese.InstallSearchService=Instalando o serviço de busca...
portuguese.StartSearchService=Iniciando o serviço de busca...
portuguese.SecureInstallDirectoryError=O MTT File Manager deve ser instalado na pasta protegida Arquivos de Programas.
portuguese.SearchServiceStopTimeout=O serviço de busca não foi interrompido em 30 segundos. A instalação foi interrompida antes da atualização dos arquivos.
portuguese.CacheCleanupError=Não foi possível remover com segurança o cache anterior do índice de busca. A instalação foi interrompida antes da atualização.

chinese.InstallSearchService=正在安装搜索服务...
chinese.StartSearchService=正在启动搜索服务...
chinese.SecureInstallDirectoryError=MTT File Manager 必须安装在受保护的“程序文件”目录中。
chinese.SearchServiceStopTimeout=搜索服务未能在 30 秒内停止。安装程序已停止，尚未更新任何文件。
chinese.CacheCleanupError=无法安全删除之前的搜索索引缓存。安装程序已停止，尚未安装更新。

russian.InstallSearchService=Установка службы поиска...
russian.StartSearchService=Запуск службы поиска...
russian.SecureInstallDirectoryError=MTT File Manager необходимо установить в защищённую папку Program Files.
russian.SearchServiceStopTimeout=Служба поиска не остановилась за 30 секунд. Установка прервана до обновления файлов.
russian.CacheCleanupError=Не удалось безопасно удалить предыдущий кэш индекса поиска. Установка прервана до установки обновления.

greek.InstallSearchService=Εγκατάσταση της υπηρεσίας αναζήτησης...
greek.StartSearchService=Εκκίνηση της υπηρεσίας αναζήτησης...
greek.SecureInstallDirectoryError=Το MTT File Manager πρέπει να εγκατασταθεί στον προστατευμένο κατάλογο Program Files.
greek.SearchServiceStopTimeout=Η υπηρεσία αναζήτησης δεν σταμάτησε εντός 30 δευτερολέπτων. Η εγκατάσταση διακόπηκε πριν ενημερωθούν αρχεία.
greek.CacheCleanupError=Δεν ήταν δυνατή η ασφαλής αφαίρεση της προηγούμενης cache του ευρετηρίου αναζήτησης. Η εγκατάσταση διακόπηκε πριν από την εγκατάσταση της ενημέρωσης.
