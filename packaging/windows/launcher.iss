#ifndef AppVersion
  #error AppVersion must be supplied
#endif

[Setup]
AppId={{AA41BA8F-0CA8-4DB5-A343-27FDC68E62F7}
AppName=OpenSC Launcher
AppVersion={#AppVersion}
AppPublisher=OpenSC
AppPublisherURL=https://shadow-test.zaikinagroup.ru/
AppSupportURL=https://shadow-test.zaikinagroup.ru/
DefaultDirName={localappdata}\Programs\OpenSC Launcher
DefaultGroupName=OpenSC
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#OutputPath}
OutputBaseFilename=opensc-launcher-{#AppVersion}-windows-x86_64-setup
SetupIconFile={#IconPath}
UninstallDisplayIcon={app}\opensc-launcher.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinaryPath}"; DestDir: "{app}"; DestName: "opensc-launcher.exe"; Flags: ignoreversion

Source: "{#NoticesPath}"; DestDir: "{app}"; DestName: "THIRD-PARTY-NOTICES.txt"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\OpenSC Launcher"; Filename: "{app}\opensc-launcher.exe"
Name: "{userdesktop}\OpenSC Launcher"; Filename: "{app}\opensc-launcher.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\opensc-launcher.exe"; Description: "{cm:LaunchProgram,OpenSC Launcher}"; Flags: nowait postinstall skipifsilent
