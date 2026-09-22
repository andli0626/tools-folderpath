# CopyPath

macOS 菜单栏小工具：把访达当前窗口或所选项目的绝对路径一键写进剪贴板。

```
/Volumes/SanDis1TB/WK-Tools/tools-folderpath
```

Rust + Tauri v2，无前端界面（不创建任何窗口），路径通过 `osascript` 读访达。当前版本 1.0.0。

## 安装

拿到 `release/CopyPath_1.0.0_universal.dmg` 后双击挂载，把 **CopyPath.app 拖到 Applications** 即可。
应用是 ad-hoc 签名（`codesign -s -`）而非公证，首次打开会被 Gatekeeper 拦住，用
**右键 → 打开 → 再点一次「打开」**过掉，之后正常双击。

从源码打包（同时产出 `.app` 与 `.dmg`，并把 `.app` 装进 `~/Applications` 后启动）：

```bash
./build-app.sh
```

产物必须装在内部盘的 `~/Applications`，不要留在外置盘：开机自启的 LaunchAgent 会把 `.app`
的绝对路径写死，开机时盘没挂载自启就静默失效。

## 用法

| 触发 | 行为 |
| --- | --- |
| <kbd>⌃</kbd><kbd>⌥</kbd><kbd>⌘</kbd><kbd>P</kbd> | 复制"所在文件夹"路径 |
| 菜单栏图标（左键或右键） | 展开菜单 |

菜单项：

- **立即获取文件夹路径** —— 与快捷键相同
- **复制选中项自身路径** —— 选中什么复制什么，不做归一化
- **最近路径** —— 最多 10 条，点一条即复制；底部"清空历史"
- **开机自启**
- **退出 CopyPath**
- **版本 1.0.0** —— 只读，取自 `tauri.conf.json`，不写死在代码里

### 路径语义

选中项优先，没有选中项时回退到前置窗口目录：

| 访达里的状态 | 「立即获取文件夹路径」 | 「复制选中项自身路径」 |
| --- | --- | --- |
| 无选中项 | 当前窗口目录 | 当前窗口目录 |
| 选中 1 个文件夹 | 该文件夹自身 | 该文件夹自身 |
| 选中 1 个文件 | 它所在的目录 | 该文件自身 |
| 选中同目录下 3 个文件 | 该目录（去重成 1 行） | 3 行 |
| 选中多个目录 | 每个目录一行 | 每个条目一行 |

多选时剪贴板里每行一个路径。结果同时发一条原生通知；通知失败不影响已写入的剪贴板。

## 首次运行授权

读访达需要自动化权限，第一次触发时系统会弹窗，允许即可：

**系统设置 → 隐私与安全性 → 自动化 → CopyPath → 打开「访达」**

## 数据位置

| 用途 | 路径 |
| --- | --- |
| 最近路径 | `~/Library/Application Support/com.lilin.folderpath/history.json` |
| 单实例锁 | 同目录 `instance.lock` |
| 开机自启 | `~/Library/LaunchAgents/CopyPath.plist` |

bundle identifier 仍是 `com.lilin.folderpath`（应用早期叫 FolderPath，改名时特意保留），
所以数据目录和已授权的自动化权限都跟着原名走，不必重新授权。

## 开发

```bash
cd src-tauri
cargo test                       # 解析器与历史落盘的单元测试，不依赖活的访达
cargo run -- --probe folder      # 不启界面，直接打印解析结果
cargo run -- --probe item
python3 ../assets/make-icons.py  # 重画图标母图与托盘模板
cargo tauri icon ../assets/icon-source.png   # 生成 icns/png 全套
```

`--probe` 是逐项验证路径语义最快的办法：在访达里摆好状态，然后打印结果，不用点托盘。

图标按 Apple HIG 的 macOS 栅格画：1024 画布、图形区 824×824 居中（四周各留 100pt）、
圆角半径 185、外部透明、不烤系统阴影。托盘图是纯黑字形 + 透明底，靠
`icon_as_template(true)` 让系统在深浅色菜单栏下自动反色。

## 故障排查

**报 -1743 / 「未获得授权」**
ad-hoc 重签名会改变 cdhash，旧的自动化授权随之失效。重置后重新授权一次：

```bash
tccutil reset AppleEvents com.lilin.folderpath
```

**「没有打开的访达窗口」**
脚本刻意不使用 `insertion location`（0 窗口时它会静默返回桌面，是个错答案），
所以没有窗口就是失败，不会动剪贴板。

**快捷键没反应**
可能被别的应用占用了。注册失败只在 stderr 打日志并降级，托盘菜单仍可用。
换键改 `src-tauri/src/lib.rs` 的 `SHORTCUT` 常量后重新打包。

**托盘图标不见了 / 起了两个**
`pgrep -x copypath` 看进程；重复启动会被单实例锁挡掉并打印提示。

**构建报 `can't find crate for xxx_derive` / `mis-aligned LINKEDIT string pool`**
本机 `ld` 偶发把 proc-macro dylib 写坏，且 cargo 会把坏文件当成新产物复用。
`build-app.sh` 会自动删掉坏产物（含 `.fingerprint`）重链，最多 8 次。
手动处理时只删 `release/deps/lib<crate>-*.dylib` 不够，必须连
`release/.fingerprint/<crate>-*` 一起删，否则不会重新链接。

**通知不弹**
系统设置 → 通知 里确认「CopyPath」允许通知。通知只是反馈，复制本身已成功。

## 已知限制

- **左键不直接取路径。** 挂了菜单的 `NSStatusItem` 会在 `mouseDown` 里自己把菜单弹出来，
  tray-icon 铺在图标上的事件覆盖层因此收不到左键（实测右键收得到）。也就是
  "左键一键复制"和"右键出菜单"在同一个图标上不可兼得，一键交给了全局快捷键。
- 选中 `.framework`、`.bundle` 这类包目录时，访达报的类别是 `folder`，会复制包自身路径。
- 文件名里含换行的条目会被丢弃并在 stderr 告警，而不是拼进上一条路径。
- 全局快捷键是编译期常量，没有设置界面。
