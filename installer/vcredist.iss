// Guided prerequisite setup. Runs before app files or services are changed.
const
  VCRedistURL = 'https://aka.ms/vs/17/release/vc_redist.x64.exe';

var
  VCRedistPage: TWizardPage;
  VCRedistStatus: TNewStaticText;

function IsVCRedistInstalled: Boolean;
var
  Installed: Cardinal;
begin
  Result := RegQueryDWordValue(HKLM64,
    'SOFTWARE\Microsoft\VisualStudio\14.0\VC\Runtimes\x64',
    'Installed', Installed) and (Installed = 1);
end;

procedure RefreshVCRedistStatus;
begin
  if IsVCRedistInstalled then
    VCRedistStatus.Caption := CustomMessage('VCRedistReady')
  else
    VCRedistStatus.Caption := CustomMessage('VCRedistMissing');
end;

procedure DownloadVCRedist(Sender: TObject);
var
  ErrorCode: Integer;
begin
  if not ShellExecAsOriginalUser('open', VCRedistURL, '', '', SW_SHOWNORMAL,
    ewNoWait, ErrorCode) then
    MsgBox(CustomMessage('VCRedistOpenFailed') + #13#10 + VCRedistURL,
      mbError, MB_OK);
end;

procedure RecheckVCRedist(Sender: TObject);
begin
  RefreshVCRedistStatus;
end;

procedure InitializeWizard;
var
  Instructions: TNewStaticText;
  DownloadButton, RecheckButton: TNewButton;
begin
  VCRedistPage := CreateCustomPage(wpSelectTasks,
    CustomMessage('VCRedistTitle'), CustomMessage('VCRedistSubtitle'));

  Instructions := TNewStaticText.Create(VCRedistPage);
  Instructions.Parent := VCRedistPage.Surface;
  Instructions.AutoSize := False;
  Instructions.WordWrap := True;
  Instructions.Width := VCRedistPage.SurfaceWidth;
  Instructions.Height := ScaleY(100);
  Instructions.Caption := CustomMessage('VCRedistInstructions');

  DownloadButton := TNewButton.Create(VCRedistPage);
  DownloadButton.Parent := VCRedistPage.Surface;
  DownloadButton.Top := ScaleY(108);
  DownloadButton.Width := VCRedistPage.SurfaceWidth;
  DownloadButton.Height := ScaleY(28);
  DownloadButton.Caption := CustomMessage('VCRedistDownloadButton');
  DownloadButton.OnClick := @DownloadVCRedist;

  RecheckButton := TNewButton.Create(VCRedistPage);
  RecheckButton.Parent := VCRedistPage.Surface;
  RecheckButton.Top := ScaleY(144);
  RecheckButton.Width := VCRedistPage.SurfaceWidth;
  RecheckButton.Height := ScaleY(28);
  RecheckButton.Caption := CustomMessage('VCRedistRecheckButton');
  RecheckButton.OnClick := @RecheckVCRedist;

  VCRedistStatus := TNewStaticText.Create(VCRedistPage);
  VCRedistStatus.Parent := VCRedistPage.Surface;
  VCRedistStatus.Top := ScaleY(184);
  VCRedistStatus.AutoSize := False;
  VCRedistStatus.WordWrap := True;
  VCRedistStatus.Width := VCRedistPage.SurfaceWidth;
  VCRedistStatus.Height := ScaleY(44);
  RefreshVCRedistStatus;
end;

function ShouldSkipPage(PageID: Integer): Boolean;
begin
  Result := (PageID = VCRedistPage.ID) and IsVCRedistInstalled;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = VCRedistPage.ID then
    RefreshVCRedistStatus;
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if CurPageID = VCRedistPage.ID then
  begin
    RefreshVCRedistStatus;
    Result := IsVCRedistInstalled;
    if not Result then
      MsgBox(CustomMessage('VCRedistMissing') + #13#10#13#10 +
        CustomMessage('VCRedistInstructions'), mbInformation, MB_OK);
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  // Also protects silent installs and a prerequisite removed after page checks.
  if not IsVCRedistInstalled then
    Result := CustomMessage('VCRedistMissing') + #13#10 + VCRedistURL;
end;
