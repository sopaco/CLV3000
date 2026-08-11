#!/usr/bin/env bash
# 把 CLV3000 打包成带图标的 macOS .app（依赖 cargo-bundle）。
# 图标 / Info.plist 来自 Cargo.toml 的 [package.metadata.bundle]。
#
# 用法：
#   scripts/bundle-macos.sh
#
# 说明：
#   - 仓库根若有本地 clamav/，会额外拷进 CLV3000.app/Contents/Resources/clamav，
#     使打出的 app 自带扫描引擎（clamav/ 在 .gitignore 里，不入库，故不写进
#     Cargo.toml 的 resources，避免缺目录时 cargo bundle 报错）。
#   - 没有 clamav/ 也能正常打包，只是 app 运行时依赖系统 ClamAV
#     （brew 安装，或 /usr/local/clamav）才能扫描。

set -euo pipefail
cd "$(dirname "$0")/.."

# 1. 确保 cargo-bundle 已安装
if ! command -v cargo-bundle >/dev/null 2>&1; then
  echo ">> 未检测到 cargo-bundle，正在安装 ..."
  cargo install cargo-bundle
fi

# 2. 打包（Release）
echo ">> cargo bundle --release ..."
cargo bundle --release

APP="target/release/clv3000.app"

# 3. 可选：把本地 clamav/ 打进包内 Resources，使 app 自带引擎
if [ -d "clamav" ]; then
  echo ">> 把本地 clamav/ 拷入 $APP/Contents/Resources/clamav ..."
  rm -rf "$APP/Contents/Resources/clamav"
  cp -R clamav "$APP/Contents/Resources/clamav"
else
  echo ">> 仓库根未发现 clamav/：打出的 app 依赖系统 ClamAV（brew 或 /usr/local/clamav）。"
fi

echo ">> 完成：$APP"
open -R "$APP"
