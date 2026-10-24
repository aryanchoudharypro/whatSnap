; Inno Setup script for whatSnap.
; Build with: iscc packaging\whatSnap.iss  (or run packaging\build-release.ps1)

#define AppName "whatSnap"
#define AppExe "whatSnap.exe"
#define AppPublisher "Aryan Choudhary"
#define AppURL "https://github.com/aryanchoudharypro/whatSnap"
#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\target\release"
#endif

[Setup]
AppId={{63FD6445-3903-4812-AF2B-1A9B0672C679}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases

; Per-user install: no UAC prompt, works on machines where the user is not an
; administrator. {autopf} resolves to {localappdata}\Programs here.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes

ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible

OutputDir=..\dist
OutputBaseFilename={#AppName}-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern

LicenseFile=..\LICENSE
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\{#AppExe}

; Offer to close a running instance rather than failing or silently leaving a
; stale exe in place, and start it again afterwards (whatSnap registers itself
; for that with RegisterApplicationRestart). "force" ends whatever hasn't
; closed by the Restart Manager's timeout, so a stuck backend can't block the
; update.
CloseApplications=force
RestartApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\backend\backend.exe"; DestDir: "{app}\backend"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\backend\LICENSE"; DestDir: "{app}\backend"; Flags: ignoreversion
Source: "..\THIRD-PARTY-NOTICES.md"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
; In the Programs folder itself (not a subfolder): notifications are tied to this
; shortcut, and the app looks for it at {userprograms}\whatSnap.lnk.
Name: "{userprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
; runasoriginaluser: start the app as the user who ran the installer, even if the
; installer was elevated. An elevated instance can't be reached by the normal
; Start menu and desktop launches, which then silently do nothing.
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[Code]
// Start-with-Windows is a per-user Run entry named whaNTV. Older versions point
// it at whantv.exe, which this install no longer provides, so re-point an
// existing entry at the new exe.
procedure CurStepChanged(CurStep: TSetupStep);
var
  RunPath: String;
  RunValue: String;
begin
  RunPath := 'Software\Microsoft\Windows\CurrentVersion\Run';
  if (CurStep = ssPostInstall) and RegQueryStringValue(HKCU, RunPath, 'whaNTV', RunValue) then
    RegWriteStringValue(HKCU, RunPath, 'whaNTV', '"' + ExpandConstant('{app}\{#AppExe}') + '"');
end;

// User data lives in {localappdata}\whatSnap, outside {app}, so it survives
// uninstall unless the user asks otherwise. Removing it means re-pairing the
// phone and losing local history, so it is never the default.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{localappdata}\whatSnap');
    if DirExists(DataDir) then
    begin
      if SuppressibleMsgBox(
           'Also delete your whatSnap data?' #13#10 #13#10
           + 'This removes the WhatsApp session, local message history and '
           + 'downloaded media in:' #13#10 + DataDir + #13#10 #13#10
           + 'You would need to link your phone again. Choose No to keep it.',
           mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
        DelTree(DataDir, True, True, True);
    end;
  end;
end;
