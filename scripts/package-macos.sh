#!/usr/bin/env bash
# 把 arm64 与 x86_64 两份 release 二进制合成 macOS 通用包：
#   CopyPath.app、CopyPath Toolbar.app、通用版 dmg/zip，以及按架构拆分的 zip。
#
# 与根目录 build-app.sh 的分工：build-app.sh 服务于「本机自用」，会装进
# ~/Applications、重启旧进程；本脚本只负责产出可分发文件，不碰用户目录，
# 供 GitHub Actions 在干净的 macOS runner 上调用。
#
# 用法: scripts/package-macos.sh <bin-root> <out-dir>
#   <bin-root>/arm64/{copypath,copypath-toolbar}
#   <bin-root>/x86_64/{copypath,copypath-toolbar}
set -euo pipefail

BIN_ROOT="${1:?用法: package-macos.sh <bin-root> <out-dir>}"
OUT_DIR="${2:?用法: package-macos.sh <bin-root> <out-dir>}"

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
SRC_DIR="$PROJECT_DIR/src-tauri"

APP_NAME="CopyPath"
EXE_NAME="copypath"
TB_APP_NAME="CopyPath Toolbar"
TB_EXE_NAME="copypath-toolbar"
BUNDLE_ID="com.lilin.folderpath"
TB_BUNDLE_ID="com.lilin.folderpath.toolbar"

if [ ! -d "$BIN_ROOT/arm64" ] || [ ! -d "$BIN_ROOT/x86_64" ]; then
    echo "缺少二进制目录：需要 $BIN_ROOT/arm64 和 $BIN_ROOT/x86_64" >&2
    exit 1
fi

# 版本号只认 tauri.conf.json 这一处真源（Cargo.toml 里那份是给 cargo 看的）。
VERSION="$(
    python3 -c 'import json,sys;print(json.load(open(sys.argv[1]))["version"])' \
        "$SRC_DIR/tauri.conf.json"
)"
[ -n "$VERSION" ] || { echo "读不到 tauri.conf.json 里的 version" >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/copypath-pkg.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

# 组装单个 .app：合并 plist、塞图标、写版本号、ad-hoc 签名并自检。
# 这里的签名不是分发签名（无开发者证书、不公证），而是运行前提：
# Apple Silicon 上未签名的二进制会被内核直接 SIGKILL。
make_bundle() {
    local exe="$1" plist="$2" app="$3" bundle_id="$4" exec_name="$5" dest="$6"
    local bundle="$dest/$app"

    rm -rf "$bundle"
    mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"

    cp "$exe" "$bundle/Contents/MacOS/$exec_name"
    cp "$SRC_DIR/icons/icon.icns" "$bundle/Contents/Resources/icon.icns"
    printf 'APPL????' > "$bundle/Contents/PkgInfo"

    plutil -convert binary1 "$plist" -o "$bundle/Contents/Info.plist"
    plutil -insert CFBundleShortVersionString -string "$VERSION" "$bundle/Contents/Info.plist"
    plutil -insert CFBundleVersion -string "$VERSION" "$bundle/Contents/Info.plist"

    codesign --force --deep --sign - --identifier "$bundle_id" "$bundle"

    [ "$(plutil -extract LSUIElement raw "$bundle/Contents/Info.plist")" = "true" ] \
        || { echo "$app: LSUIElement 不为 true" >&2; exit 1; }
    [ "$(plutil -extract CFBundleIdentifier raw "$bundle/Contents/Info.plist")" = "$bundle_id" ] \
        || { echo "$app: bundle id 不是 $bundle_id" >&2; exit 1; }
    [ "$(plutil -extract CFBundleExecutable raw "$bundle/Contents/Info.plist")" = "$exec_name" ] \
        || { echo "$app: CFBundleExecutable 不是 $exec_name" >&2; exit 1; }
    codesign --verify --strict "$bundle"
}

# dmg 里放两个 .app 和一个指向 /Applications 的软链接，拖一下就算装好。
make_dmg() {
    local src="$1" dmg="$2"
    rm -f "$dmg"
    hdiutil create -volname "$APP_NAME" -srcfolder "$src" -ov -quiet -format UDZO "$dmg"
}

# ditto 保留符号位与资源分支，比 zip 更贴合 .app 的分发习惯。
make_zip() {
    local src="$1" zip_path="$2"
    rm -f "$zip_path"
    ( cd "$src" && ditto -c -k --sequesterRsrc --zlibCompressionLevel 9 . "$zip_path" )
}

stage_apps() {  # $1 输出目录  $2 exe 目录  $3 架构标签（universal/arm64/x86_64）
    local dest="$1" exe_dir="$2" label="$3"
    mkdir -p "$dest"
    make_bundle "$exe_dir/$EXE_NAME" "$SRC_DIR/Info.plist" "$APP_NAME.app" "$BUNDLE_ID" "$EXE_NAME" "$dest"
    make_bundle "$exe_dir/$TB_EXE_NAME" "$SRC_DIR/Info.toolbar.plist" "$TB_APP_NAME.app" "$TB_BUNDLE_ID" "$TB_EXE_NAME" "$dest"
    ln -s /Applications "$dest/Applications"
    echo "已组装 ${label}：${dest}"
}

# ---------- 通用版（arm64 + x86_64 用 lipo 合成一份） ----------
UNIV_EXE="$WORK/universal-bin"
mkdir -p "$UNIV_EXE"
lipo -create "$BIN_ROOT/arm64/$EXE_NAME" "$BIN_ROOT/x86_64/$EXE_NAME" -output "$UNIV_EXE/$EXE_NAME"
lipo -create "$BIN_ROOT/arm64/$TB_EXE_NAME" "$BIN_ROOT/x86_64/$TB_EXE_NAME" -output "$UNIV_EXE/$TB_EXE_NAME"

for e in "$EXE_NAME" "$TB_EXE_NAME"; do
    archs="$(lipo -archs "$UNIV_EXE/$e")"
    case "$archs" in
        *arm64*x86_64* | *x86_64*arm64*) ;;
        *) echo "$e 不是通用二进制：$archs" >&2; exit 1 ;;
    esac
done

STAGE_UNIV="$WORK/stage-universal"
stage_apps "$STAGE_UNIV" "$UNIV_EXE" "通用版"

make_dmg "$STAGE_UNIV" "$OUT_DIR/${APP_NAME}_${VERSION}_universal.dmg"
make_zip "$STAGE_UNIV" "$OUT_DIR/${APP_NAME}_${VERSION}_universal.zip"

# ---------- 单架构版（给只想下对应体积的用户，也便于按机器排查） ----------
for arch in arm64 x86_64; do
    STAGE_ARCH="$WORK/stage-$arch"
    stage_apps "$STAGE_ARCH" "$BIN_ROOT/$arch" "$arch"
    make_zip "$STAGE_ARCH" "$OUT_DIR/${APP_NAME}_${VERSION}_${arch}.zip"
done

echo "版本：$VERSION"
ls -1sh "$OUT_DIR" | sed 's/^/  /'
