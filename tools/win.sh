#!/usr/bin/env bash
# 在 Windows 上跑 cargo / 打包：先把 WSL 源码同步到 D:\projects\_build\xueer-ear，再进 VS 开发者环境执行。
# 源码以 WSL 这份为准；D 盘那份只是构建副本（target/ 和 dist/ 留在 D 盘复用缓存）。
# 用法：tools/win.sh cargo test --workspace
#       tools/win.sh pwsh-script packaging/build-windows.ps1 [参数...]
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MIRROR=/mnt/d/projects/_build/xueer-ear
WIN_MIRROR='D:\projects\_build\xueer-ear'
mkdir -p "$MIRROR"
rsync -a --delete --exclude target/ --exclude dist/ --exclude .git/ "$ROOT"/ "$MIRROR"/

PWSH=powershell.exe
[ -x /mnt/d/devtool/pwsh/pwsh.exe ] && PWSH=/mnt/d/devtool/pwsh/pwsh.exe

if [ "${1:-}" = pwsh-script ]; then
  shift; script="$1"; shift
  # 打包脚本要 cargo、python 和 ISCC；本机都装在 D:\devtool，不在 Windows 的 PATH 里
  exec "$PWSH" -NoProfile -ExecutionPolicy Bypass -Command "
\$env:Path = 'D:\devtool\rust\cargo\bin;D:\devtool\Python\python314;' + \$env:Path
\$env:XUEER_EAR_ISCC = 'D:\devtool\InnoSetup6\ISCC.exe'
# GUI 清单要求 highestAvailable；测试进程按调用者权限跑，免得 cargo test 报 740
\$env:__COMPAT_LAYER = 'RunAsInvoker'
& '$WIN_MIRROR\\${script//\//\\}' $*
exit \$LASTEXITCODE
" 2>&1 | tr -d '\r'
fi

cmd="$*"
exec "$PWSH" -NoProfile -ExecutionPolicy Bypass -Command "
\$ErrorActionPreference='Stop'
\$env:Path = 'D:\devtool\rust\cargo\bin;' + \$env:Path
\$vs = & \"\${env:ProgramFiles(x86)}\Microsoft Visual Studio\Installer\vswhere.exe\" -latest -products * -property installationPath
& \"\$vs\Common7\Tools\Launch-VsDevShell.ps1\" -Arch amd64 -HostArch amd64 -SkipAutomaticLocation | Out-Null
\$env:RUSTFLAGS='-C target-feature=+crt-static'
# GUI 清单要求 highestAvailable；测试进程按调用者权限跑，免得 cargo test 报 740
\$env:__COMPAT_LAYER='RunAsInvoker'
Set-Location '$WIN_MIRROR'
$cmd
exit \$LASTEXITCODE
" 2>&1 | tr -d '\r'
