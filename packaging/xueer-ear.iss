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
Source: "{#StageDir}\RNNOISE-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
; Only the new GUI implements the current cooperative-shutdown contract.
Source: "{#StageDir}\xueer-ear.exe"; DestName: "xueer-ear-shutdown.exe"; Flags: dontcopy

[Icons]
Name: "{autoprograms}\雪耳"; Filename: "{app}\xueer-ear.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\雪耳"; Filename: "{app}\xueer-ear.exe"; WorkingDir: "{app}"; Tasks: desktopicon

; 开机自启走 HKCU Run（和桌宠、音箱一样）。启动文件夹的快捷方式 2026-11-17 开机没被执行。
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "XueerEar"; ValueData: """{app}\xueer-ear.exe"" --autostart"; Flags: uninsdeletevalue; Tasks: autostart

; 1.0.1 及更早放在启动文件夹的快捷方式，升级时删掉，免得开机启动两次
[InstallDelete]
Type: files; Name: "{userstartup}\雪耳.lnk"

[Run]
Filename: "{app}\xueer-ear.exe"; Description: "打开雪耳"; Flags: nowait postinstall skipifsilent shellexec runasoriginaluser
; 控制面板「下载更新」带 /SELFUPDATE=1 静默安装：装完自己重新打开，开好后台服务并说一声已更新。
; 别人自己用 /SILENT 部署时不带这个参数，不会突然弹出窗口
Filename: "{app}\xueer-ear.exe"; Parameters: "--updated"; Flags: nowait runasoriginaluser; Check: IsSelfUpdate

[Code]
var
  ShutdownComplete: Boolean;

function IsSelfUpdate: Boolean;
begin
  Result := WizardSilent and (ExpandConstant('{param:SELFUPDATE|0}') = '1');
end;

function CooperativeShutdown(const Application: String; const WorkingDirectory: String): Boolean;
var
  ExitCode: Integer;
begin
  Result := ShellExec('open', Application, '--quit', WorkingDirectory, SW_HIDE, ewWaitUntilTerminated, ExitCode);
  if Result then
    Result := ExitCode = 0;
  // 1.2.0 以前的雪耳（以及打开了「以管理员身份运行」的）是管理员身份，
  // 普通权限的退出请求发不过去：再以管理员身份请求一次（会弹 UAC）
  if not Result then
  begin
    Result := ShellExec('runas', Application, '--quit', WorkingDirectory, SW_HIDE, ewWaitUntilTerminated, ExitCode);
    if Result then
      Result := ExitCode = 0;
  end;
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
// 还被系统进程加载着的删不掉，跳过：下次升级或重启后再删。不用 RestartReplace：
// 它要写机器级注册表，这个安装器按当前用户装、没有管理员权限，调用会直接报错中断（1.0.1 实测）。
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
        if DeleteFile(Path) then
          Log('Removed old bridge: ' + Found.Name)
        else
          Log('Old bridge still in use, left for later: ' + Found.Name);
      end;
    until not FindNext(Found);
  finally
    FindClose(Found);
  end;
end;

const
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    RemoveOldBridges('xueer_ear_bridge-{#AppVersion}.dll');
    // 重装时取消勾选开机自启：删掉以前写的 Run 值
    if not WizardIsTaskSelected('autostart') then
      RegDeleteValue(HKCU, RunKey, 'XueerEar');
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RemoveOldBridges('');
end;

// No [UninstallDelete] for LOCALAPPDATA\XueerEar: user identity/settings/logs stay intact.
