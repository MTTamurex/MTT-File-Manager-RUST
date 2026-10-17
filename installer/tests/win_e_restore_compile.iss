// Compile the real registry adapter as well as the in-memory test adapter.
[Setup]
AppName=WinE restore compile validation
AppVersion=1
DefaultDirName={tmp}\WinERestoreCompile
PrivilegesRequired=lowest
#ifdef WinERestore64
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
#endif
CreateAppDir=no
Uninstallable=no
OutputBaseFilename=win-e-restore-compile
Compression=none

[Code]
function MTTWinECommand: String;
begin
  Result := '"C:\Apps\Example\mtt-file-manager.exe"';
end;

#include "..\win_e_restore.iss"

function InitializeSetup: Boolean;
var
  Present: Boolean;
  Text, Expected, ResultPath: String;
begin
  // Exercise the native adapter read-only. Never restore/delete a live value.
  Result := False;
  if not WinEReadValue('Software\WinERestoreTests\Missing-' +
    GetDateTimeString('yyyymmddhhnnsszzz', '-', ':'), 'Command', Present, Text) then
    RaiseException('Native adapter failed to read an absent key');
  if Present then
    RaiseException('Unexpected key in native adapter fixture');
  if not WinEReadValue('Software', 'WinERestoreTests-MissingValue-' +
    GetDateTimeString('yyyymmddhhnnsszzz', '-', ':'), Present, Text) then
    RaiseException('Native adapter failed to read an absent value');
  if Present then
    RaiseException('Unexpected value in native adapter fixture');
  if RegQueryStringValue(HKCU, 'Environment', 'TEMP', Expected) then
  begin
    if not WinEReadValue('Environment', 'TEMP', Present, Text) then
      RaiseException('Native adapter failed to read an existing string');
    if (not Present) or (Text <> Expected) then
      RaiseException('Native adapter returned a different string');
  end;
  ResultPath := ExpandConstant('{param:TestResult}');
  if ResultPath = '' then
    RaiseException('TestResult parameter is required');
  if not SaveStringToFile(ResultPath, 'native adapter read checks passed', False) then
    RaiseException('Could not write native adapter result');
end;
