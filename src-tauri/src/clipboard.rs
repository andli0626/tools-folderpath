/// 写入纯文本剪贴板。
pub fn write_text(text: &str) -> Result<(), String> {
    // arboard 的 macOS 实现不是 Send，每次新建反而最省事（一次 OS 调用量级）。
    let mut clipboard = arboard::Clipboard::new().map_err(|e| format!("无法访问剪贴板：{e}"))?;
    clipboard
        .set_text(text.to_string())
        .map_err(|e| format!("写入剪贴板失败：{e}"))
}
