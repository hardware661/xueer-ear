#ifndef AppVersion
  #error AppVersion must be supplied from Cargo metadata
#endif
#ifndef StageDir
  #error StageDir must contain the verified release allowlist
#endif

[Setup]
AppId={{660FE38B-9856-4B3B-A919-7D9DBD5A80F3}
AppName=雪耳
AppVersion={#AppVersion}
AppPublisher=hardware661
AppPublisherURL=https://github.com/hardware661/xueer-ear
AppSupportURL=https://github.com/hardware661/xueer-ear/issues
AppUpdatesURL=https://github.com/hardware661/xueer-ear/releases/latest
DefaultDirName={localappdata}\Programs\xueer-ear
DefaultGroupName=雪耳
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.22000
OutputBaseFilename=xueer-ear-{#AppVersion}-windows-x64-setup
SetupIconFile=..\assets\app-icon.ico
UninstallDisplayIcon={app}\xueer-ear.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
AlwaysRestart=no
Uninstallable=yes
UsePreviousAppDir=yes

[Languages]
#if FileExists(AddBackslash(CompilerPath) + "Languages\ChineseSimplified.isl")
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
#endif
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "创建桌面快捷方式"; GroupDescription: "快捷方式"; Flags: unchecked
Name: "autostart"; Description: "登录 Windows 时自动启动并开启后台服务"; GroupDescription: "启动"

[Files]
Source: "{#StageDir}\xueer-ear.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\xueer-ear-host.exe"; DestDir: "{app}"; Flags: ignoreversion
; Versioned bridge can remain mapped by Windows after cooperative Disable.
; Never overwrite it during same-version reinstall or force-stop its host.
Source: "{#StageDir}\xueer_ear_bridge-{#AppVersion}.dll"; DestDir: "{app}"; Flags: onlyifdoesntexist uninsrestartdelete
Source: "{#StageDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\Start.cmd"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\Stop.cmd"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#StageDir}\OCTICONS-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
; Only the new GUI implements the current cooperative-shutdown contract.
Source: "{#StageDir}\xueer-ear.exe"; DestName: "xueer-ear-shutdown.exe"; Flags: dontcopy

[Icons]
Name: "{autoprograms}\雪耳"; Filename: "{app}\xueer-ear.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\雪耳"; Filename: "{app}\xueer-ear.exe"; WorkingDir: "{app}"; Tasks: desktopicon
Name: "{userstartup}\雪耳"; Filename: "{app}\xueer-ear.exe"; Parameters: "--autostart"; WorkingDir: "{app}"; Tasks: autostart

[Run]
Filename: "{app}\xueer-ear.exe"; Description: "打开雪耳"; Flags: nowait postinstall skipifsilent shellexec runasoriginaluser

[Code]
var
  ShutdownComplete: Boolean;

function CooperativeShutdown(const Application: String; const WorkingDirectory: String): Boolean;
var
  ExitCode: Integer;
begin
  Result := ShellExec('open', Application, '--quit', WorkingDirectory, SW_HIDE, ewWaitUntilTerminated, ExitCode);
  if Result then
    Result := ExitCode = 0;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if ShutdownComplete then
    Exit;
  ExtractTemporaryFile('xueer-ear-shutdown.exe');
  ExtractTemporaryFile('xueer-ear-host.exe');
  if not CooperativeShutdown(ExpandConstant('{tmp}\xueer-ear-shutdown.exe'), ExpandConstant('{tmp}')) then
  begin
    Result := '雪耳 尚未完成退出与恢复，请先退出雪耳后重试。';
    Exit;
  end;
  ShutdownComplete := True;
end;

function InitializeUninstall: Boolean;
begin
  if not FileExists(ExpandConstant('{app}\xueer-ear.exe')) then
  begin
    MsgBox('找不到雪耳退出程序。请先恢复安装，再执行卸载。', mbError, MB_OK);
    Result := False;
    Exit;
  end;
  Result := CooperativeShutdown(ExpandConstant('{app}\xueer-ear.exe'), ExpandConstant('{app}'));
  if not Result then
    MsgBox('雪耳 尚未完成退出与恢复，卸载已中止。', mbError, MB_OK);
end;

// 桥接 DLL 文件名带版本号，升级后旧版本的会留在安装目录（审查 R13）。
// Keep = 要留下的文件名（升级时是这一版的，卸载时为空 = 全删）。
// 删不掉（还被系统加载着）的标记成重启后删除，不影响安装和卸载。
procedure RemoveOldBridges(const Keep: String);
var
  Found: TFindRec;
  Path: String;
begin
  if FindFirst(ExpandConstant('{app}\xueer_ear_bridge-*.dll'), Found) then
  try
    repeat
      if CompareText(Found.Name, Keep) <> 0 then
      begin
        Path := ExpandConstant('{app}\') + Found.Name;
        if not DeleteFile(Path) then
          RestartReplace(Path, '');
      end;
    until not FindNext(Found);
  finally
    FindClose(Found);
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    RemoveOldBridges('xueer_ear_bridge-{#AppVersion}.dll');
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveOldBridges('');
end;

// No [UninstallDelete] for LOCALAPPDATA\XueerEar: user identity/settings/logs stay intact.
