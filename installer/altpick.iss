; Inno Setup script for altpick's Setup.exe.
; Build with installer\build.ps1 (it passes AppVersion from Cargo.toml).
;
; Per-user install, no admin rights:
;   - files go to %LOCALAPPDATA%\Programs\altpick
;   - that folder is added to the user PATH (and removed on uninstall)
;   - `altpick setup` turns the hotkey on; uninstall runs `altpick remove`
; Files written by an installer carry no "downloaded from the internet"
; mark, so PowerShell loads altpick-hook.ps1 without Unblock-File.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir ".."
#endif

[Setup]
AppId={{67803C43-2715-4BE0-AC1E-D76786EEA825}
AppName=AltPick
AppVersion={#AppVersion}
AppPublisher=AltPick
DefaultDirName={localappdata}\Programs\altpick
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes
PrivilegesRequired=lowest
ChangesEnvironment=yes
OutputDir={#SourceDir}\target\installer
OutputBaseFilename=altpick-{#AppVersion}-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayName=AltPick
UninstallDisplayIcon={app}\altpick.exe
CloseApplications=no

[Tasks]
Name: allowscripts; Description: "Allow PowerShell to load the hotkey (sets your own PowerShell execution policy to RemoteSigned, the usual setting, only if it currently blocks scripts)"

[Files]
Source: "{#SourceDir}\target\release\altpick.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\shell\altpick-hook.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\shell\altpick.lua"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\shell\altpick.bash"; DestDir: "{app}"; Flags: ignoreversion

[Run]
Filename: "{app}\altpick.exe"; Parameters: "setup --allow-scripts"; Tasks: allowscripts; Flags: runhidden; StatusMsg: "Turning on the hotkey..."
Filename: "{app}\altpick.exe"; Parameters: "setup"; Tasks: not allowscripts; Flags: runhidden; StatusMsg: "Turning on the hotkey..."

[UninstallRun]
Filename: "{app}\altpick.exe"; Parameters: "remove"; Flags: runhidden; RunOnceId: "altpick-remove"

[Messages]
FinishedLabel=AltPick is installed.%n%nOpen a new terminal, start typing a command and press Alt+F.%n%nIf something doesn't work, run: altpick doctor

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

{ AltPick used to be called filepick (its own AppId). Uninstall it silently
  first, so the old hotkey lines, PATH entry and files don't stay behind. }
const
  OldAppKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\{6E3C9F2A-4B1D-4C7E-9A55-3F2D8B7C1E40}_is1';

procedure RemoveOldFilepick;
var
  Uninstaller, OldExe: string;
  Code, Waited: Integer;
begin
  if not RegQueryStringValue(HKCU, OldAppKey, 'UninstallString', Uninstaller) then
    Exit;
  Uninstaller := RemoveQuotes(Uninstaller);
  if not FileExists(Uninstaller) then
    Exit;
  OldExe := AddBackslash(ExtractFileDir(Uninstaller)) + 'filepick.exe';
  Exec(Uninstaller, '/VERYSILENT /SUPPRESSMSGBOXES /NORESTART', '', SW_HIDE, ewWaitUntilTerminated, Code);
  { The uninstaller finishes in a copy of itself; wait for it (at most 30 s). }
  Waited := 0;
  while (FileExists(OldExe) or RegKeyExists(HKCU, OldAppKey)) and (Waited < 30000) do
  begin
    Sleep(500);
    Waited := Waited + 500;
  end;
  RemoveDir(ExtractFileDir(Uninstaller));  { only if it's empty now }
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssInstall then
    RemoveOldFilepick;
  { Before [Run], so `altpick setup` and `altpick doctor` see the new PATH. }
  if CurStep = ssPostInstall then
    AddToPath(ExpandConstant('{app}'));
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveFromPath(ExpandConstant('{app}'));
end;
