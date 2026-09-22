#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::fs::{File, OpenOptions};
use std::io;
use std::path::PathBuf;

use fs2::FileExt;

/// 单实例：菜单栏应用重复启动会出现两个托盘图标，直接用文件锁挡掉。
fn acquire_single_instance() -> io::Result<File> {
    let dir = config_dir()?;
    std::fs::create_dir_all(&dir)?;
    let lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(dir.join("instance.lock"))?;
    lock.try_lock_exclusive()
        .map_err(|e| io::Error::new(io::ErrorKind::AlreadyExists, e))?;
    Ok(lock)
}

fn config_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other("环境变量 HOME 缺失"))?;
    Ok(home.join("Library/Application Support/com.lilin.folderpath"))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `--probe [folder|item]` 只打印解析结果就退出，便于在没有窗口的托盘应用上做验证。
    if args.first().map(String::as_str) == Some("--probe") {
        std::process::exit(copypath_lib::probe(args.get(1).map(String::as_str)));
    }

    match acquire_single_instance() {
        // 锁必须活到进程结束，否则第二个实例就能抢到。
        Ok(lock) => std::mem::forget(lock),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
            eprintln!("CopyPath 已经在运行，不再启动第二个实例");
            std::process::exit(0);
        }
        Err(e) => eprintln!("单实例锁不可用（{e}），继续启动"),
    }

    copypath_lib::run();
}
