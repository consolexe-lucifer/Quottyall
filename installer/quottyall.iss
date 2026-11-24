; Inno Setup script for the **Quottyall** fork. Deliberately a separate file from
; installer\quotty.iss, with its own AppId, name and install directory, so this
; build can be installed beside the original Quotty instead of replacing it.
; Built by tools\build-installer-fork.ps1, which passes the version from
; Cargo.toml:
;   ISCC.exe /DAppVersion=1.5.1 installer\quottyall.iss
#ifndef AppVersion
  #define AppVersion "1.0.0"
#endif
#define AppName "Quottyall"
#define AppPublisher "consolexe-lucifer (fork of Quotty by Brent)"
#define AppURL "https://github.com/consolexe-lucifer/Quottyall"
#define AppExe "quottyall.exe"
; Same executable name as the original, but a different AppId keeps the two
; installations and their uninstall entries apart.
#define ForkAppId "{{A3D7C1E2-5B64-4F09-8C77-2D1E9A6B4F30}"

[Setup]
AppId={#ForkAppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppPublisherURL={#AppURL}
AppSupportURL={#AppURL}/issues
AppUpdatesURL={#AppURL}/releases
VersionInfoVersion={#AppVersion}
VersionInfoDescription=Quottyall - quota strip (fork of Quotty)
VersionInfoCopyright=Quotty by Brent (GPL-3.0); fork additions by consolexe-lucifer
; Per-user install: no admin prompt, and autostart/settings live per user anyway.
PrivilegesRequired=lowest
; A folder of its own: {autopf}\Quottyall, never {autopf}\Quotty.
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName} {#AppVersion}
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename={#AppName}-Setup-{#AppVersion}
SetupIconFile=..\assets\quotty.ico
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "en"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "autostart"; Description: "{cm:AutoStartTask}"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\docs\minimax-api.md"; DestDir: "{app}\docs"; Flags: ignoreversion
Source: "..\docs\deepseek-api.md"; DestDir: "{app}\docs"; Flags: ignoreversion

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{userstartup}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: autostart

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; The strip has no taskbar window, so let the uninstaller close it itself.
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im {#AppExe}"; Flags: runhidden; RunOnceId: "StopQuottyall"

[UninstallDelete]
Type: files; Name: "{userstartup}\{#AppName}.lnk"
Type: files; Name: "{userdesktop}\{#AppName}.lnk"

[CustomMessages]
ru.AutoStartTask=Запускать Quottyall при входе в Windows
en.AutoStartTask=Start Quottyall when Windows starts

[Code]
// A running copy would keep quotty.exe locked during an upgrade.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/f /im {#AppExe}', '',
       SW_HIDE, ewWaitUntilTerminated, ResultCode);
  Result := '';
end;
