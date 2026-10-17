// Runs the production restore algorithm against an in-memory registry only.
#define WinERestoreTest

[Setup]
AppName=WinE restore regression tests
AppVersion=1
DefaultDirName={tmp}\WinERestoreTests
PrivilegesRequired=lowest
CreateAppDir=no
Uninstallable=no
OutputBaseFilename=win-e-restore-tests
Compression=none
SetupLogging=yes

[Code]
var
  MockRegistry: TStringList;
  Mutations, FailAt, CommandReads, ChangeCommandAtRead, TestsPassed: Integer;
  UnreadableName: String;

function MTTWinECommand: String;
begin
  Result := '"C:\Apps\Example\mtt-file-manager.exe"';
end;

function ValueIndex(KeyPath, Name: String): Integer;
var
  Index: Integer;
  Prefix: String;
begin
  Result := -1;
  Prefix := KeyPath + '|' + Name + '=';
  for Index := 0 to MockRegistry.Count - 1 do
    if CompareText(Copy(MockRegistry.Strings[Index], 1, Length(Prefix)), Prefix) = 0 then
    begin
      Result := Index;
      Exit;
    end;
end;

procedure PutValue(KeyPath, Name, Text: String);
var
  Index: Integer;
begin
  Index := ValueIndex(KeyPath, Name);
  if Index < 0 then
    MockRegistry.Add(KeyPath + '|' + Name + '=' + Text)
  else
    MockRegistry.Strings[Index] := KeyPath + '|' + Name + '=' + Text;
end;

function WinEReadValue(KeyPath, Name: String; var Present: Boolean;
  var Text: String): Boolean;
var
  Index, PrefixLength: Integer;
begin
  if Name = '' then
  begin
    CommandReads := CommandReads + 1;
    if (ChangeCommandAtRead > 0) and (CommandReads = ChangeCommandAtRead) then
      PutValue(KeyPath, Name, 'another application');
  end;
  Index := ValueIndex(KeyPath, Name);
  Present := Index >= 0;
  Text := '';
  if Present then
  begin
    PrefixLength := Length(KeyPath + '|' + Name + '=');
    Text := Copy(MockRegistry.Strings[Index], PrefixLength + 1, MaxInt);
  end;
  Result := (UnreadableName = '') or (Name <> UnreadableName);
end;

function BeforeMutation: Boolean;
begin
  Mutations := Mutations + 1;
  Result := (FailAt = 0) or (Mutations <> FailAt);
end;

function WinEWriteValue(KeyPath, Name, Text: String): Boolean;
begin
  Result := BeforeMutation;
  if Result then
    PutValue(KeyPath, Name, Text);
end;

function WinEDeleteValue(KeyPath, Name: String): Boolean;
var
  Index: Integer;
begin
  Result := BeforeMutation;
  if Result then
  begin
    Index := ValueIndex(KeyPath, Name);
    if Index >= 0 then
      MockRegistry.Delete(Index);
  end;
end;

procedure WinEDeleteEmptyStateKey;
begin
  // No keys exist in this adapter, only values. Never access the real registry.
end;

#include "..\win_e_restore.iss"

procedure Check(Condition: Boolean; Description: String);
begin
  if not Condition then
    RaiseException(Description);
end;

procedure CheckValue(KeyPath, Name: String; Present: Boolean; Text: String);
var
  Actual: TWinEValue;
begin
  Check(WinEReadValue(KeyPath, Name, Actual.Present, Actual.Text), 'Read failed');
  Check(Actual.Present = Present, 'Unexpected value presence: ' + Name);
  if Present then
    Check(Actual.Text = Text, 'Unexpected value content: ' + Name);
end;

procedure ResetRegistry;
begin
  MockRegistry.Clear;
  Mutations := 0;
  FailAt := 0;
  CommandReads := 0;
  ChangeCommandAtRead := 0;
  UnreadableName := '';
end;

procedure SeedBackup(CommandMode, DelegateMode: Integer);
begin
  ResetRegistry;
  PutValue(DefaultWinECommandKey, '', MTTWinECommand);
  PutValue(DefaultWinECommandKey, 'DelegateExecute', '');
  PutValue(WinEStateKey, 'Command', MTTWinECommand);
  if CommandMode = 0 then
    PutValue(WinEStateKey, 'HadDefault', '0')
  else
    PutValue(WinEStateKey, 'HadDefault', '1');
  if CommandMode = 2 then
    PutValue(WinEStateKey, 'PreviousDefault', 'previous command')
  else
    PutValue(WinEStateKey, 'PreviousDefault', '');
  if DelegateMode = 0 then
    PutValue(WinEStateKey, 'HadDelegateExecute', '0')
  else
    PutValue(WinEStateKey, 'HadDelegateExecute', '1');
  if DelegateMode = 2 then
    PutValue(WinEStateKey, 'PreviousDelegateExecute', 'previous delegate')
  else
    PutValue(WinEStateKey, 'PreviousDelegateExecute', '');
end;

procedure TestAbsentEmptyAndCustomValues;
var
  CommandMode, DelegateMode: Integer;
  ExpectedCommand, ExpectedDelegate, Snapshot: String;
begin
  for CommandMode := 0 to 2 do
    for DelegateMode := 0 to 2 do
    begin
      SeedBackup(CommandMode, DelegateMode);
      Check(RestoreWinEIfOwned, 'Restore failed');
      ExpectedCommand := '';
      if CommandMode = 2 then
        ExpectedCommand := 'previous command';
      ExpectedDelegate := '';
      if DelegateMode = 2 then
        ExpectedDelegate := 'previous delegate';
      CheckValue(DefaultWinECommandKey, '', CommandMode <> 0, ExpectedCommand);
      CheckValue(DefaultWinECommandKey, 'DelegateExecute', DelegateMode <> 0, ExpectedDelegate);
      CheckValue(WinEStateKey, 'Command', False, '');
      Snapshot := MockRegistry.Text;
      Check(RestoreWinEIfOwned, 'Repeated restore failed');
      Check(MockRegistry.Text = Snapshot, 'Restore is not idempotent');
      TestsPassed := TestsPassed + 1;
    end;
end;

procedure TestThirdPartyChangesAndOwnership;
var
  Snapshot: String;
begin
  SeedBackup(2, 2);
  PutValue(DefaultWinECommandKey, '', 'another application');
  PutValue(DefaultWinECommandKey, 'DelegateExecute', 'another delegate');
  Check(RestoreWinEIfOwned, 'Changed command restore failed');
  CheckValue(DefaultWinECommandKey, '', True, 'another application');
  CheckValue(DefaultWinECommandKey, 'DelegateExecute', True, 'another delegate');
  TestsPassed := TestsPassed + 1;

  SeedBackup(2, 2);
  PutValue(DefaultWinECommandKey, 'DelegateExecute', 'another delegate');
  PutValue(WinEStateKey, 'Unrelated', 'keep');
  Check(RestoreWinEIfOwned, 'Changed delegate restore failed');
  CheckValue(DefaultWinECommandKey, '', True, 'previous command');
  CheckValue(DefaultWinECommandKey, 'DelegateExecute', True, 'another delegate');
  CheckValue(WinEStateKey, 'Unrelated', True, 'keep');
  TestsPassed := TestsPassed + 1;

  SeedBackup(2, 2);
  PutValue(WinEStateKey, 'Command', '"C:\Apps\Other\mtt-file-manager.exe"');
  Snapshot := MockRegistry.Text;
  Check(RestoreWinEIfOwned, 'Foreign backup handling failed');
  Check(MockRegistry.Text = Snapshot, 'Foreign backup was modified');
  TestsPassed := TestsPassed + 1;
end;

procedure TestLegacyRegistration;
begin
  ResetRegistry;
  PutValue(DefaultWinECommandKey, '', MTTWinECommand);
  PutValue(DefaultWinECommandKey, 'DelegateExecute', '');
  Check(RestoreWinEIfOwned, 'Legacy restore failed');
  Check(MockRegistry.Count = 0, 'Legacy owned override remains');
  TestsPassed := TestsPassed + 1;

  ResetRegistry;
  PutValue(DefaultWinECommandKey, '', MTTWinECommand);
  PutValue(DefaultWinECommandKey, 'DelegateExecute', 'another delegate');
  Check(RestoreWinEIfOwned, 'Legacy custom delegate restore failed');
  CheckValue(DefaultWinECommandKey, 'DelegateExecute', True, 'another delegate');
  TestsPassed := TestsPassed + 1;
end;

procedure TestRemovedValuesAndCase;
begin
  SeedBackup(2, 2);
  MockRegistry.Delete(ValueIndex(DefaultWinECommandKey, 'DelegateExecute'));
  Check(RestoreWinEIfOwned, 'Removed delegate handling failed');
  CheckValue(DefaultWinECommandKey, '', True, 'previous command');
  CheckValue(DefaultWinECommandKey, 'DelegateExecute', False, '');
  TestsPassed := TestsPassed + 1;

  SeedBackup(2, 2);
  MockRegistry.Delete(ValueIndex(DefaultWinECommandKey, ''));
  Check(RestoreWinEIfOwned, 'Removed command handling failed');
  CheckValue(DefaultWinECommandKey, '', False, '');
  CheckValue(DefaultWinECommandKey, 'DelegateExecute', True, '');
  TestsPassed := TestsPassed + 1;

  SeedBackup(2, 2);
  PutValue(DefaultWinECommandKey, '', Uppercase(MTTWinECommand));
  PutValue(WinEStateKey, 'Command', Uppercase(MTTWinECommand));
  Check(RestoreWinEIfOwned, 'Case-insensitive ownership handling failed');
  CheckValue(DefaultWinECommandKey, '', True, 'previous command');
  TestsPassed := TestsPassed + 1;
end;

procedure TestIncompleteAndUnreadableBackups;
var
  Snapshot: String;
begin
  SeedBackup(2, 2);
  PutValue(WinEStateKey, 'HadDefault', 'invalid');
  Snapshot := MockRegistry.Text;
  Check(not RestoreWinEIfOwned, 'Invalid backup accepted');
  Check(MockRegistry.Text = Snapshot, 'Invalid backup changed live settings');
  TestsPassed := TestsPassed + 1;

  SeedBackup(2, 2);
  MockRegistry.Delete(ValueIndex(WinEStateKey, 'PreviousDelegateExecute'));
  Snapshot := MockRegistry.Text;
  Check(not RestoreWinEIfOwned, 'Incomplete backup accepted');
  Check(MockRegistry.Text = Snapshot, 'Incomplete backup changed live settings');
  TestsPassed := TestsPassed + 1;

  SeedBackup(2, 2);
  UnreadableName := 'Command';
  Snapshot := MockRegistry.Text;
  Check(not RestoreWinEIfOwned, 'Unreadable owner accepted');
  Check(MockRegistry.Text = Snapshot, 'Unreadable owner changed settings');
  TestsPassed := TestsPassed + 1;
end;

procedure TestFailuresRetainBackupAndCanRetry;
var
  Failure: Integer;
  Snapshot: String;
begin
  for Failure := 1 to 7 do
  begin
    SeedBackup(2, 2);
    Snapshot := MockRegistry.Text;
    FailAt := Failure;
    Check(not RestoreWinEIfOwned, 'Injected mutation failure was ignored');
    CheckValue(WinEStateKey, 'Command', True, MTTWinECommand);
    if Failure <= 2 then
      Check(MockRegistry.Text = Snapshot, 'Failed restore did not roll back');
    FailAt := 0;
    Check(RestoreWinEIfOwned, 'Retry failed');
    CheckValue(DefaultWinECommandKey, '', True, 'previous command');
    CheckValue(DefaultWinECommandKey, 'DelegateExecute', True, 'previous delegate');
    TestsPassed := TestsPassed + 1;
  end;
end;

procedure TestConcurrentCommandChange;
begin
  SeedBackup(2, 2);
  ChangeCommandAtRead := 3;
  Check(not RestoreWinEIfOwned, 'Concurrent command change was ignored');
  CheckValue(DefaultWinECommandKey, '', True, 'another application');
  CheckValue(DefaultWinECommandKey, 'DelegateExecute', True, '');
  CheckValue(WinEStateKey, 'Command', True, MTTWinECommand);
  TestsPassed := TestsPassed + 1;
end;

function InitializeSetup: Boolean;
var
  ResultPath: String;
begin
  Result := False; // Never install anything, even when all assertions pass.
  ResultPath := ExpandConstant('{param:TestResult}');
  if ResultPath = '' then
    RaiseException('TestResult parameter is required');
  MockRegistry := TStringList.Create;
  try
    TestAbsentEmptyAndCustomValues;
    TestThirdPartyChangesAndOwnership;
    TestLegacyRegistration;
    TestRemovedValuesAndCase;
    TestIncompleteAndUnreadableBackups;
    TestFailuresRetainBackupAndCanRetry;
    TestConcurrentCommandChange;
    Check(SaveStringToFile(ResultPath, IntToStr(TestsPassed) + ' cases passed', False),
      'Could not write test result');
  finally
    MockRegistry.Free;
  end;
end;
