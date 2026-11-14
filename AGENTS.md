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
- Claude 会话：接管系统组件、指纹、私有协议相关代码容易被安全分类器掐断，按 STATE.md 最新一节的范围做，范围外的先问主人
