// 設定・タイル・左端・ブックマークを1つの JSON にまとめる／戻す。ファイルの読み書きと画面操作は main.rs 側。
use serde_json::{json, Value};

pub const KIND: &str = "ddlauncher-backup";
pub const VERSION: u64 = 1;

#[derive(Debug, Default, PartialEq)]
pub struct Parts {
    pub settings: Option<Value>,
    pub tiles: Option<Value>,
    pub rail: Option<Value>,
    pub bookmarks: Option<Value>,
}

fn parse_obj(label: &str, text: Option<&str>) -> Result<Option<Value>, String> {
    let Some(t) = text else { return Ok(None) };
    let v: Value =
        serde_json::from_str(t).map_err(|e| format!("{label} が JSON として読めない: {e}"))?;
    if !v.is_object() {
        return Err(format!("{label} の形が違う"));
    }
    Ok(Some(v))
}

// 現在のファイルの中身（文字列。無ければ None）から、バックアップの JSON を作る
pub fn build(
    app_version: &str,
    settings: Option<&str>,
    tiles: Option<&str>,
    rail: Option<&str>,
    bookmarks: Option<&str>,
) -> Result<String, String> {
    let mut m = serde_json::Map::new();
    m.insert("kind".into(), json!(KIND));
    m.insert("version".into(), json!(VERSION));
    m.insert("app".into(), json!(app_version));
    for (k, label, t) in [
        ("settings", "設定", settings),
        ("tiles", "タイル", tiles),
        ("rail", "左端", rail),
        ("bookmarks", "ブックマーク", bookmarks),
    ] {
        if let Some(v) = parse_obj(label, t)? {
            m.insert(k.into(), v);
        }
    }
    serde_json::to_string_pretty(&Value::Object(m)).map_err(|e| e.to_string())
}

// バックアップの JSON を読んで、中身を検証する。壊れていたら Err（何も書き換えない）
pub fn parse(text: &str) -> Result<Parts, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("JSON として読めない: {e}"))?;
    let obj = v.as_object().ok_or("バックアップの形ではない")?;
    if obj.get("kind").and_then(Value::as_str) != Some(KIND) {
        return Err("DDlauncher のバックアップではない".to_string());
    }
    match obj.get("version").and_then(Value::as_u64) {
        Some(n) if n <= VERSION => {}
        _ => return Err("対応していないバージョンのバックアップ".to_string()),
    }
    let get = |k: &str, label: &str| -> Result<Option<Value>, String> {
        match obj.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(x) if x.is_object() => Ok(Some(x.clone())),
            Some(_) => Err(format!("{label} の形が違う")),
        }
    };
    Ok(Parts {
        settings: get("settings", "設定")?,
        tiles: get("tiles", "タイル")?,
        rail: get("rail", "左端")?,
        bookmarks: get("bookmarks", "ブックマーク")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let s = build(
            "0.1.0",
            Some(r#"{"anchor":"center"}"#),
            Some(r#"{"cols":6,"tiles":[]}"#),
            None,
            Some(r#"{"nodes":[]}"#),
        )
        .unwrap();
        let p = parse(&s).unwrap();
        assert_eq!(p.settings.unwrap()["anchor"], "center");
        assert_eq!(p.tiles.unwrap()["cols"], 6);
        assert!(p.rail.is_none());
        assert!(p.bookmarks.is_some());
    }

    #[test]
    fn rejects_other_json() {
        assert!(parse(r#"{"anchor":"center"}"#).is_err());
        assert!(parse("[1,2]").is_err());
        assert!(parse("not json").is_err());
    }

    #[test]
    fn rejects_future_version() {
        assert!(parse(r#"{"kind":"ddlauncher-backup","version":99}"#).is_err());
        assert!(parse(r#"{"kind":"ddlauncher-backup"}"#).is_err());
    }

    #[test]
    fn rejects_bad_part() {
        assert!(parse(r#"{"kind":"ddlauncher-backup","version":1,"tiles":[1]}"#).is_err());
        assert!(build("0.1.0", Some("[1]"), None, None, None).is_err());
        assert!(build("0.1.0", Some("{oops"), None, None, None).is_err());
    }

    #[test]
    fn empty_parts_ok() {
        let p = parse(r#"{"kind":"ddlauncher-backup","version":1}"#).unwrap();
        assert_eq!(p, Parts::default());
    }
}
