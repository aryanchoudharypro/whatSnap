; Inno Setup script for whaNTV.
; Build with: iscc packaging\whantv.iss  (or run packaging\build-release.ps1)

#define AppName "whaNTV"
#define AppExe "whantv.exe"
#define AppPublisher "Aryan Choudhary"
#define AppURL "https://github.com/aryanchoudharypro/whaNTV"
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
; stale exe in place.
CloseApplications=yes
RestartApplications=no

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
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{group}\{cm:UninstallProgram,{#AppName}}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
// User data lives in {localappdata}\whaNTV, outside {app}, so it survives
// uninstall unless the user asks otherwise. Removing it means re-pairing the
// phone and losing local history, so it is never the default.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  DataDir: String;
begin
  if CurUninstallStep = usPostUninstall then
  begin
    DataDir := ExpandConstant('{localappdata}\whaNTV');
    if DirExists(DataDir) then
    begin
      if SuppressibleMsgBox(
           'Also delete your whaNTV data?' #13#10 #13#10
           + 'This removes the WhatsApp session, local message history and '
           + 'downloaded media in:' #13#10 + DataDir + #13#10 #13#10
           + 'You would need to link your phone again. Choose No to keep it.',
           mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES then
        DelTree(DataDir, True, True, True);
    end;
  end;
end;
