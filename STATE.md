# STATE · 雪耳 xueer-ear

> 任何 AI 接手先读这里；每次会话收尾在最上面追加一节（最新在上），写清做了什么、验证了什么、还差什么。
> 计划原文：主人桌面 `雪耳xueer-ear计划-v1.md`（v1.1）。

## 2026-11-15 · 只做「安全部分」（Claude；主人原话："那你先做安全的部分 再被掐就不让你做了 自主推进吧"）

**背景**：上一会话在读 Win+H 接管 / 输入法协议相关代码时被安全分类器掐断 3 次。主人定：这一轮只做不碰系统接管和私有协议的部分；再被掐，这个项目就不交给 Claude 做了。

**本轮范围（安全部分）**：
- 项目文件：AGENTS.md、STATE.md、`~/projects/README.md` 登记、共用记忆 `projects/xueer-ear.md`
- 新 crate `crates/textflow`：纯文字处理（A1 热词、A2 口癖、A3 标点、A4 语音指令识别、A5 数字、D13 终端整段拼接），不依赖 Windows，WSL 里测
- 新 crate `crates/history`：C10 本地历史记录（jsonl、按天数清理），纯文件读写

**本轮明确不碰（留给主人决定交给谁）**：
- `winh-host`、`winh-bridge`、`backend` 里的微信云端引擎、系统文件指纹、`protocol`
- 把 textflow / history 接进 Win+H 和终端输出路径（要改 winh-host）
- C9 指纹兜底、C11 独立快捷键（键盘钩子 + 模拟键入）、C12 更新检查、B6 界面、B8 提示音、打包安装、nav 登记上线、推 GitHub

### 进度（每步做完更新）

- [x] 1 AGENTS.md / STATE.md
- [ ] 2 textflow crate + 测试
- [ ] 3 history crate + 测试
- [ ] 4 登记 README / 记忆

**已有提交（上一会话）**：`095d4e7` 改名 + 接受 26200.9974 指纹；`deab5ea` recognize-wave-wetype 命令。都只在本地，没推送、没在真机验证。
