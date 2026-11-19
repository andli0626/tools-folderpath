# downloads/

发布页（仓库根目录 `index.html`）里的下载按钮全部指向本目录的**相对路径**，
不使用 GitHub Release、`raw.githubusercontent.com` 或任何第三方图床/CDN。

## 页面引用的文件名（必须完全一致）

| 页面按钮 | 文件名 |
| --- | --- |
| 通用版 dmg（推荐） | `CopyPath_1.3.0_universal.dmg` |
| 通用版 zip | `CopyPath_1.3.0_universal.zip` |
| Apple Silicon 单架构 | `CopyPath_1.3.0_arm64.zip` |
| Intel 单架构 | `CopyPath_1.3.0_x86_64.zip` |

## 文件从哪来

在本机打包（产物默认落在 `~/Downloads`）：

```bash
./build-app.sh                 # 本机架构，装进 ~/Applications
# 或只产出发行文件、不安装：
./scripts/package-macos.sh
```

再从 GitHub Actions 取通用包（推 `v*` 标签后，Release 资产里已有同名文件）。
把四个文件原样复制到本目录即可：

```bash
cp ~/Downloads/CopyPath_1.3.0_universal.dmg  ./downloads/
cp ~/Downloads/CopyPath_1.3.0_universal.zip  ./downloads/
cp ~/Downloads/CopyPath_1.3.0_arm64.zip      ./downloads/
cp ~/Downloads/CopyPath_1.3.0_x86_64.zip     ./downloads/
```

> 先确认版本号：升级到 1.4.0 时，页面里的文件名与 `downloads/` 里的文件要一起改，
> 否则按钮会 404。发布前用 `scripts/check-site-deps.sh` 顺手核对一遍。

## 注意

- 不要把 dmg/zip 放进 Release 再用 `releases/download/...` 链接 —— 那个域名（`objects.githubusercontent.com`）
  是国内失败率最高的环节。
- 单个文件 ≤ 100 MB、整站 ≤ 1 GB，这是 GitHub Pages 的硬限制。
