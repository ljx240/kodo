#!/usr/bin/env bash
#
# Kodo 一键启动。可从任意目录调用。
#
#   ./start.sh          启动桌面应用（Tauri：起 Vite，再编译并运行 Rust 外壳）
#   ./start.sh --ui     只启动浏览器界面（跳过 Rust 编译，秒开，适合调界面）

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DESKTOP="$ROOT/apps/desktop"
PORT=1420

usage() {
  cat <<'EOF'
用法: ./start.sh [--ui]

  （无参数）   启动桌面应用：Tauri 会先起 Vite，再编译并运行 Rust 外壳
  --ui         只启动浏览器界面（Vite），跳过 Rust 编译
  -h, --help   显示本说明
EOF
}

mode=app
for arg in "$@"; do
  case "$arg" in
    --ui | ui) mode=ui ;;
    -h | --help)
      usage
      exit 0
      ;;
    *)
      echo "未知参数：$arg" >&2
      usage >&2
      exit 2
      ;;
  esac
done

# ---------------------------------------------------------------- toolchain --

missing=""
need() { command -v "$1" >/dev/null 2>&1 || missing="$missing $1"; }
need node
need npm
if [ "$mode" = app ]; then
  need cargo
fi
if [ -n "$missing" ]; then
  echo "缺少必需命令:$missing" >&2
  echo "node/npm 见 https://nodejs.org，cargo 见 https://rustup.rs。" >&2
  exit 1
fi

# ------------------------------------------------------------------- deps ----

# 依赖安装不自动执行，先问一次。
if [ ! -d "$DESKTOP/node_modules" ]; then
  echo "未安装前端依赖：$DESKTOP/node_modules 不存在。"
  printf "现在执行 npm install ? [y/N] "
  read -r reply
  case "$reply" in
    [yY]*) (cd "$DESKTOP" && npm install) ;;
    *)
      echo "已取消。请手动执行：cd apps/desktop && npm install" >&2
      exit 1
      ;;
  esac
fi

# ------------------------------------------------------------------- port ----

# vite 配的是 strictPort，端口被占会直接失败；这里先把占用者报出来。
if lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >/dev/null 2>&1; then
  echo "端口 $PORT 已被占用：" >&2
  lsof -nP -iTCP:"$PORT" -sTCP:LISTEN >&2
  echo "请先停掉上面的进程，再重新运行本脚本。" >&2
  exit 1
fi

# ----------------------------------------------------------------- launch ----

cd "$DESKTOP"

if [ "$mode" = app ]; then
  echo "启动桌面应用（首次需要编译 Rust，耗时较长）..."
  exec npm run tauri dev
fi

echo "启动界面开发服务器： http://localhost:$PORT"
echo
echo "  /ui-demo/conversation                     对话（Inspector 展开）"
echo "  /ui-demo/conversation?inspector=closed    对话（Inspector 折叠）"
echo "  /ui-demo/trace                            响应轨迹"
echo "  /ui-demo/archive                          归档"
echo "  /ui-demo/settings                         设置"
echo
echo "参考视口 1586×992。Ctrl+C 停止。"
exec npm run dev
