//! `CopyPath Toolbar.app` 入口：被拖进访达工具栏后，点击即复制当前文件夹路径。
//! 无窗口、无托盘、不抢焦点——Finder 在点击瞬间仍是前置应用，所以解析到的就是
//! 触发它的那个窗口。干完即退，每次点击都是一次全新进程。

fn main() {
    #[cfg(target_os = "macos")]
    copypath_lib::notify::set_bundle("com.lilin.folderpath.toolbar");
    std::process::exit(copypath_lib::toolbar_copy());
}
