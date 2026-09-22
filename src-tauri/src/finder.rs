//! 通过 `osascript` 读取访达（Finder）当前窗口与选中项，解析出绝对路径。
//!
//! 脚本每行输出一条 `kflag \t 选中项路径 \t 所在容器路径`，两种复制模式共用一次调用。

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// 脚本主动判定「无访达窗口」时抛出的哨兵串，靠它把该情形与普通失败区分开。
pub const NO_WINDOW_MARK: &str = "__FOLDERPATH_NO_WINDOW__";

/// 变量名刻意避开 AppleScript 保留字：`kind`、`ip`、`ref` 等做赋值目标会报 -10003/-10006。
/// 脚本不含 `activate`，否则会抢前台焦点让菜单栏应用闪一下。
pub const SCRIPT: &str = r#"tell application "Finder"
    if (count of Finder windows) is 0 then
        error "__FOLDERPATH_NO_WINDOW__" number -2700
    end if
    set selItems to selection
    set resText to ""
    repeat with anItem in selItems
        try
            set curObj to contents of anItem
            set kflag to "F"
            if (class of curObj) is folder then set kflag to "D"
            set itemP to POSIX path of (curObj as alias)
            set contP to itemP
            try
                set contP to POSIX path of ((container of curObj) as alias)
            end try
            set resText to resText & kflag & tab & itemP & tab & contP & linefeed
        end try
    end repeat
    if resText is "" then
        set winP to POSIX path of ((target of front Finder window) as alias)
        return "W" & tab & winP & tab & winP & linefeed
    end if
    return resText
end tell"#;

/// 两个托盘入口对应两种语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// 选中文件夹取自身，选中文件取其所在目录，无选中项取前置窗口目录。
    FolderPath,
    /// 一律取选中项自身路径。
    ItemPath,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinderError {
    NoWindow,
    Failed(String),
}

impl std::fmt::Display for FinderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoWindow => f.write_str("没有打开的访达窗口，请先在访达中打开一个文件夹"),
            Self::Failed(m) => write!(f, "读取访达路径失败：{m}"),
        }
    }
}

struct Record {
    kind: char,
    item_path: String,
    container_path: String,
}

fn strip_trailing_slash(raw: &str) -> PathBuf {
    let mut s = raw.trim_end_matches('/');
    if s.is_empty() {
        s = "/";
    }
    PathBuf::from(s)
}

/// 纯函数，便于用真机抓到的样本做单测（不依赖活的访达）。
///
/// 逐行严格解析：每行必须是 `kflag \t 选中项路径 \t 容器路径`。字段数不对的行
/// （含 osascript 在末尾补的那颗空行）直接丢弃并告警——名字里带换行的文件会让
/// 该条记录断成两行，这里选择丢掉而不是拼回，以免把上一条路径污染成带 `\n` 的假路径。
pub fn parse(stdout: &str, mode: Mode) -> Result<Vec<PathBuf>, FinderError> {
    let mut records: Vec<Record> = Vec::new();
    for line in stdout.lines() {
        let cols: Vec<&str> = line.splitn(3, '\t').collect();
        match (cols.len(), cols.first().and_then(|c| c.chars().next())) {
            (3, Some(kind @ ('D' | 'F' | 'W'))) => records.push(Record {
                kind,
                item_path: cols[1].to_string(),
                container_path: cols[2].to_string(),
            }),
            // 末尾空行是 osascript 的固定输出，不值得告警。
            _ if line.is_empty() => {}
            _ => eprintln!("忽略无法解析的一行：{line:?}"),
        }
    }

    if records.is_empty() {
        return Err(FinderError::Failed("访达没有返回任何路径".into()));
    }

    let mut out: Vec<PathBuf> = Vec::new();
    match mode {
        Mode::ItemPath => {
            for r in &records {
                out.push(strip_trailing_slash(&r.item_path));
            }
        }
        Mode::FolderPath => {
            for r in &records {
                // W 记录两列相同，取哪列都一样。
                let raw = if r.kind == 'D' || r.kind == 'W' { &r.item_path } else { &r.container_path };
                let p = strip_trailing_slash(raw);
                // 同目录下的多个文件归一到一条，保持首次出现顺序。
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    Ok(out)
}

pub fn resolve(mode: Mode) -> Result<Vec<PathBuf>, FinderError> {
    // .output() 会同时抽干 stdout/stderr，不会像 spawn()+wait() 那样在管道写满时死锁。
    // 写死绝对路径：开机自启时由 LaunchAgent 拉起，$PATH 未必含 /usr/bin。
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(SCRIPT)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| FinderError::Failed(format!("无法执行 osascript：{e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if stderr.contains(NO_WINDOW_MARK) {
            return Err(FinderError::NoWindow);
        }
        // osascript 的错误形如 "77:103: execution error: … (-2700)"。
        let msg = stderr
            .lines()
            .rev()
            .find(|l| !l.trim().is_empty())
            .unwrap_or("未知错误")
            .trim()
            .to_string();
        return Err(FinderError::Failed(msg));
    }

    parse(&String::from_utf8_lossy(&out.stdout), mode)
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真机抓取：一个窗口、选中单个文件
    const ONE_FILE: &str = "F\t/Users/lilin/Downloads/iShot.png\t/Users/lilin/Downloads/\n";
    // 真机抓取：一个窗口、无选中项
    const WINDOW_ONLY: &str = "W\t/Users/lilin/Downloads/\t/Users/lilin/Downloads/\n";
    // 同构样本：三个同目录文件
    const THREE_FILES: &str = "F\t/V/a.txt\t/V/\nF\t/V/b.txt\t/V/\nF\t/V/c.txt\t/V/\n";
    // 同构样本：一个文件夹 + 一个文件
    const FOLDER_AND_FILE: &str =
        "D\t/V/src-tauri/\t/V/\nF\t/V/a.txt\t/V/\n";

    #[test]
    fn parses_window_fallback_record() {
        let got = parse(WINDOW_ONLY, Mode::FolderPath).unwrap();
        assert_eq!(got, vec![PathBuf::from("/Users/lilin/Downloads")]);
    }

    #[test]
    fn folder_mode_maps_file_to_its_container() {
        let got = parse(ONE_FILE, Mode::FolderPath).unwrap();
        assert_eq!(got, vec![PathBuf::from("/Users/lilin/Downloads")]);
    }

    #[test]
    fn item_mode_keeps_the_file_itself() {
        let got = parse(ONE_FILE, Mode::ItemPath).unwrap();
        assert_eq!(got, vec![PathBuf::from("/Users/lilin/Downloads/iShot.png")]);
    }

    #[test]
    fn folder_mode_dedupes_files_of_same_folder() {
        let got = parse(THREE_FILES, Mode::FolderPath).unwrap();
        assert_eq!(got, vec![PathBuf::from("/V")]);
        let items = parse(THREE_FILES, Mode::ItemPath).unwrap();
        assert_eq!(items.len(), 3);
    }

    #[test]
    fn selected_folder_is_not_promoted_to_its_parent() {
        let got = parse(FOLDER_AND_FILE, Mode::FolderPath).unwrap();
        assert_eq!(
            got,
            vec![PathBuf::from("/V/src-tauri"), PathBuf::from("/V")]
        );
    }

    #[test]
    fn volume_root_keeps_a_single_slash() {
        let got = parse("W\t/\t/\n", Mode::FolderPath).unwrap();
        assert_eq!(got, vec![PathBuf::from("/")]);
    }

    #[test]
    fn ignores_the_extra_trailing_newline_osascript_adds() {
        // 真机 `od -c` 实测：除脚本自己的 linefeed 外，osascript 还会再补一个换行。
        // 少了这条，路径尾部会多拼出一个 '\n'。
        let raw = "F\t/Users/lilin/Downloads/a.png\t/Users/lilin/Downloads/\n\n";
        assert_eq!(
            parse(raw, Mode::ItemPath).unwrap(),
            vec![PathBuf::from("/Users/lilin/Downloads/a.png")]
        );
        assert_eq!(
            parse(raw, Mode::FolderPath).unwrap(),
            vec![PathBuf::from("/Users/lilin/Downloads")]
        );
    }

    #[test]
    fn names_with_newlines_are_dropped_not_spliced_into_the_previous_path() {
        // 已知取舍：这类条目会被丢弃（并在 stderr 告警），而不是产出一条带 '\n' 的假路径。
        assert!(matches!(
            parse("F\t/V/a\nb\t/V/\n\n", Mode::ItemPath),
            Err(FinderError::Failed(_))
        ));
    }

    #[test]
    fn empty_output_is_a_failure_not_an_empty_success() {
        assert!(matches!(parse("", Mode::FolderPath), Err(FinderError::Failed(_))));
    }
}
