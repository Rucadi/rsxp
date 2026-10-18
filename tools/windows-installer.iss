; SPDX-License-Identifier: GPL-2.0-or-later
[Setup]
AppId=org.rsxp.engine
AppName=RSXP
AppVersion=0.1.0
DefaultDirName={localappdata}\Programs\RSXP
DefaultGroupName=RSXP
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\artifacts
OutputBaseFilename=rsxp-windows-x86_64-setup
Compression=lzma2
SolidCompression=yes
UninstallDisplayIcon={app}\rsxp.exe
LicenseFile=..\COPYING

[Files]
Source: "..\artifacts\windows\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\RSXP"; Filename: "{app}\rsxp.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\RSXP"; Filename: "{app}\rsxp.exe"; WorkingDir: "{app}"

[Run]
Filename: "{app}\rsxp.exe"; Description: "Launch RSXP"; Flags: nowait postinstall skipifsilent
