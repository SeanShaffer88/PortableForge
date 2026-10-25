; PortableForge installer — Inno Setup, per-user (no admin needed).
; Installs BOTH programs: portableforge.exe (window + commands) and
; portableforge-agent.exe (the test-box helper) — the helper must
; sit beside the main program, so they always install together.
; User data (settings, run folders) lives under the user's home
; folder, outside this install dir, and survives uninstall.
#ifndef AppVersion
  #define AppVersion "0.1.4"
#endif

[Setup]
AppId={{8F3A2C41-9D5E-4B7A-A1C6-3E8D5F2B9A04}
AppName=PortableForge
AppVersion={#AppVersion}
AppVerName=PortableForge {#AppVersion}
AppPublisher=Sean Shaffer
DefaultDirName={localappdata}\Programs\PortableForge
DefaultGroupName=PortableForge
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputBaseFilename=PortableForge-Setup-v{#AppVersion}-x64
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayName=PortableForge {#AppVersion}

[Files]
Source: "..\target\release\portableforge.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\target\release\portableforge-agent.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\PortableForge"; Filename: "{app}\portableforge.exe"
Name: "{autodesktop}\PortableForge"; Filename: "{app}\portableforge.exe"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Put a shortcut on the desktop"; GroupDescription: "Shortcuts:"

[Run]
Filename: "{app}\portableforge.exe"; Description: "Open PortableForge now"; Flags: nowait postinstall skipifsilent
