; SPDX-License-Identifier: MIT
#ifndef BundleDir
  #error BundleDir must name a verified portable bundle
#endif
#ifndef AppVersion
  #error AppVersion must be supplied by package-installer.ps1
#endif

[Setup]
AppId={{56AC8A2D-5A58-4C7E-B941-C14FB13657D1}
AppName=Strata
AppVersion={#AppVersion}
AppPublisher=Strata
DefaultDirName={localappdata}\Programs\Strata
DefaultGroupName=Strata
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=Strata-{#AppVersion}-windows-x64-setup
Compression=lzma2/fast
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\bin\strata.exe
CloseApplications=yes
RestartApplications=no
LicenseFile=..\..\LICENSE

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "german"; MessagesFile: "compiler:Languages\German.isl"
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "italian"; MessagesFile: "compiler:Languages\Italian.isl"
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Strata"; Filename: "{app}\bin\strata.exe"; WorkingDir: "{app}"; IconFilename: "{app}\bin\strata.exe"
Name: "{autodesktop}\Strata"; Filename: "{app}\bin\strata.exe"; WorkingDir: "{app}"; IconFilename: "{app}\bin\strata.exe"; Tasks: desktopicon

[UninstallRun]
Filename: "{app}\bin\strata.exe"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitStrata"
Filename: "{app}\bin\strata-shell.exe"; Parameters: "--unregister --target ""{app}\bin\strata.exe"""; Flags: runhidden waituntilterminated; RunOnceId: "RestoreFileManager"
