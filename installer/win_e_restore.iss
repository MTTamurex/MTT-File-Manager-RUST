// Restore only this installation's per-user Win+E override. The backup schema
// is shared with src/infrastructure/windows/default_file_manager/win_e.rs.
const
  DefaultWinECommandKey = 'Software\Classes\CLSID\{52205fd8-5dfb-447d-801a-d0b52f2e83e1}\shell\opennewwindow\command';
  WinEStateKey = 'Software\MTT-File-Manager\ShellIntegration\WinE';

type
  TWinEValue = record
    Present: Boolean;
    Text: String;
  end;

#ifndef WinERestoreTest
function WinERegOpenKeyEx(Root: Integer; KeyPath: String; Options, Access: Cardinal;
  var Key: LongWord): Integer;
  external 'RegOpenKeyExW@advapi32.dll stdcall';
function WinERegQueryValueEx(Key: LongWord; Name: String; Reserved: Cardinal;
  var ValueType: Cardinal; Data: Cardinal; var Size: Cardinal): Integer;
  external 'RegQueryValueExW@advapi32.dll stdcall';
function WinERegCloseKey(Key: LongWord): Integer;
  external 'RegCloseKey@advapi32.dll stdcall';

function WinEReadValue(KeyPath, Name: String; var Present: Boolean;
  var Text: String): Boolean;
var
  Key: LongWord;
  Access, ValueType, Size: Cardinal;
  Status: Integer;
begin
  Result := False;
  Present := False;
  Text := '';
  // RegValueExists alone cannot distinguish absence from access denied. Use
  // the same registry view as Inno's HKCU functions and fail closed on errors.
  Access := $0001; // KEY_QUERY_VALUE
  if Is64BitInstallMode then
    Access := Access or $0100 // KEY_WOW64_64KEY
  else
    Access := Access or $0200; // KEY_WOW64_32KEY
  Status := WinERegOpenKeyEx(HKCU, KeyPath, 0, Access, Key);
  if (Status = 2) or (Status = 3) then // Missing key/path
  begin
    Result := True;
    Exit;
  end;
  if Status <> 0 then
    Exit;
  try
    Size := 0;
    Status := WinERegQueryValueEx(Key, Name, 0, ValueType, 0, Size);
    if Status = 2 then // Missing value
    begin
      Result := True;
      Exit;
    end;
    if (Status <> 0) or ((ValueType <> 1) and (ValueType <> 2)) then
      Exit;
    Present := True;
    Result := RegQueryStringValue(HKCU, KeyPath, Name, Text);
  finally
    WinERegCloseKey(Key);
  end;
end;

function WinEWriteValue(KeyPath, Name, Text: String): Boolean;
begin
  Result := RegWriteStringValue(HKCU, KeyPath, Name, Text);
end;

function WinEDeleteValue(KeyPath, Name: String): Boolean;
var
  Present: Boolean;
  Text: String;
begin
  Result := WinEReadValue(KeyPath, Name, Present, Text);
  if Result and Present then
    Result := RegDeleteValue(HKCU, KeyPath, Name);
end;

procedure WinEDeleteEmptyStateKey;
begin
  // An unrelated value/subkey must not prevent restoration or be deleted.
  RegDeleteKeyIfEmpty(HKCU, WinEStateKey);
end;
#endif

function WinEValuesEqual(Left, Right: TWinEValue): Boolean;
begin
  Result := (Left.Present = Right.Present) and
    ((not Left.Present) or (Left.Text = Right.Text));
end;

function WinEReadPrevious(PreviousName, HadName: String;
  var Previous: TWinEValue): Boolean;
var
  Had: TWinEValue;
begin
  Result := False;
  Previous.Present := False;
  Previous.Text := '';
  if not WinEReadValue(WinEStateKey, HadName, Had.Present, Had.Text) then
    Exit;
  if not Had.Present then
    Exit;
  if Had.Text = '0' then
  begin
    Result := True;
    Exit;
  end;
  if Had.Text <> '1' then
    Exit;
  Result := WinEReadValue(WinEStateKey, PreviousName,
    Previous.Present, Previous.Text) and Previous.Present;
end;

function WinEApplyValue(Name: String; Value: TWinEValue): Boolean;
begin
  if Value.Present then
    Result := WinEWriteValue(DefaultWinECommandKey, Name, Value.Text)
  else
    Result := WinEDeleteValue(DefaultWinECommandKey, Name);
end;

function WinEClearOwnedBackup: Boolean;
begin
  // Delete ownership last so a failed cleanup can be retried without touching
  // another installation's backup or unrelated state values.
  Result := WinEDeleteValue(WinEStateKey, 'PreviousDefault') and
    WinEDeleteValue(WinEStateKey, 'HadDefault') and
    WinEDeleteValue(WinEStateKey, 'PreviousDelegateExecute') and
    WinEDeleteValue(WinEStateKey, 'HadDelegateExecute');
  if not Result then
    Exit;
  Result := WinEDeleteValue(WinEStateKey, 'Command');
  if Result then
    WinEDeleteEmptyStateKey;
end;

procedure WinERollbackDelegate(Previous, Written: TWinEValue);
var
  Current: TWinEValue;
begin
  if WinEReadValue(DefaultWinECommandKey, 'DelegateExecute',
    Current.Present, Current.Text) and WinEValuesEqual(Current, Written) then
  begin
    if not WinEApplyValue('DelegateExecute', Previous) then
      Log('Win+E delegate rollback failed; backup retained.');
  end;
end;

function RestoreWinEIfOwned: Boolean;
var
  Owner, CurrentCommand, CurrentDelegate, PreviousCommand,
    PreviousDelegate, Check: TWinEValue;
  HasBackup, RestoreDelegate: Boolean;
begin
  Result := False;
  if not WinEReadValue(WinEStateKey, 'Command', Owner.Present, Owner.Text) then
    Exit;
  HasBackup := Owner.Present;
  if HasBackup and (CompareText(Owner.Text, MTTWinECommand) <> 0) then
  begin
    Result := True;
    Exit;
  end;
  if not WinEReadValue(DefaultWinECommandKey, '',
    CurrentCommand.Present, CurrentCommand.Text) then
    Exit;
  if (not CurrentCommand.Present) or
    (CompareText(CurrentCommand.Text, MTTWinECommand) <> 0) then
  begin
    Result := True;
    if HasBackup then
      Result := WinEClearOwnedBackup;
    Exit;
  end;
  if not WinEReadValue(DefaultWinECommandKey, 'DelegateExecute',
    CurrentDelegate.Present, CurrentDelegate.Text) then
    Exit;

  PreviousCommand.Present := False;
  PreviousCommand.Text := '';
  PreviousDelegate := PreviousCommand;
  RestoreDelegate := CurrentDelegate.Present and (CurrentDelegate.Text = '');
  if HasBackup then
  begin
    // Validate the required backup values before changing either live value.
    if not WinEReadPrevious('PreviousDefault', 'HadDefault', PreviousCommand) then
      Exit;
    if RestoreDelegate and
      (not WinEReadPrevious('PreviousDelegateExecute', 'HadDelegateExecute',
        PreviousDelegate)) then
      Exit;
  end;

  if not WinEReadValue(DefaultWinECommandKey, 'DelegateExecute',
    Check.Present, Check.Text) then
    Exit;
  if not WinEValuesEqual(Check, CurrentDelegate) then
    Exit;
  if not WinEReadValue(DefaultWinECommandKey, '', Check.Present, Check.Text) then
    Exit;
  if not WinEValuesEqual(Check, CurrentCommand) then
    Exit;

  // Keep our command selected until its delegate has been restored. If the
  // command write fails, undo only the delegate value we actually wrote.
  if RestoreDelegate then
    if not WinEApplyValue('DelegateExecute', PreviousDelegate) then
      Exit;
  if not WinEReadValue(DefaultWinECommandKey, '', Check.Present, Check.Text) then
  begin
    if RestoreDelegate then
      WinERollbackDelegate(CurrentDelegate, PreviousDelegate);
    Exit;
  end;
  if not WinEValuesEqual(Check, CurrentCommand) then
  begin
    if RestoreDelegate then
      WinERollbackDelegate(CurrentDelegate, PreviousDelegate);
    Exit;
  end;
  if not WinEApplyValue('', PreviousCommand) then
  begin
    if RestoreDelegate then
      WinERollbackDelegate(CurrentDelegate, PreviousDelegate);
    Exit;
  end;

  Result := True;
  if HasBackup then
    Result := WinEClearOwnedBackup;
end;
