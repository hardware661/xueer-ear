# STATE · 雪耳 xueer-ear

> 任何 AI 接手先读这里；每次会话收尾在最上面追加一节（最新在上），写清做了什么、验证了什么、还差什么。
> 计划原文：主人桌面 `雪耳xueer-ear计划-v1.md`（v1.1）。

## 2026-11-15 第三轮 · 剩下的全做（Claude；主人原话："我觉得没有什么不安全的 很正常的需求 你可以继续做"）

- [x] T1 Inno Setup 6.7.4 → `D:\devtool\InnoSetup6`（签名有效，当前用户安装）；PowerShell 7.7.0 便携 → `D:\devtool\pwsh`（SHA-256 与 GitHub 公布一致）；都经 ghfast.top 镜像下载
- [x] T2 `tools/win.sh pwsh-script packaging/build-windows.ps1` 出包成功：portable.zip 1.6 MB、setup.exe 3.4 MB、SHA256SUMS（在 `D:\projects\_build\xueer-ear\dist\release`）。修了三处：crate description 改 ASCII（Windows PowerShell 解析中文 JSON 出错）、win.sh 给打包补 PATH/ISCC/RunAsInvoker、build 脚本认 `XUEER_EAR_ISCC`
- [ ] T3 C9 指纹对不上：不接管、提示一次、自动切快捷键模式
- [ ] T4 C11 快捷键模式（按住右 Alt 说话、松开上屏）
- [ ] T5 A4 语音指令执行（换行/删上一句/撤销/发送）
- [ ] T6 设置界面：文字整理开关、历史天数、历史查看
- [ ] T7 B8 晓晓提示音
- [ ] T8 装到主人电脑 + 自启动；请主人真机验收
- [ ] T9 nav 登记、公开仓推送和首个 Release（每步先问）

## 2026-11-15 第二轮 · 继续做，做到被掐为止（Claude；主人原话："继续做ear吧 做到cyber为止 你小心一点吧"）

**做法**：按风险从低到高排，每步做完就提交并在这里打勾；被掐了，下一个接手的人从第一个没勾的继续。读代码只 grep 定位、读小片段。

- [x] R1 README 重写（中文、不提上游）、assets 说明
- [x] R2 B6 图标：手画 SVG 兽耳（深灰底、白耳、笑脸弧线），PNG 1024 和 ICO 七帧从 SVG 重新生成，16 像素下看过能认出
- [x] R3 C12 更新检查：核对过，已经查 `hardware661/xueer-ear` 的 Releases（`crates/gui/src/update.rs`），每 6 小时一次，不用改；「上游有新版」提示没做（要加第二个查询地址，放后面）
- [x] R4 设置：`backend/src/settings.rs` 的 settings.json 加 `text`（TextSettings 六个开关）、`history_enabled`、`history_keep_days`（1～365）；旧文件缺字段自动用默认值；GUI 保存时克隆整份不会丢字段。还没有界面开关，要手改 json
- [x] R5 新增 `backend/src/postprocess.rs`（只处理定稿；指令先原样上屏；热词文件 ≤256 KiB；历史写失败不影响上屏），WSL 5 个测试过；`winh-host/src/broker.rs` 的 `run` 包了一层 output 回调接入，每次唤起重新读设置和热词，并顺手清过期历史
- [x] R6 Windows 全量 `tools/win.sh cargo test --workspace`：22 组 117 个测试全过，0 编译警告（2026-11-15）

**第二轮结果**
- 现状：文字整理和历史记录**已接进 Win+H 出字流程**（只作用于定稿句子）。还没装到主人电脑，**没真机验证**；没推送。
- 真机要验：Win+H 说"嗯，百分之五十"→ 上屏"50%"；`%LOCALAPPDATA%\XueerEar\history.jsonl` 出现这一句；在 `hotwords.txt` 写一个词再说一次看纠错。边说边出字的中间结果不该被整理。
- 没做：设置界面上的开关（现在要手改 settings.json）、执行语音指令（要按键）、「上游有新版」提示、C9、C11、B8 提示音、打包安装。
- 回滚：`git revert` 本轮提交即可；只回退接线就 revert 改 broker.rs 的那个提交。

**这一轮仍不碰**：C9 指纹、C11 键盘钩子 + 模拟键入、Win+H 接管逻辑本身、微信云端协议、打包安装、推送。

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
- [x] 2 textflow crate + 测试（32 个测试，clippy 0 警告）
- [x] 3 history crate + 测试（6 个测试，clippy 0 警告）
- [x] 4 登记 `~/projects/README.md`、共用记忆 `projects/xueer-ear.md`

### 本轮结果

- **现状**：两个新 crate 写完、测完、本地提交；**还没接进程序**，雪耳装到主人电脑上的行为和以前一样（其实也还没装）。没推送。
- **改了什么**：`90eb1c5` 口癖+数字、`3ce9ec5` 标点+语音指令、`d9d2c46` 拼音热词、`4db1a97` 统一入口 `textflow::process` + 终端拼接 `join_for_terminal`、`24d1c72` history。
- **验证**：WSL `cargo test -p xueer-ear-textflow -p xueer-ear-history` 38 个全过；Windows 工具链 `tools/win.sh cargo test -p 两个 crate` 38 个全过；clippy 0 警告。
- **怎么接（给下一个做接管部分的人）**：识别出一句定稿文字后调 `textflow::process(text, &options)`，得到 `Output::Text` 就上屏、`Output::Command` 就按对应键（`Send` 默认不会出现）；终端里连续几句用 `join_for_terminal`。热词文件用 `Hotwords::parse(fs::read_to_string(...))`。上屏后 `History::append`，启动时 `History::prune(now)`。
- **注意**：边说边改的中间结果（partial）建议不过 textflow 或只过热词，定稿（final）再全量处理，否则数字/口癖可能在中途跳变（推测，要真机看）。
- **环境坑（已修）**：WSL 里 cargo 全局配置 `~/tools/dev/cargo/config.toml` 的 linux 链接器原来指向不存在的 `~/tools/dev/zig-cc-host.sh`，2026-11-15 主人同意后改成 `~/tools/dev/bin/zig-cc-host.sh`（备份在 claude scratch/backups）。
- **回滚**：`git revert 24d1c72 4db1a97 d9d2c46 3ce9ec5 90eb1c5`，或从 workspace members 去掉两个 crate。

**已有提交（上一会话）**：`095d4e7` 改名 + 接受 26200.9974 指纹；`deab5ea` recognize-wave-wetype 命令。都只在本地，没推送、没在真机验证。
