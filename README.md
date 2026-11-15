<div align="center">

<img src="assets/app-icon.svg" width="88" height="88" alt="雪耳图标" />

# 雪耳

**按下 Win+H，说话，文字就到光标那里。**

Windows 11 的语音输入。保留系统的 Win+H，识别换成微信输入法的云端识别。

</div>

> 需要联网，不用登录。程序第一次运行时随机生成一个匿名设备身份，只存在本机。
> 不需要装微信或微信输入法。

## 能做什么

| 功能 | 说明 |
| --- | --- |
| Win+H | 用 Windows 自带的语音输入面板和快捷键 |
| 边说边出字 | 说话时文字持续写入；识别结果更新时，会修改还没定稿的那一截 |
| 停顿收句 | 静音 3 秒自动收句（可选 1、2、3、5、10 秒）；接着说就接着听 |
| 选麦克风 | 跟随系统默认，或者指定一个；改完下次按 Win+H 生效 |
| Windows Terminal | 支持边说边改；你手动编辑、按回车或切走后，不再改前面的字 |
| 文字整理 | 热词纠错、去掉句首的“嗯”“呃”、中文数字转阿拉伯数字（开发中，还没接进程序） |
| 历史记录 | 本地保存最近 7 天说过的话，可以改天数或关闭（开发中，还没接进程序） |

## 快速开始

> [!IMPORTANT]
> 程序启动时会核对几个系统组件的指纹。只有已经适配过的 Windows 版本才会接管 Win+H；
> 对不上就不接管，系统原来的语音输入照常能用。

1. 到 [发布页](https://github.com/hardware661/xueer-ear/releases) 下载便携版或安装版。
2. 运行 `xueer-ear.exe`，打开「后台服务」，识别来源选「输入法识别」。
3. 在要打字的地方按 Win+H，开始说话。

| 文件 | 用法 |
| --- | --- |
| `xueer-ear-<版本>-windows-x64-portable.zip` | 解压后直接运行 |
| `xueer-ear-<版本>-windows-x64-setup.exe` | 装到当前用户目录，建开始菜单快捷方式 |
| `SHA256SUMS.txt` | 两个文件的 SHA-256 校验值 |

想换回微软识别，识别来源选「Windows 原生」。托盘「退出」会先把 Win+H 还给系统，再退出程序。
要往管理员权限的窗口里打字，雪耳也要用管理员身份运行。

### 设置和数据放在哪

设置、匿名设备身份、热词和历史记录都在 `%LOCALAPPDATA%\XueerEar`。升级和卸载都不会删这个目录。

发布包里只有程序和桥接 DLL，不含设备身份、凭据、录音或日志。程序不读聊天内容。

## 检查更新

启动时检查一次，之后每 6 小时检查一次。只有便携版和安装版都发布完整，才会提示新版本。
提示里可以选「下载更新」（打开发布页）、「跳过此版本」或「关闭提示」。

## 开发

先读 [AGENTS.md](AGENTS.md) 和 [STATE.md](STATE.md)。

```bash
# 纯 Rust 的部分（文字整理、历史记录）在 WSL 或 Linux 里就能测
cargo test -p xueer-ear-textflow -p xueer-ear-history
```

完整构建需要 Windows：Rust（MSVC 工具链）、Visual Studio C++ 构建工具、Windows SDK 和 CMake；
出安装包还要 Inno Setup 6。

```powershell
cargo test --workspace
./packaging/build-windows.ps1   # PowerShell 7：生成便携版和安装版
```

工具要求和安装包的约定见 [packaging/README.md](packaging/README.md)。

## 许可

[MIT](LICENSE)
