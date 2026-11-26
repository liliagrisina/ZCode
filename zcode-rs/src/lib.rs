//! 编码代理核心。界面在 `ui`，并且只在 `gui` feature 下编译。

pub mod agent;
pub mod config;
pub mod extension;
pub mod session;
pub mod tools;

#[cfg(feature = "gui")]
pub mod ui;

#[cfg(test)]
mod testutil;

/// 按字符数截断。超出时保留 `max_chars - 1` 个字符并加省略号。
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out: String = input.chars().take(max_chars - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::truncate_chars;

    #[test]
    fn truncate_respects_chars() {
        assert_eq!(truncate_chars("你好世界", 3), "你好…");
        assert_eq!(truncate_chars("ab", 4), "ab");
        assert_eq!(truncate_chars("abcd", 0), "");
    }
}
