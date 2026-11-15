# 雪耳 xueer-ear · AGENTS

主人自己的 Windows Win+H 语音输入（识别走微信输入法云端），基于朋友的微win（MIT）二开，跑在主人的 Windows 11 上。

## 先读

1. `STATE.md` 最新一节（现状、本轮范围、未验项）
2. 主人桌面的计划 `雪耳xueer-ear计划-v1.md`
3. `README.md`

## 常用命令

```bash
cargo test -p xueer-ear-textflow -p xueer-ear-history   # 纯 Rust 部分，WSL 里直接跑
tools/win.sh cargo test --workspace                     # 全量，走 Windows 工具链（同步到 D:\projects\_build\xueer-ear）
tools/win.sh pwsh-script packaging/build-windows.ps1    # 打包
```

核对外观不要在屏幕上弹窗，用离屏渲染出 PNG 再看：`xueer-ear-host.exe --panel-render <目录>`（听写面板四种样子）、`xueer-ear.exe --snapshot <目录>`（控制面板和添加热词对话框）。

发布与回滚：无线上部署；GitHub 公开仓推送、打标签、发 Release 每次先问主人。

## 分层

- `crates/textflow/`：识别文字的后处理，纯函数，不依赖 Windows 和其他 crate
- `crates/history/`：本地历史记录 jsonl，只依赖 serde
- `crates/protocol`、`backend`、`audio`：协议、识别引擎、音频
- `crates/winh-bridge`、`winh-host`：Win+H 接管（Windows 专用）
- `crates/gui`、`voice-cli`：界面与命令行

## 禁止

- 不把凭据、主人的个人热词写进仓库（热词在 `%LOCALAPPDATA%\XueerEar\hotwords.txt`）
- 不推送、不发布，除非主人当次同意
- LICENSE 里朋友那行版权不能删
- 读接管、指纹、协议相关代码时用 grep 定位、只读小片段，每步写进 STATE.md（2026-11-15 被安全分类器掐断过，主人同日确认这些都是正常需求，可以做）
