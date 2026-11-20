<div align="center">

<img src="assets/app-icon.svg" width="88" height="88" alt="Xueer Ear icon" />

# Xueer Ear (雪耳)

**Tap Right Alt, speak, and the words stream into the cursor as you talk.**

Chinese voice typing for Windows 11. It also works in terminal apps such as Claude Code and Codex.<br>
Recognition runs on WeChat Input Method's cloud service: no model download, no sign-in, and the installer is under 4 MB.

[![Release](https://img.shields.io/github/v/release/hardware661/xueer-ear)](https://github.com/hardware661/xueer-ear/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/hardware661/xueer-ear/total)](https://github.com/hardware661/xueer-ear/releases)
[![Windows 11](https://img.shields.io/badge/Windows-11-0078d4)](#install)
[![License](https://img.shields.io/github/license/hardware661/xueer-ear)](LICENSE)

[Download](https://github.com/hardware661/xueer-ear/releases/latest) · [中文说明](README.md) · [Changelog](CHANGELOG.md) · [Issues](https://github.com/hardware661/xueer-ear/issues)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="docs/images/panel-dark.png">
  <img src="docs/images/panel.png" width="352" alt="The cat-eared dictation panel at the top of the screen: settings on the left, a live waveform in the middle, translate on the right" />
</picture>

</div>

> [!NOTE]
> The interface is in Chinese, and recognition is tuned for Mandarin (English words mixed into Chinese sentences are fine). The rest of this page is a short English summary; [README.md](README.md) has the full manual.

## What it does

- Streaming dictation: text appears at the cursor while you are still talking. When you pause, the sentence is cleaned up (filler words, numbers, hotwords) and dictation keeps going.
- Works in terminals: one session can last 30 minutes. Saying "换行" (new line) sends Ctrl+J, so Claude Code gets a line break instead of a submit; "发送" (send) presses Enter.
- No model download: recognition and translation use WeChat Input Method's cloud service, so nothing heavy runs on your PC. You do not need WeChat installed or an account.
- Voice commands: delete the last sentence, delete everything, send, and translate the whole round into English or Chinese.
- Hotwords: add a term once and misheard product names get corrected. A built-in list covers common AI and programming terms.
- Follows Windows: dark mode follows the system theme, and the installed version updates itself in one click (download, SHA-256 check, silent install, restart).

<p align="center">
  <img src="docs/images/control-panel.png" width="720" alt="Control panel in light and dark mode" />
</p>

## How it compares

| | Xueer Ear | Windows voice typing (Win+H) | [CapsWriter-Offline](https://github.com/HaujetZhao/CapsWriter-Offline) | [Handy](https://github.com/cjpais/Handy) |
| --- | --- | --- | --- | --- |
| Output | Streams while you talk | Streams while you talk | Whole result after you release the key | Whole result after you stop recording |
| Recognition | WeChat Input Method cloud | Microsoft cloud | Local offline model | Local offline model |
| Model download | No | No | Yes | Yes |
| Works offline | No | No | Yes | Yes |
| Hotwords | Yes | No | Yes | Yes |
| Platforms | Windows 11 | Windows 10, 11 | Windows 10, 11 | Windows, macOS, Linux |
| Open source | MIT | No | MIT | MIT |

The trade-off: Xueer Ear needs a network connection and sends your audio to WeChat Input Method's servers for recognition. In return you get streaming output with no local model. If you need offline recognition or want audio to stay on your machine, use CapsWriter-Offline or Handy.

In our tests, Win+H stops after about 30 seconds in a terminal. Xueer Ear does not go through Win+H, so it has no such limit.

## Install

You need Windows 11, a network connection and a microphone. Administrator rights are not required.

1. Download `xueer-ear-<version>-windows-x64-setup.exe` from the [releases page](https://github.com/hardware661/xueer-ear/releases/latest). A portable ZIP is also available.
2. Run the installer. It is not code-signed, so SmartScreen may say "Windows protected your PC"; click "More info", then "Run anyway". You can check the file against `SHA256SUMS.txt`.
3. On first launch the background service starts automatically.
4. Click into any text field, tap Right Alt and start talking. Tap it again, click the panel or press Esc to stop. Dictation also stops after 10 seconds without new text.

The hotkey can be changed to Right Ctrl, CapsLock, F8 or F9 in the control panel.

## Voice commands

Say a command on its own, with a short pause before and after:

| Say | Effect |
| --- | --- |
| 换行 | New line (Ctrl+J in terminals; turn this off for plain bash/zsh, where Ctrl+J runs the command) |
| 删除 / 删掉上一句 | Delete the last sentence Xueer Ear typed |
| 全部删除 / 清空 | Delete everything typed in this session |
| 发送 | Press Enter (off by default) |
| 翻译成英文 | Translate this round's text into English, leaving English parts as they are |
| 翻译成中文 | Translate this round's text into Chinese |
| 翻译 | Chinese text becomes English; all-English text becomes Chinese |

Commands only touch text that Xueer Ear typed in the current session.

## Privacy

- Audio is streamed to WeChat Input Method's cloud for recognition while you dictate. Xueer Ear does not save recordings.
- Text you translate is sent to the translation service in use (WeChat Input Method by default; Microsoft, Google or Zhipu only if you add a key).
- No account: a random anonymous device identity is created on first run and kept on your PC.
- Settings, hotwords, history and logs live in `%LOCALAPPDATA%\XueerEar`. Apart from recognition and translation, the app only contacts GitHub anonymously to check for and download updates.

## Integration: dictation status

Xueer Ear writes `%LOCALAPPDATA%\XueerEar\status.json` at least every 5 seconds while the service runs. Other programs can read `dictating` (true during a dictation round) and `updated_ms` (Unix milliseconds; treat more than 15 seconds old as "not dictating") to stay quiet while you talk. [Xueer Speaker](https://github.com/hardware661/xueer-speaker) uses it to hold announcements until you finish.

## Build

A full build needs Windows with the Rust MSVC toolchain, Visual Studio C++ build tools, the Windows SDK and CMake; the installer also needs Inno Setup 6.

```powershell
cargo test --workspace
./packaging/build-windows.ps1   # PowerShell 7: builds the portable ZIP and the installer
```

See [CONTRIBUTING.md](CONTRIBUTING.md) before sending a pull request.

## Credits

- [weiwin](https://github.com/Elnyxn/weiwin) (Elnyxn, MIT): Xueer Ear is built on it; the WeChat Input Method cloud recognition client comes from weiwin.
- [CapsWriter-Offline](https://github.com/HaujetZhao/CapsWriter-Offline) (Haujet Zhao, MIT): part of the built-in hotword list follows its examples.

## License

[MIT](LICENSE)
