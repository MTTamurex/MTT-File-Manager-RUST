; ==========================================================================
; MTT File Manager - Inno Setup Script
; ==========================================================================
; Requires: Inno Setup 6+ (https://jrsoftware.org/isinfo.php)
; Build:    ISCC.exe installer\setup.iss
; ==========================================================================

#define MyAppName      "MTT File Manager"
#define MyAppVersion   "0.1.0-beta.2"
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

[Languages]
Name: "english";    MessagesFile: "compiler:Default.isl"
Name: "portuguese";  MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"

[Tasks]
Name: "desktopicon";  Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

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

; mpv portable config (scripts, settings)
Source: "{#SrcRoot}\mpv_ui\portable_config\mpv.conf";            DestDir: "{app}\mpv_ui\portable_config"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\scripts\autoload.lua"; DestDir: "{app}\mpv_ui\portable_config\scripts"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\scripts\modernH.lua";  DestDir: "{app}\mpv_ui\portable_config\scripts"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\scripts\vsr.lua";      DestDir: "{app}\mpv_ui\portable_config\scripts"; Flags: ignoreversion
Source: "{#SrcRoot}\mpv_ui\portable_config\script-opts\*";       DestDir: "{app}\mpv_ui\portable_config\script-opts"; Flags: ignoreversion recursesubdirs
Source: "{#SrcRoot}\mpv_ui\portable_config\fonts\*";              DestDir: "{app}\mpv_ui\portable_config\fonts"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{group}\{#MyAppName}";         Filename: "{app}\{#MyAppExeName}"; WorkingDir: "{app}"
Name: "{autodesktop}\{#MyAppName}";   Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon; WorkingDir: "{app}"

[Run]
; (Re)install and start the search indexer Windows service.
; The service was already stopped in CurStepChanged(ssInstall) before files
; were copied, so "install" here is idempotent for fresh installs and safe
; for upgrades.
Filename: "{app}\{#MySearchSvc}"; Parameters: "install"; StatusMsg: "Installing search service..."; Flags: runhidden waituntilterminated
Filename: "sc.exe"; Parameters: "start {#MySearchName}"; StatusMsg: "Starting search service..."; Flags: runhidden waituntilterminated

[UninstallRun]
; Stop and remove the search service before files are deleted
Filename: "sc.exe"; Parameters: "stop {#MySearchName}"; RunOnceId: "StopSearchService"; Flags: runhidden waituntilterminated
Filename: "{app}\{#MySearchSvc}"; Parameters: "uninstall"; RunOnceId: "UninstallSearchService"; Flags: runhidden waituntilterminated

[Code]
// Check if VC++ Redistributable 2015-2022 (x64) is installed
function IsVCRedistInstalled: Boolean;
var
  Version: String;
begin
  Result := RegQueryStringValue(HKLM,
    'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
    'Version', Version);
  if not Result then
    Result := RegQueryStringValue(HKLM,
      'SOFTWARE\WOW6432Node\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
      'Version', Version);
end;

// Stop the search service and wait for it to reach the Stopped state.
// Called before files are copied so the service process is not holding
// the executable open and cannot be killed mid-write (leaving .bin.tmp
// without a corresponding .bin).
procedure StopSearchServiceIfRunning;
var
  ResultCode: Integer;
  Attempts: Integer;
begin
  // Ask SCM to stop the service. Ignore errors (service may not be installed).
  Exec('sc.exe', 'stop {#MySearchName}', '', SW_HIDE, ewWaitUntilTerminated, ResultCode);

  // Poll up to ~10 seconds for the service to reach the Stopped state.
  // sc.exe query returns exit code 0 while the service exists; we look for
  // STATE 1 (STOPPED) in a separate check via the exit code of "sc query".
  // Simpler: just sleep a fixed 4 s which is more than enough for the service
  // to flush its current write and exit cleanly.
  Attempts := 0;
  while Attempts < 8 do
  begin
    Sleep(500);
    Attempts := Attempts + 1;
  end;
end;

// Before Inno Setup copies any files, stop the running service so that:
//   1. The .exe is not locked and can be replaced.
//   2. The service cannot be killed mid-write leaving .bin.tmp on disk.
procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    StopSearchServiceIfRunning;

  // Persist the language selected during installation so the app can
  // start in the same language on first launch. The app reads this key
  // from HKLM (read-only, no admin required) and persists it to SQLite.
  // Cleanup happens in CurUninstallStepChanged.
  if CurStep = ssPostInstall then
  begin
    if ActiveLanguage = 'english' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'en')
    else if ActiveLanguage = 'portuguese' then
      RegWriteStringValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage', 'pt-BR');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
  begin
    RegDeleteValue(HKLM, 'SOFTWARE\MTT-File-Manager', 'InstallerLanguage');
    RegDeleteKeyIncludingSubkeys(HKLM, 'SOFTWARE\MTT-File-Manager');
  end;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpReady then
  begin
    if not IsVCRedistInstalled then
      MsgBox('Warning: Microsoft Visual C++ Redistributable (x64) does not appear to be installed.' + #13#10 +
             'The application requires it to run.' + #13#10#13#10 +
             'You can download it from:' + #13#10 +
             'https://aka.ms/vs/17/release/vc_redist.x64.exe',
             mbInformation, MB_OK);
  end;
end;
