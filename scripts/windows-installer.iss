#define AppVersion GetEnv("NVPN_RELEASE_VERSION")
#define SourceDir GetEnv("NVPN_WINDOWS_PUBLISH_DIR")
#define OutputDir GetEnv("NVPN_WINDOWS_INSTALLER_OUTPUT_DIR")
#define OutputBaseName GetEnv("NVPN_WINDOWS_INSTALLER_BASENAME")
#define ProjectRoot GetEnv("NVPN_PROJECT_ROOT")

[Setup]
AppId={{DA4FA554-4718-4E6D-8CE8-E43A05B4B723}
AppName=Nostr VPN
AppVersion={#AppVersion}
AppPublisher=Nostr VPN
DefaultDirName={localappdata}\Programs\Nostr VPN
DefaultGroupName=Nostr VPN
DisableProgramGroupPage=yes
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBaseName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
PrivilegesRequired=lowest
SetupIconFile={#ProjectRoot}\windows\NostrVpn.Windows\Assets\nostr-vpn.ico
UninstallDisplayIcon={app}\NostrVpn.Windows.exe

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Nostr VPN"; Filename: "{app}\NostrVpn.Windows.exe"; IconFilename: "{app}\Assets\nostr-vpn.ico"
Name: "{autodesktop}\Nostr VPN"; Filename: "{app}\NostrVpn.Windows.exe"; IconFilename: "{app}\Assets\nostr-vpn.ico"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
Filename: "{app}\NostrVpn.Windows.exe"; Description: "Launch Nostr VPN"; Flags: nowait postinstall skipifsilent
