# 参与雪耳

欢迎提问题、提建议、发 PR。English is welcome too.

## 报告问题

1. 先在 [issues](https://github.com/hardware661/xueer-ear/issues) 里搜一下，看有没有人提过。
2. 用「问题反馈」模板新建 issue，写清楚：
   - 在哪个程序里出的问题，比如 Windows Terminal 里的 Claude Code、微信、记事本；
   - 怎么操作的，看到了什么，你希望是什么样。
3. 在控制面板「更多设置」里点「复制诊断信息」，贴进 issue。诊断信息里有版本、系统版本、几项设置和日志最后 50 行；日志只记字数和错误原因，不记你说过的话。

不要贴设备身份文件（`%LOCALAPPDATA%\XueerEar` 里的 `wetype-device.json` 一类）、翻译服务的 Key 和录音。

## 提建议

用「功能建议」模板，或者到 [Discussions](https://github.com/hardware661/xueer-ear/discussions) 里聊。说清楚在什么场景下需要，现在是怎么凑合的。

## 改代码

先读 [AGENTS.md](AGENTS.md) 了解分层：

| 目录 | 放什么 |
| --- | --- |
| `crates/textflow` | 识别文字的后处理和语音指令，纯函数，不依赖 Windows |
| `crates/history` | 本地历史记录 |
| `crates/protocol`、`crates/backend`、`crates/audio` | 协议、识别引擎、翻译、音频 |
| `crates/winh-host`、`crates/winh-bridge` | 后台服务：快捷键听写、上屏、听写面板、接管 Win+H |
| `crates/gui`、`crates/voice-cli` | 控制面板、翻译小窗、命令行工具 |

动手前：

- 修 bug 先写一个能复现的测试，再改代码。
- 不依赖 Windows 的部分在 WSL 或 Linux 里就能测：`cargo test -p xueer-ear-textflow -p xueer-ear-history -p xueer-ear-backend -p xueer-ear-host -p xueer-ear`。
- 完整测试要在 Windows 上跑：`cargo test --workspace`。
- 提交前跑 `cargo fmt --all`，CI 会检查格式。
- 改了界面，用离屏渲染核对，不用真的弹窗：`xueer-ear.exe --snapshot <目录>`（浅色一套，深色在 `dark` 子目录）、`xueer-ear-host.exe --panel-render <目录>`。PR 里附上浅色和深色的截图。
- 新增依赖、改安装包或接管 Win+H 的部分，先开 issue 说一下。

提交信息写清楚改了什么、为什么。一个 PR 只做一件事。

## 发版

维护者负责。改版本号，在 [CHANGELOG.md](CHANGELOG.md) 加上 `## v<版本>` 一节，打 `v<版本>` 标签推上去。CI 构建、测试、打包，再用 CHANGELOG 里那一节当发布说明。

## 许可

提交的代码按 [MIT](LICENSE) 许可发布。
