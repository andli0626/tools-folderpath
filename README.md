# CopyPath

macOS 菜单栏小工具：把访达当前窗口或所选项目的绝对路径一键写进剪贴板。

```
/Volumes/SanDis1TB/WK-Tools/tools-folderpath
```

Rust + Tauri v2，无 Web 前端（悬浮按钮是原生 AppKit 面板），路径通过 `osascript` 读访达。当前版本 1.2.1。

**开源地址**：<http://192.168.0.200/lil/tools-folderpath>

**要求**：macOS 12.0+，Apple Silicon 或 Intel（产物为 universal 包）。

## 功能一览

- **屏幕悬浮按钮**：常驻可拖动的小胶囊，点一下即复制选中项自身路径，右键切换语义或隐藏
- **全局快捷键** <kbd>⌃</kbd><kbd>⌥</kbd><kbd>⌘</kbd><kbd>P</kbd>：复制「所在文件夹」路径，无需切换窗口
- **菜单栏托盘**：左键或右键图标展开菜单，含两种复制语义
- **访达工具栏按钮**（CopyPath Toolbar.app）：点一下即复制当前文件夹路径，语义与快捷键完全一致
- **最近路径**：最多保留 10 条，点一条即复制
- **开机自启**：一键勾选，写 LaunchAgent
- **复制即通知**：原生通知显示结果；通知失败不影响已写入的剪贴板
- **失败不动剪贴板**：取不到路径时保留原内容并提示，不会把旧值覆盖成错答案

1.1.5 起附带 **CopyPath Toolbar.app**：拖进访达工具栏后，点一下即复制当前文件夹路径
（语义与「立即获取文件夹路径」完全一致）。1.2.0 起自带**屏幕悬浮按钮**（默认开启）。

## 安装

本机打包直接产出到 **`~/Downloads/CopyPath_1.2.1_universal.dmg`**。双击挂载，把
**CopyPath.app 拖到 Applications** 即可（想用工具栏按钮就把 **CopyPath Toolbar.app**
也一并拖过去）。

本机生成的 dmg 不带隔离属性（`com.apple.quarantine`），所以双击就能打开，不需要额外的
Gatekeeper 步骤。只有当 dmg 经过浏览器下载或隔空投送转到别的机器时才可能被拦，那时用
**右键 → 打开 → 再点一次「打开」**过掉。

从源码打包（同时产出两个 `.app` 与 `.dmg`，并把两个 `.app` 装进 `~/Applications` 后启动主程序）：

```bash
git clone http://192.168.0.200/lil/tools-folderpath
cd tools-folderpath
./build-app.sh
```

产物必须装在内部盘的 `~/Applications`，不要留在外置盘：开机自启的 LaunchAgent 会把 `.app`
的绝对路径写死，开机时盘没挂载自启就静默失效。

关于签名：本项目不做开发者证书签名，也不做公证。但 `build-app.sh` 里那次
`codesign --sign -`（ad-hoc）**不能删** —— 它不是分发签名，而是运行前提，
Apple Silicon 上未签名的二进制会被内核直接 SIGKILL（实测 exit 137）。

### 卸载

1. 退出：托盘菜单 → 「退出 CopyPath」；工具栏按钮是一次性进程，无需退出。
2. 删除 `~/Applications/CopyPath.app` 与 `~/Applications/CopyPath Toolbar.app`。
3. 若开过自启：删除 `~/Library/LaunchAgents/CopyPath.plist`。
4. 数据残留（可选）：删除 `~/Library/Application Support/com.lilin.folderpath/`。
5. 已授予的自动化权限（可选）：

   ```bash
   tccutil reset AppleEvents com.lilin.folderpath
   tccutil reset AppleEvents com.lilin.folderpath.toolbar
   ```

访达工具栏里的按钮本身：按住 <kbd>⌘</kbd> 把它从工具栏拖出来即可移除。

## 用法

| 触发 | 行为 |
| --- | --- |
| 屏幕悬浮按钮（左键） | 复制选中项自身路径 |
| 屏幕悬浮按钮（右键） | 两种语义切换 / 隐藏按钮 |
| <kbd>⌃</kbd><kbd>⌥</kbd><kbd>⌘</kbd><kbd>P</kbd> | 复制「所在文件夹」路径 |
| 菜单栏图标（左键或右键） | 展开菜单 |
| 访达工具栏按钮（CopyPath Toolbar） | 与快捷键相同，见下 |

### 屏幕悬浮按钮

1.2.0 起默认显示在主屏幕右缘居中。它是挂了 `NonactivatingPanel` 掩码的原生 NSPanel：
**点击不激活 CopyPath，Finder 保持前置**，读到的选中项语义与托盘菜单完全一致，结果照常
进「最近路径」历史并弹通知。

- **左键**：复制选中项自身路径（与托盘「复制选中项自身路径」同一条代码路径）
- **× 按钮**：胶囊右侧常驻关闭钮，点一下即隐藏（1.2.1 起提供，等同右键「隐藏悬浮按钮」）
- **按住拖动**：移动位置，松手即记忆（外接屏拔掉后还原不出位置时，自动回到默认位置）
- **右键**：小菜单——两种复制语义切换、「隐藏悬浮按钮」
- **显隐开关**：托盘菜单「悬浮按钮」勾选项

开关状态与位置存在 `float-button.json`（见「数据位置」）。按钮是常驻窗口而非一次性
进程，与工具栏 helper（点完即退、不写历史）不同，悬浮按钮写历史、随主程序存亡。

### 访达工具栏按钮

1. 装好 `~/Applications/CopyPath Toolbar.app`（`build-app.sh` 自动完成）。
2. 在访达里打开它所在的文件夹，选中它，按住 <kbd>⌘</kbd> 拖到访达工具栏。

之后在任何访达窗口点该按钮：helper 启动 → 复制当前文件夹路径 → 立即退出，每次点击都是
全新进程，与主程序是否运行无关。它不写「最近路径」历史（那是托盘应用的运行时状态）。
详细安装步骤与常见问题见[安装指南](docs/核心功能/访达工具栏按钮-安装指南.md)，
实现原理见[技术方案](docs/核心功能/访达工具栏按钮-实现技术方案.md)。

菜单项：

- **立即获取文件夹路径** —— 与快捷键相同
- **复制选中项自身路径** —— 选中什么复制什么，不做归一化
- **最近路径** —— 最多 10 条，点一条即复制；底部「清空历史」
- **悬浮按钮** —— 显隐开关（默认开）
- **开机自启**
- **退出 CopyPath**
- **版本 1.2.1** —— 只读，取自 `tauri.conf.json`，不写死在代码里

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

读访达需要自动化权限，第一次触发时系统会弹窗，允许即可。**CopyPath Toolbar 是独立的
bundle id，需要单独授权一次**（首次点工具栏按钮时弹窗）：

**系统设置 → 隐私与安全性 → 自动化 → CopyPath / CopyPath Toolbar → 打开「访达」**

## 数据位置

| 用途 | 路径 |
| --- | --- |
| 最近路径 | `~/Library/Application Support/com.lilin.folderpath/history.json` |
| 悬浮按钮开关与位置 | 同目录 `float-button.json` |
| 单实例锁 | 同目录 `instance.lock` |
| 开机自启 | `~/Library/LaunchAgents/CopyPath.plist` |

CopyPath Toolbar.app 不写任何数据：一次性进程，只读访达 → 写剪贴板 → 退出。

bundle identifier 仍是 `com.lilin.folderpath`（应用早期叫 FolderPath，改名时特意保留），
所以数据目录和已授权的自动化权限都跟着原名走，不必重新授权。

## 项目结构

```
build-app.sh                  # 唯一打包入口：编译 → 组装两个 .app → 签名 → dmg → 安装
assets/                       # 图标母图与生成脚本
dist/index.html               # 占位页（本项目无真实前端）
src-tauri/
├── tauri.conf.json           # 版本号单一真源
├── Info.plist                # 主程序 bundle
├── Info.toolbar.plist        # 工具栏 helper bundle
└── src/
    ├── main.rs               # 托盘应用入口（bin: copypath）
    ├── toolbar_main.rs       # 工具栏 helper 入口（bin: copypath-toolbar）
    ├── lib.rs                # 共享：快捷键注册、复制+通知、两种模式分发
    ├── float_button.rs       # 屏幕悬浮按钮：非激活 NSPanel、拖动记忆、右键菜单
    ├── finder.rs             # osascript 解析窗口/选中项，路径归一化与去重
    ├── clipboard.rs          # 剪贴板写入
    ├── notify.rs             # 原生通知（含阻塞版 show_and_wait，供一次性进程用）
    ├── history.rs            # 最近路径落盘，上限 HISTORY_MAX = 10
    └── tray.rs               # 托盘菜单
```

一个 crate 产出两个二进制，共用 `copypath_lib`；`build-app.sh` 把它们分别装进
两个 `.app` bundle（bundle id 不同，授权相互独立）。

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
tccutil reset AppleEvents com.lilin.folderpath.toolbar   # 工具栏按钮是独立 bundle id
```

**工具栏按钮点了没反应**
按钮绑定的是**拖入时**的 `.app` 路径。重新打包后旧路径失效（被移走或替换掉了），
把 `~/Applications/CopyPath Toolbar.app` 重新 <kbd>⌘</kbd> 拖入一次即可。
另一种可能：首次点击的自动化授权弹窗还没允许。

**「没有打开的访达窗口」**
脚本刻意不使用 `insertion location`（0 窗口时它会静默返回桌面，是个错答案），
所以没有窗口就是失败，不会动剪贴板。

**快捷键没反应**
可能被别的应用占用了。注册失败只在 stderr 打日志并降级，托盘菜单仍可用。
换键改 `src-tauri/src/lib.rs` 的 `SHORTCUT` 常量后重新打包。

**托盘图标不见了 / 起了两个**
`pgrep -x copypath` 看进程；重复启动会被单实例锁挡掉并打印提示。

**悬浮按钮不见了 / 想关掉**
托盘菜单 → 「悬浮按钮」勾选即可显隐。状态与位置存在
`~/Library/Application Support/com.lilin.folderpath/float-button.json`，
删掉该文件重启，回到默认（显示、右缘居中）。拖出屏幕后不会自动找回来：还原不出
位置（如拔掉外接屏）时启动会回落默认位置，但运行中拖到屏幕外就还在屏幕外，用开关
关掉再开即可复位。

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
  「左键一键复制」和「右键出菜单」在同一个图标上不可兼得，一键交给了全局快捷键与悬浮按钮。
- 悬浮按钮是常驻窗口：占用一小块屏幕，需要整块屏幕时用托盘菜单或右键菜单隐藏。
  它没有独立快捷键，显隐只有托盘开关一个入口。
- 工具栏按钮只覆盖「立即获取文件夹路径」一种语义，没有「复制选中项自身路径」入口，
  也不写最近路径历史。
- 工具栏按钮绑定拖入时的 `.app` 路径，重装/挪动后需重新拖入。
- 选中 `.framework`、`.bundle` 这类包目录时，访达报的类别是 `folder`，会复制包自身路径。
- 文件名里含换行的条目会被丢弃并在 stderr 告警，而不是拼进上一条路径。
- 全局快捷键是编译期常量，没有设置界面。

## 文档

- [访达工具栏按钮 · 安装指南](docs/核心功能/访达工具栏按钮-安装指南.md) —— 拖入、首次授权、验证方法、FAQ
- [访达工具栏按钮 · 实现技术方案](docs/核心功能/访达工具栏按钮-实现技术方案.md) —— 架构、时序、关键技术点
- [推广落地页](docs/推广/index.html) —— 单文件静态网页，用户视角的产品介绍
- [推广文档撰写指南](docs/推广/推广文档撰写指南.md) —— 推广页怎么写：视角、结构、事实纪律与验证流程
- [发布记录](docs/发布/README.md) —— 各版本变更记录（v1.2.0、v1.1.5、v1.0.0）
- [docs/README.md](docs/README.md) —— 文档中心索引

本 README 描述当前状态，历史交付记录不写在这里。
