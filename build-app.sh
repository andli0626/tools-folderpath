#!/bin/bash
# 打包 CopyPath.app、CopyPath Toolbar.app 与 dmg 安装镜像：dmg 输出到 ~/Downloads，
# .app 装到 ~/Applications。只需本机使用，不做开发者签名与公证。
#
# 三条硬约束：
#   1) 项目在 /Volumes/SanDis1TB 外置盘上，target 默认放内部盘（可用环境变量覆盖）。
#   2) 装到 ~/Applications 而不是留在外置盘：开机自启的 LaunchAgent 会把 .app 的
#      绝对路径写死，开机时盘没挂载自启就静默失效了。
#   3) 不用 `cargo tauri build`：它要多编一遍 tauri_macros，而本机 ld 在这个 crate 上
#      稳定链出坏 dylib。本项目无前端、无窗口，bundler 干的活（合并 Info.plist、
#      塞 .icns）几条 cp/plutil 就能等价完成，dmg 直接交给 hdiutil。
set -euo pipefail

PROJECT_DIR="$(cd "$(dirname "$0")" && pwd)"
APP_NAME="CopyPath"
EXE_NAME="copypath"
BUNDLE_ID="com.lilin.folderpath"
TB_APP_NAME="CopyPath Toolbar"
TB_EXE_NAME="copypath-toolbar"
TB_BUNDLE_ID="com.lilin.folderpath.toolbar"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/Users/lilin/cargo-target/tools-folderpath}"
export CARGO_BUILD_JOBS="${CARGO_BUILD_JOBS:-4}"
INSTALL_DIR="$HOME/Applications"

cd "$PROJECT_DIR/src-tauri"

if [ "$INSTALL_DIR" != "$HOME/Applications" ]; then
    echo "安装目录异常：$INSTALL_DIR" >&2
    exit 1
fi

# 同一个 target 目录上并发跑两个 cargo 会互相写坏产物，这里直接拒绝。
if pgrep -f "[c]argo build" >/dev/null; then
    echo "检测到另一个 cargo build 在跑，等它结束再打包。" >&2
    exit 1
fi

# 本机 ld 会把 proc-macro dylib 写成 LINKEDIT 字符串池未对齐的文件
# （dlopen 报 mis-aligned LINKEDIT string pool，rustc 报 can't find crate）。
# 概率性且每个 crate 独立，重来一次通常就好了；但 cargo 会把坏文件当成新产物复用，
# 所以重试前必须连 .fingerprint 一起删掉，只删 dylib 不会触发重链。
build_with_repair() {
    local attempt crates c
    for attempt in $(seq 1 8); do
        if "$@" > /tmp/folderpath-build-attempt.log 2>&1; then
            [ "$attempt" -gt 1 ] && echo "第 $attempt 次尝试成功"
            return 0
        fi
        crates=$({
            grep -o "can't find crate for \`[A-Za-z0-9_]*\`" /tmp/folderpath-build-attempt.log \
                | sed "s/.*\`\(.*\)\`/\1/" || true
            grep -o "deps/lib[A-Za-z0-9_]*-[0-9a-f]*\.dylib" /tmp/folderpath-build-attempt.log \
                | sed "s|deps/lib\(.*\)-[0-9a-f]*\.dylib|\1|" || true
        } | sort -u)
        if [ -z "$crates" ]; then
            echo "构建失败（不是 dylib 损坏）：" >&2
            tail -20 /tmp/folderpath-build-attempt.log >&2
            return 1
        fi
        echo "第 $attempt 次尝试：重链损坏的 proc-macro：$(echo "$crates" | tr '\n' ' ')" >&2
        for c in $crates; do
            rm -f "$CARGO_TARGET_DIR"/release/deps/lib"$c"-*.dylib
            rm -rf "$CARGO_TARGET_DIR"/release/.fingerprint/"$c"-*
            rm -rf "$CARGO_TARGET_DIR"/release/.fingerprint/"${c//_/-}"-*
        done
    done
    echo "连续 8 次都链不出可用的 proc-macro dylib，先看看磁盘和杀毒软件再重试。" >&2
    return 1
}

build_with_repair cargo build --release --bin "$EXE_NAME" --bin "$TB_EXE_NAME"

VERSION="$(sed -n 's/.*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' tauri.conf.json | head -1)"
[ -n "$VERSION" ] || { echo "读不到 tauri.conf.json 里的 version" >&2; exit 1; }

APP_BUNDLE="$CARGO_TARGET_DIR/release/$APP_NAME.app"
rm -rf "$APP_BUNDLE"
mkdir -p "$APP_BUNDLE/Contents/MacOS" "$APP_BUNDLE/Contents/Resources"

cp "$CARGO_TARGET_DIR/release/$EXE_NAME" "$APP_BUNDLE/Contents/MacOS/$EXE_NAME"
cp icons/icon.icns "$APP_BUNDLE/Contents/Resources/icon.icns"
printf 'APPL????' > "$APP_BUNDLE/Contents/PkgInfo"

# 手工 .app 只认同级 Info.plist（`cargo tauri build` 的自动合并这一步被我们自己接管了）。
plutil -convert binary1 Info.plist -o "$APP_BUNDLE/Contents/Info.plist"
plutil -insert CFBundleShortVersionString -string "$VERSION" "$APP_BUNDLE/Contents/Info.plist"
plutil -insert CFBundleVersion -string "$VERSION" "$APP_BUNDLE/Contents/Info.plist"

# 这里的 ad-hoc 签名不是分发签名（无开发者证书、不公证），而是运行前提：
# Apple Silicon 上未签名的二进制会被内核直接 SIGKILL，实测 exit 137。
codesign --force --deep --sign - --identifier "$BUNDLE_ID" "$APP_BUNDLE"

# 自检：任一不符就别把坏包装进 ~/Applications。
[ "$(plutil -extract LSUIElement raw "$APP_BUNDLE/Contents/Info.plist")" = "true" ] \
    || { echo "LSUIElement 不为 true" >&2; exit 1; }
[ "$(plutil -extract CFBundleIdentifier raw "$APP_BUNDLE/Contents/Info.plist")" = "$BUNDLE_ID" ] \
    || { echo "bundle id 不是 $BUNDLE_ID" >&2; exit 1; }
[ "$(plutil -extract CFBundleExecutable raw "$APP_BUNDLE/Contents/Info.plist")" = "$EXE_NAME" ] \
    || { echo "CFBundleExecutable 不是 $EXE_NAME" >&2; exit 1; }
codesign --verify --strict --verbose=2 "$APP_BUNDLE"

# 工具栏 helper（CopyPath Toolbar.app）：被点击时复制当前文件夹路径后立即退出。
# 与主程序是两个 bundle id、互不依赖；首次点击会弹一次访达自动化授权。
TB_APP_BUNDLE="$CARGO_TARGET_DIR/release/$TB_APP_NAME.app"
rm -rf "$TB_APP_BUNDLE"
mkdir -p "$TB_APP_BUNDLE/Contents/MacOS" "$TB_APP_BUNDLE/Contents/Resources"

cp "$CARGO_TARGET_DIR/release/$TB_EXE_NAME" "$TB_APP_BUNDLE/Contents/MacOS/$TB_EXE_NAME"
cp icons/icon.icns "$TB_APP_BUNDLE/Contents/Resources/icon.icns"
printf 'APPL????' > "$TB_APP_BUNDLE/Contents/PkgInfo"

plutil -convert binary1 Info.toolbar.plist -o "$TB_APP_BUNDLE/Contents/Info.plist"
plutil -insert CFBundleShortVersionString -string "$VERSION" "$TB_APP_BUNDLE/Contents/Info.plist"
plutil -insert CFBundleVersion -string "$VERSION" "$TB_APP_BUNDLE/Contents/Info.plist"

codesign --force --deep --sign - --identifier "$TB_BUNDLE_ID" "$TB_APP_BUNDLE"

[ "$(plutil -extract LSUIElement raw "$TB_APP_BUNDLE/Contents/Info.plist")" = "true" ] \
    || { echo "Toolbar: LSUIElement 不为 true" >&2; exit 1; }
[ "$(plutil -extract CFBundleIdentifier raw "$TB_APP_BUNDLE/Contents/Info.plist")" = "$TB_BUNDLE_ID" ] \
    || { echo "Toolbar: bundle id 不是 $TB_BUNDLE_ID" >&2; exit 1; }
[ "$(plutil -extract CFBundleExecutable raw "$TB_APP_BUNDLE/Contents/Info.plist")" = "$TB_EXE_NAME" ] \
    || { echo "Toolbar: CFBundleExecutable 不是 $TB_EXE_NAME" >&2; exit 1; }
codesign --verify --strict --verbose=2 "$TB_APP_BUNDLE"

# dmg：卷里放两个 .app 和一个指向 /Applications 的软链接，拖一下就算装好。
# 产物落在 ~/Downloads，不进仓库目录。
DMG="$HOME/Downloads/${APP_NAME}_${VERSION}_universal.dmg"
STAGE="$CARGO_TARGET_DIR/release/dmg-staging"
rm -rf "$STAGE" && mkdir -p "$STAGE"
cp -R "$APP_BUNDLE" "$STAGE/"
cp -R "$TB_APP_BUNDLE" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -quiet -format UDZO "$DMG"
rm -rf "$STAGE"
echo "安装包：$DMG"

# 旧实例还占着单实例锁和托盘，先停掉再换。
pkill -x "$EXE_NAME" 2>/dev/null || true
sleep 1
rm -rf "$INSTALL_DIR/$APP_NAME.app" "$INSTALL_DIR/FolderPath.app" "$INSTALL_DIR/$TB_APP_NAME.app"
cp -R "$APP_BUNDLE" "$TB_APP_BUNDLE" "$INSTALL_DIR/"
echo "已安装：$INSTALL_DIR/$APP_NAME.app + $TB_APP_NAME.app（v$VERSION）"
open "$INSTALL_DIR/$APP_NAME.app"
