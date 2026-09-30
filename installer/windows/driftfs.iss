; DriftFS Windows Installer Script (Inno Setup 6+)
; Defines installation layout, WinFsp dependency detection, shortcuts, and uninstallation.

#define MyAppName "DriftFS"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "DriftFS Contributors"
#define MyAppURL "https://github.com/shditz/DriftFS"
#define MyAppExeName "driftfs-ui.exe"

[Setup]
AppId={{9F82D374-B3E1-4D56-8A33-9125C0736E6E}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}
AppUpdatesURL={#MyAppURL}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
AllowNoIcons=yes
LicenseFile=..\..\LICENSE-MIT
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
OutputDir=..\..\dist
OutputBaseFilename=driftfs-setup-v{#MyAppVersion}-windows-x86_64
SetupIconFile=..\..\assets\icons\driftfs.ico
UninstallDisplayIcon={app}\assets\driftfs.ico
Compression=lzma2/ultra64
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "autostart"; Description: "Start DriftFS automatically when Windows starts"; GroupDescription: "Startup options:"; Flags: unchecked

[Files]
Source: "..\..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\icons\driftfs.ico"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "..\..\assets\icons\driftfs.png"; DestDir: "{app}\assets"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\config.example.toml"; DestDir: "{app}"; DestName: "config.example.toml"; Flags: ignoreversion
Source: "..\..\target\release\winfsp-x64.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\driftfs.ico"
Name: "{group}\Uninstall {#MyAppName}"; Filename: "{uninstallexe}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; IconFilename: "{app}\assets\driftfs.ico"; Tasks: desktopicon
Name: "{userstartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: autostart

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#StringChange(MyAppName, '&', '&&')}}"; Flags: nowait postinstall skipifsilent

[Code]
// Detect WinFsp runtime presence via standard registry keys
function IsWinFspInstalled: Boolean;
begin
  Result := RegKeyExists(HKLM, 'Software\WinFsp') or
            RegKeyExists(HKLM, 'SOFTWARE\WOW6432Node\WinFsp') or
            RegKeyExists(HKLM64, 'Software\WinFsp');
end;

function InitializeSetup(): Boolean;
var
  ErrorCode: Integer;
begin
  Result := True;
  if not IsWinFspInstalled then
  begin
    if MsgBox('DriftFS requires the WinFsp runtime to mount Google Drive as a local Windows drive.' + #13#10 + #13#10 +
              'WinFsp was not detected on this system.' + #13#10 + #13#10 +
              'Would you like to open the WinFsp download page in your web browser now?',
              mbConfirmation, MB_YESNO) = idYes then
    begin
      ShellExec('open', 'https://winfsp.dev/rel/', '', '', SW_SHOWNORMAL, ewNoWait, ErrorCode);
    end;
  end;
end;
