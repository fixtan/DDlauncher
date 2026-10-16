// スタートメニューのプログラム一覧（.lnk / .url）を集める。
//
// 見る場所は2つ。同名のフォルダは1つにまとめて、同じフォルダ内で同名の項目は先に見つけたほう
// （ユーザー側）だけ残す。並べ替えは画面側で行う（日本語の並びを Intl.Collator に任せる）。

use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

#[derive(Serialize, Debug, PartialEq)]
pub struct AppNode {
    pub name: String,
    pub path: String, // 項目: .lnk のフルパス。フォルダ: 先に見つけたフォルダのパス（表示には使わない）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<AppNode>>, // Some ならフォルダ
}

// 既定の2か所。環境変数が無ければ（Windows 以外など）空になる。
pub fn default_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for (var, tail) in [
        ("APPDATA", r"Microsoft\Windows\Start Menu\Programs"),
        ("ProgramData", r"Microsoft\Windows\Start Menu\Programs"),
    ] {
        if let Some(base) = std::env::var_os(var) {
            let p = Path::new(&base).join(tail.replace('\\', std::path::MAIN_SEPARATOR_STR));
            if p.is_dir() {
                v.push(p);
            }
        }
    }
    v
}

fn is_shortcut(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("lnk") | Some("url")
    )
}

fn merge(into: &mut Vec<AppNode>, node: AppNode) {
    let key = node.name.to_lowercase();
    let is_folder = node.children.is_some();
    let same = into
        .iter_mut()
        .find(|n| n.children.is_some() == is_folder && n.name.to_lowercase() == key);
    match same {
        None => into.push(node),
        Some(existing) => {
            // フォルダ同士は中身を足し合わせる。項目同士は先に見つけたものを残す。
            if let (Some(a), Some(b)) = (existing.children.as_mut(), node.children) {
                for c in b {
                    merge(a, c);
                }
            }
        }
    }
}

fn scan_dir(dir: &Path) -> Vec<AppNode> {
    let mut out: Vec<AppNode> = Vec::new();
    let Ok(rd) = fs::read_dir(dir) else {
        return out;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        let Some(fname) = path.file_name().and_then(|n| n.to_str()).map(String::from) else {
            continue;
        };
        if path.is_dir() {
            let children = scan_dir(&path);
            if !children.is_empty() {
                merge(
                    &mut out,
                    AppNode {
                        name: fname,
                        path: path.to_string_lossy().into_owned(),
                        children: Some(children),
                    },
                );
            }
        } else if is_shortcut(&path) {
            let name = path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or(&fname)
                .to_string();
            merge(
                &mut out,
                AppNode {
                    name,
                    path: path.to_string_lossy().into_owned(),
                    children: None,
                },
            );
        }
    }
    out
}

pub fn scan(roots: &[PathBuf]) -> Vec<AppNode> {
    let mut out: Vec<AppNode> = Vec::new();
    for r in roots {
        for n in scan_dir(r) {
            merge(&mut out, n);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(p: &Path) {
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, b"x").unwrap();
    }

    #[test]
    fn scan_merges_roots_and_filters() {
        let base = std::env::temp_dir().join(format!("ddl-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let user = base.join("user");
        let common = base.join("common");
        touch(&user.join("Notepad.lnk"));
        touch(&user.join("Tools/A.lnk"));
        touch(&common.join("notepad.lnk")); // 同名（大文字小文字違い）→ 1つだけ
        touch(&common.join("Tools/B.lnk")); // 同名フォルダ → 中身を合わせる
        touch(&common.join("Tools/desktop.ini")); // 対象外
        touch(&common.join("readme.txt")); // 対象外
        touch(&common.join("Empty/desktop.ini")); // 空のフォルダは出さない
        touch(&common.join("日本語アプリ.url"));

        let tree = scan(&[user.clone(), common]);
        let mut names: Vec<_> = tree.iter().map(|n| n.name.clone()).collect();
        names.sort();
        assert_eq!(names, vec!["Notepad", "Tools", "日本語アプリ"]);
        let tools = tree.iter().find(|n| n.name == "Tools").unwrap();
        let mut kids: Vec<_> = tools
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|n| n.name.clone())
            .collect();
        kids.sort();
        assert_eq!(kids, vec!["A", "B"]);
        // 先に渡したルート（ユーザー側）のパスが残る
        let np = tree.iter().find(|n| n.name == "Notepad").unwrap();
        assert!(np.path.contains("user"));
        let _ = fs::remove_dir_all(&base);
    }
}
