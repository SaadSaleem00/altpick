; Inno Setup script for filepick's Setup.exe.
; Build with installer\build.ps1 (it passes AppVersion from Cargo.toml).
;
; Per-user install, no admin rights:
;   - files go to %LOCALAPPDATA%\Programs\filepick
;   - that folder is added to the user PATH (and removed on uninstall)
;   - `filepick setup` turns the hotkey on; uninstall runs `filepick remove`
; Files written by an installer carry no "downloaded from the internet"
; mark, so PowerShell loads filepick-hook.ps1 without Unblock-File.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir ".."
#endif

[Setup]
AppId={{6E3C9F2A-4B1D-4C7E-9A55-3F2D8B7C1E40}
AppName=filepick
AppVersion={#AppVersion}
AppPublisher=filepick
DefaultDirName={localappdata}\Programs\filepick
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ChangesEnvironment=yes
OutputDir={#SourceDir}\target\installer
OutputBaseFilename=filepick-{#AppVersion}-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName=filepick
UninstallDisplayIcon={app}\filepick.exe
CloseApplications=no

[Tasks]
Name: allowscripts; Description: "Allow PowerShell to load the hotkey (sets your own PowerShell execution policy to RemoteSigned, the usual setting, only if it currently blocks scripts)"

[Files]
Source: "{#SourceDir}\target\release\filepick.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\shell\filepick-hook.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\shell\filepick.lua"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\shell\filepick.bash"; DestDir: "{app}"; Flags: ignoreversion

[InstallDelete]
; The hook used to be filepick.ps1. Next to filepick.exe on PATH, PowerShell
; ran that script instead of the exe for `filepick doctor`.
Type: files; Name: "{app}\filepick.ps1"

[Run]
Filename: "{app}\filepick.exe"; Parameters: "setup --allow-scripts"; Tasks: allowscripts; Flags: runhidden; StatusMsg: "Turning on the hotkey..."
Filename: "{app}\filepick.exe"; Parameters: "setup"; Tasks: not allowscripts; Flags: runhidden; StatusMsg: "Turning on the hotkey..."

[UninstallRun]
Filename: "{app}\filepick.exe"; Parameters: "remove"; Flags: runhidden; RunOnceId: "filepick-remove"

[Messages]
FinishedLabel=filepick is installed.%n%nOpen a new terminal, start typing a command and press Alt+F.%n%nIf something doesn't work, run: filepick doctor

[Code]
const
  EnvKey = 'Environment';

{ The user PATH split on ';' contains Dir (case-insensitive, trailing \ ignored)? }
function PathHas(Paths, Dir: string): Boolean;
var
  Rest, Part: string;
  P: Integer;
begin
  Result := False;
  Rest := Paths + ';';
  Dir := Lowercase(RemoveBackslashUnlessRoot(Dir));
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Part := Trim(Copy(Rest, 1, P - 1));
    Rest := Copy(Rest, P + 1, Length(Rest));
    if Lowercase(RemoveBackslashUnlessRoot(Part)) = Dir then
    begin
      Result := True;
      Exit;
    end;
  end;
end;

procedure AddToPath(Dir: string);
var
  Paths: string;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Paths) then
    Paths := '';
  if PathHas(Paths, Dir) then
    Exit;
  if (Paths <> '') and (Copy(Paths, Length(Paths), 1) <> ';') then
    Paths := Paths + ';';
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Paths + Dir);
end;

procedure RemoveFromPath(Dir: string);
var
  Paths, Rest, Part, Kept: string;
  P: Integer;
begin
  if not RegQueryStringValue(HKCU, EnvKey, 'Path', Paths) then
    Exit;
  if not PathHas(Paths, Dir) then
    Exit;
  Kept := '';
  Rest := Paths + ';';
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Part := Copy(Rest, 1, P - 1);
    Rest := Copy(Rest, P + 1, Length(Rest));
    if (Trim(Part) <> '') and
       (Lowercase(RemoveBackslashUnlessRoot(Trim(Part))) <> Lowercase(RemoveBackslashUnlessRoot(Dir))) then
    begin
      if Kept <> '' then
        Kept := Kept + ';';
      Kept := Kept + Part;
    end;
  end;
  RegWriteExpandStringValue(HKCU, EnvKey, 'Path', Kept);
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  { Before [Run], so `filepick setup` and `filepick doctor` see the new PATH. }
  if CurStep = ssPostInstall then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
