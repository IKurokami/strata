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
SetupIconFile=..\..\data\brand\strata.ico
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
Name: "{group}\Strata"; Filename: "{app}\bin\strata.exe"; WorkingDir: "{app}"; IconFilename: "{app}\bin\strata.exe"; AppUserModelID: "io.github.lgse.Strata"
Name: "{autodesktop}\Strata"; Filename: "{app}\bin\strata.exe"; WorkingDir: "{app}"; IconFilename: "{app}\bin\strata.exe"; AppUserModelID: "io.github.lgse.Strata"; Tasks: desktopicon

[UninstallRun]
Filename: "{app}\bin\strata.exe"; Parameters: "--quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitStrata"
Filename: "{app}\bin\strata-shell.exe"; Parameters: "--unregister --target ""{app}\bin\strata.exe"""; Flags: runhidden waituntilterminated skipifdoesntexist; RunOnceId: "RestoreFileManager"

[CustomMessages]
ShellRestoreFailed_en=Unable to restore file manager settings. Strata has not been uninstalled.
ShellRestoreFailed_vi=Không thể khôi phục thiết lập trình quản lý file. Strata chưa được gỡ cài đặt.
ShellRestoreFailed_fr=Impossible de restaurer les paramètres du gestionnaire de fichiers. Strata n’a pas été désinstallé.
ShellRestoreFailed_de=Die Dateimanager-Einstellungen konnten nicht wiederhergestellt werden. Strata wurde nicht deinstalliert.
ShellRestoreFailed_es=No se pudo restaurar la configuración del gestor de archivos. Strata no se ha desinstalado.
ShellRestoreFailed_ja=ファイルマネージャーの設定を復元できません。Strata はアンインストールされていません。
ShellRestoreFailed_pt_BR=Não foi possível restaurar as configurações do gerenciador de arquivos. O Strata não foi desinstalado.
ShellRestoreFailed_ko=파일 관리자 설정을 복원할 수 없습니다. Strata가 제거되지 않았습니다.
ShellRestoreFailed_it=Impossibile ripristinare le impostazioni del gestore file. Strata non è stato disinstallato.
ShellRestoreFailed_ru=Не удалось восстановить настройки файлового менеджера. Strata не была удалена.
ShellRestoreFailed_zh_CN=无法恢复文件管理器设置。Strata 尚未卸载。

[Code]
function RestoreFailureMessage: String;
var
  Locale: String;
begin
  Locale := 'en';
  if ActiveLanguage = 'french' then Locale := 'fr';
  if ActiveLanguage = 'german' then Locale := 'de';
  if ActiveLanguage = 'spanish' then Locale := 'es';
  if ActiveLanguage = 'japanese' then Locale := 'ja';
  if ActiveLanguage = 'brazilianportuguese' then Locale := 'pt_BR';
  if ActiveLanguage = 'italian' then Locale := 'it';
  if ActiveLanguage = 'russian' then Locale := 'ru';
  Result := CustomMessage('ShellRestoreFailed_' + Locale);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Helper: String;
  ExitCode: Integer;
begin
  if CurUninstallStep <> usUninstall then Exit;
  Helper := ExpandConstant('{app}\bin\strata-shell.exe');
  if not FileExists(Helper) then
    Helper := ExpandConstant('{localappdata}\Strata\ShellIntegration\strata-shell-v2.exe');
  if not FileExists(Helper) then begin
    if RegValueExists(HKCU, 'Software\Strata\FileManagerV2', 'JournalV2') or
       RegValueExists(HKCU, 'Software\Strata\FileManagerV2', 'Executable') or
       RegValueExists(HKCU, 'Software\Strata\FileManager', 'Executable') then
      RaiseException(RestoreFailureMessage);
    Exit;
  end;
  if not Exec(Helper, '--unregister --target "' + ExpandConstant('{app}\bin\strata.exe') + '"', '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    RaiseException(RestoreFailureMessage);
  if ExitCode <> 0 then RaiseException(RestoreFailureMessage);
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ExitCode: Integer;
begin
  if CurStep <> ssPostInstall then Exit;
  if Exec(ExpandConstant('{app}\bin\strata-shell.exe'),
      '--upgrade-integration --target "' + ExpandConstant('{app}\bin\strata.exe') + '"',
      '', SW_HIDE, ewWaitUntilTerminated, ExitCode) then
    Log(Format('Shell integration upgrade exit code: %d', [ExitCode]))
  else
    Log('Shell integration upgrade helper could not start');
end;
