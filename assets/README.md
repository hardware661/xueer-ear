# 图标与常量

`app-icon.svg` 是图标源文件，`app-icon.png` 和 `app-icon.ico` 由它渲染。ICO 含 16、24、32、48、64、128、256 像素七帧。界面、任务栏和托盘共用这一套。

`github-mark.svg` 来自 [Primer Octicons](https://github.com/primer/octicons/blob/main/icons/mark-github-16.svg)，PNG 和 ICO 是同一个矢量图换格式；许可见 `LICENSE-octicons.txt`。

`wetype-bootstrap.bin` 和 `wetype-sign-salt.bin` 是识别客户端用的固定常量，不含设备身份或凭据。匿名身份在程序第一次运行时随机生成，只存在本机数据目录。
