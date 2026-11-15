# STATE · 雪耳 xueer-ear

> 任何 AI 接手先读这里；每次会话收尾在最上面追加一节（最新在上），写清做了什么、验证了什么、还差什么。
> 计划原文：主人桌面 `雪耳xueer-ear计划-v1.md`（v1.1）。

## 2026-11-15 14:35 · 真机问题二（Claude；主人："说话的时候会弹出来一个弹窗…某些语音键入功能在此应用中可能不起作用…打断我的语音输入"；"说着说着它就突然给我停住了…可能是有时间长度的要求"）

- [ ] G1/G2 分析（service.log 22 次唤起，全部在 Windows Terminal 里）：
  - 11 次 `停止`（微软面板发 Stop）、8 次 `正常结束 检测到手动输入`（终端输入保护数到了别的键鼠输入）、1 次 `终端听写文字无效`（云端文字带控制字符，整次被拒）。
  - 被切的几乎都是**还没出过定稿、连续说了 7～10 秒**的；中途有停顿（出过定稿或 paused）的能撑到 24 秒。（推测）终端模式下 broker 只往终端打字、不给面板发 Partial/Final，面板长时间收不到结果就弹「某些语音键入功能在此应用中可能不起作用」并停止。
  - 「检测到手动输入」到底是哪个键/鼠标动作，日志里没有，要补。
- [x] G3 终端路径遇到控制字符换成空格，不再整次放弃（`revision.rs` + 测试）
- [x] G4 日志补上：`正常结束` 带「来源 键盘/鼠标/原始… 键码」和秒数，`停止` 带「面板要求停止（本轮 N 秒）」。14:34 已重装（Windows 134 测试过）并以自启方式拉起，status ready
- [x] G5 新日志（14:35～14:39，5 次，全在终端）：3 次 `面板要求停止（本轮 30.3 秒）`、1 次 9.3 秒、1 次 `正常结束 来源 原始键盘 0xd`（回车，主人发消息，属正常）。**结论：微软面板在终端里约 30 秒就自己停，雪耳没误判**。（推测）终端模式下雪耳直接往终端打字、不给面板回 Partial/Final，面板以为这个应用不支持语音键入，弹「某些语音键入功能…可能不起作用」并按自己的超时停下
- [ ] G7 主人另报：某次按 Win+H 右下角弹「正在使用麦克风」，前几秒说话没识别。日志看正常情况下开口到上传约 1～2 秒（连接时音频先排队，最多 128 块，开口前 0.36 秒也会补上），没找到丢字证据；没对上是哪一次，待再遇到时记下时间
- [ ] G8 14:48 右 Alt 说了一长句没上屏：service.log `快捷键听写失败：IO error: 重叠 I/O 操作在进行中。 (os error 997)`，前一刻 Win+H 第 33 轮刚因回车结束。待查（一次一个假设）：快捷键会话和刚结束的 Win+H 会话抢同一个连接/设备
- 主人 14:49 要的方向：「Win+H 的展示效果 + 右 Alt 的能力」，不喜欢那几种按法方案；问朋友怎么做的——朋友只有 Win+H，终端里同样直接往终端打字（和我们一样会被面板 30 秒停），没有快捷键模式
- [ ] G6 待定修法：A 终端里也给面板回进度（要先验证面板会不会在终端里重复上屏）；B 终端里改用右 Alt 快捷键模式（不经过面板）。先请主人在记事本里连说 40 秒，确认非终端的应用不受 30 秒限制

## 2026-11-15 14:20 · 真机反馈处理中（Claude；主人："我按win+H给我弹出来的是那个它自带的那个微软的…点了一下雪耳的那个托盘，然后又貌似可以直接使用"；"nav可以登记"；"公开仓可以先建私有的嘛，然后再转公开"）

- [x] F1 结论：**不是故障**。Win+H 的面板永远是微软那个（设计如此：只换底下的识别，不换面板）。service.log（14:03:20 起）里三次唤起都由雪耳接受：[1] PCM peak≤34、上传 0，就是没录到说话就停了；[2] 识别出 partial 但停止时没出定稿；[3] 正常出定稿并记进历史。托盘左键只是打开面板，不会改变接管状态。后续可做：面板看不出是谁在识别，计划里 B6「听写时托盘耳朵抖动」能补这个提示（没做）
- [x] F2 nav 登记：oracle-infra `0abcd74`（services.yaml repo+external 服务、owner-access app-auth）已 push.sh 同步；hub 06:14 书签私密 21→22，登录 nav.hdwde.com 可见「雪耳」→ GitHub 仓
- [x] F3 GitHub 私有仓 https://github.com/hardware661/xueer-ear 已建，main 推到 `712e1f2`（推前 gitleaks 33 个提交无泄漏）；第一次 CI 因 cargo fmt 检查失败，格式化后 `671b7cb` 的 CI（Windows 测试 + MSVC 静态构建 + 双包）通过。转公开、打标签、发 Release 仍每次先问

## 2026-11-15 第三轮 · 剩下的全做（Claude；主人原话："我觉得没有什么不安全的 很正常的需求 你可以继续做"）

- [x] T1 Inno Setup 6.7.4 → `D:\devtool\InnoSetup6`（签名有效，当前用户安装）；PowerShell 7.7.0 便携 → `D:\devtool\pwsh`（SHA-256 与 GitHub 公布一致）；都经 ghfast.top 镜像下载
- [x] T2 `tools/win.sh pwsh-script packaging/build-windows.ps1` 出包成功：portable.zip 1.6 MB、setup.exe 3.4 MB、SHA256SUMS（在 `D:\projects\_build\xueer-ear\dist\release`）。修了三处：crate description 改 ASCII（Windows PowerShell 解析中文 JSON 出错）、win.sh 给打包补 PATH/ISCC/RunAsInvoker、build 脚本认 `XUEER_EAR_ISCC`
- [x] T3 C9：`install.rs` 指纹/路径对不上返回 `Unsupported`；`broker` 认出后不接管 Win+H，只开快捷键模式，状态栏显示原因（原生 Win+H 照常）
- [x] T4 C11：`hotkey.rs`（只轮询热键状态，不装全局钩子；SendInput Unicode 键入）+ `hotkey_state.rs`（误触 <300ms、组合键取消）+ 音源 `MicrophoneHold`（松开后这句正常收尾）。设置 `hotkey_enabled`（默认开）、`hotkey_vk`（默认右 Alt 0xA5，只允许 F 键/修饰键/CapsLock/ScrollLock）
- [x] T5 A4：快捷键路径执行指令（`typing.rs`：终端换行用 Shift+Enter、删上一句按字数退格、发送默认关）；Win+H 路径仍原样上屏（那条路的上屏由系统输入面板负责）。Windows 全量 129 个测试通过
- [x] T6 控制面板加「文字与历史」一行（`gui/src/prefs.rs` 菜单逻辑 4 个测试）：6 个文字开关、快捷键模式、记录历史、保留 1/7/30/90 天、用记事本打开历史和热词表。窗口高 530→588；主色从绿换成深灰（B6）。截图看过布局；菜单弹出要真机点
- [x] T7 B8：`assets/sounds/{missed,fallback}.wav`（Edge 晓晓生成、16k 单声道、去首尾静音，共 ~100 KB）打包进 host；快捷键模式没识别出字时放「这次没听清」，指纹不符退回时放「系统更新了…」。设置 `sounds_enabled` 默认开，面板菜单能关。没做「雪儿在听」（Win+H 自己有提示音，快捷键按下就响太吵）
- [x] T8 2026-11-15 14:05 静默安装 0.1.0 到 `%LOCALAPPDATA%\Programs\xueer-ear`，勾了开机自启（启动文件夹 `雪耳.lnk --autostart`）；以自启方式拉起后 host 在跑、status.json `phase=ready`（已接管 Win+H）。CLI 用晓晓合成 WAV 走云端识别通过（「把甲骨文的内存加到50%，然后重启New API。」）。**待主人真机验收**：Win+H 和按住右 Alt 各在记事本、Chrome、Windows Terminal 说几句
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
