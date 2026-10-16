// 起動履歴。「最近使ったアプリ」の元になる。path・起動回数・最後に起動した時刻（UNIX 秒）。
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    pub count: u32,
    pub last: u64,
}

const MAX_ENTRIES: usize = 200;

fn same(a: &str, b: &str) -> bool {
    a.to_lowercase() == b.to_lowercase()
}

// 履歴に 1 回分を足す。同じ path（大文字小文字は区別しない）は回数を増やして時刻を更新する。
pub fn record(mut list: Vec<Entry>, path: &str, now: u64) -> Vec<Entry> {
    // 空、または ms-settings:display や control.exe のようなコマンド（区切りなし）は履歴に入れない
    if path.trim().is_empty() || !(path.contains('\\') || path.contains('/')) {
        return list;
    }
    if let Some(e) = list.iter_mut().find(|e| same(&e.path, path)) {
        e.count = e.count.saturating_add(1);
        e.last = now;
    } else {
        list.push(Entry {
            path: path.to_string(),
            count: 1,
            last: now,
        });
    }
    // 増えすぎたら古いものから捨てる
    if list.len() > MAX_ENTRIES {
        list.sort_by(|a, b| b.last.cmp(&a.last));
        list.truncate(MAX_ENTRIES);
    }
    list
}

// 新しい順に n 件
pub fn recent(list: &[Entry], n: usize) -> Vec<Entry> {
    let mut v = list.to_vec();
    v.sort_by(|a, b| b.last.cmp(&a.last));
    v.truncate(n);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_counts_and_updates() {
        let l = record(vec![], "C:\\A.exe", 10);
        let l = record(l, "c:\\a.EXE", 20);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].count, 2);
        assert_eq!(l[0].last, 20);
    }

    #[test]
    fn empty_path_ignored() {
        assert!(record(vec![], "  ", 1).is_empty());
    }

    #[test]
    fn commands_ignored() {
        assert!(record(vec![], "ms-settings:display", 1).is_empty());
        assert!(record(vec![], "control.exe", 1).is_empty());
        assert_eq!(record(vec![], "shell:AppsFolder\\X!App", 1).len(), 1);
    }

    #[test]
    fn recent_is_newest_first_and_limited() {
        let mut l = vec![];
        for (i, p) in ["/a", "/b", "/c", "/d"].iter().enumerate() {
            l = record(l, p, 100 + i as u64);
        }
        l = record(l, "/a", 200);
        let r = recent(&l, 3);
        assert_eq!(
            r.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            ["/a", "/d", "/c"]
        );
    }

    #[test]
    fn capped() {
        let mut l = vec![];
        for i in 0..(MAX_ENTRIES + 20) {
            l = record(l, &format!("/p{i}"), i as u64);
        }
        assert_eq!(l.len(), MAX_ENTRIES);
        assert!(l
            .iter()
            .any(|e| e.path == format!("/p{}", MAX_ENTRIES + 19)));
        assert!(!l.iter().any(|e| e.path == "/p0"));
    }
}
