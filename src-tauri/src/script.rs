// ブックマークの「コマンド」を実行するための .bat の中身を作る。
// 複数行のコマンド（cd と npm run build など）を、そのまま1つのバッチとして新しいコンソールで走らせる。

// command: ユーザーが書いたコマンド（複数行可）。cwd: 作業フォルダ（空なら指定なし）。
pub fn build_script(command: &str, cwd: &str) -> String {
    let mut s = String::new();
    // 日本語のパスや出力が文字化けしないよう UTF-8 にする（画面には出さない）
    s.push_str("@chcp 65001 >nul\r\n");
    let cwd = cwd.trim();
    if !cwd.is_empty() {
        // " は Windows のパスには使えないので、囲むだけでよい
        s.push_str(&format!("cd /d \"{}\"\r\n", cwd.replace('"', "")));
    }
    for line in command.lines() {
        s.push_str(line.trim_end_matches('\r'));
        s.push_str("\r\n");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multi_line_with_cwd() {
        let s = build_script("cd C:\\a\r\nnpm run build\n", "C:\\Users\\x y\\proj");
        assert_eq!(
            s,
            "@chcp 65001 >nul\r\ncd /d \"C:\\Users\\x y\\proj\"\r\ncd C:\\a\r\nnpm run build\r\n"
        );
    }

    #[test]
    fn no_cwd() {
        assert_eq!(build_script("dir", "  "), "@chcp 65001 >nul\r\ndir\r\n");
    }

    #[test]
    fn quotes_removed_from_cwd() {
        assert!(build_script("x", "C:\\a\"b").contains("cd /d \"C:\\ab\""));
    }
}
