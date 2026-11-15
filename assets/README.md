# 图标与常量

`app-icon.svg` 是图标源文件（深灰圆角底、一对白色兽耳、下方一道声波，手画矢量），`app-icon.png` 和 `app-icon.ico` 由它渲染。ICO 含 16、24、32、48、64、128、256 像素七帧。界面、任务栏和托盘共用这一套。

`github-mark.svg` 来自 [Primer Octicons](https://github.com/primer/octicons/blob/main/icons/mark-github-16.svg)，PNG 和 ICO 是同一个矢量图换格式；许可见 `LICENSE-octicons.txt`。

`sounds/` 里是 Edge 晓晓生成的提示音（16 kHz 单声道 WAV），打包进 host 本地播放。

`wetype-bootstrap.bin` 和 `wetype-sign-salt.bin` 是识别客户端用的固定常量，不含设备身份或凭据。匿名身份在程序第一次运行时随机生成，只存在本机数据目录。
