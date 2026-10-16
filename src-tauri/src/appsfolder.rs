// Windows の「アプリ」フォルダ（shell:AppsFolder）から、.lnk のないアプリ（ストアアプリ）を足す。
// あわせて、フォルダ名・項目名を Windows が表示している名前（日本語など）に直す。
//
// 「アプリ」フォルダは階層のない一覧なので、7-Zip や Accessories のようなフォルダ構造は
// 引き続き .lnk の走査（startmenu.rs）から作る。ここで足すのは、ストアアプリだけ。

use crate::startmenu::AppNode;

// 起動・アイコン取得のとき、ストアアプリのパスにはこの接頭辞を付ける
pub const PREFIX: &str = "shell:AppsFolder\\";

// ストアアプリの識別子は「パッケージ名!アプリID」の形。パスや GUID ではないものだけを通す。
fn is_store_id(id: &str) -> bool {
    id.contains('!') && !id.contains('\\') && !id.contains('/') && !id.starts_with('{')
}

// 一覧から、ストアアプリだけを AppNode にする。既に同名の項目があれば足さない。
pub fn store_nodes(raw: Vec<(String, String)>, existing: &[AppNode]) -> Vec<AppNode> {
    let mut out: Vec<AppNode> = Vec::new();
    for (name, id) in raw {
        let name = name.trim().to_string();
        // 名前が解決できていない項目（"ms-resource:..." のまま）は出さない
        if name.is_empty() || name.to_lowercase().starts_with("ms-resource:") || !is_store_id(&id) {
            continue;
        }
        let key = name.to_lowercase();
        let dup = |n: &AppNode| n.children.is_none() && n.name.to_lowercase() == key;
        if existing.iter().any(dup) || out.iter().any(dup) {
            continue;
        }
        out.push(AppNode {
            name,
            path: format!("{PREFIX}{id}"),
            children: None,
        });
    }
    out
}

// (表示名, 識別子) の一覧
#[cfg(windows)]
pub fn enumerate() -> Result<Vec<(String, String)>, String> {
    imp::enumerate()
}
#[cfg(not(windows))]
pub fn enumerate() -> Result<Vec<(String, String)>, String> {
    Ok(Vec::new())
}

// 各ノードの名前を、Windows が表示している名前にする（取れなければそのまま）
#[cfg(windows)]
pub fn localize(nodes: &mut [AppNode]) {
    imp::localize(nodes)
}
#[cfg(not(windows))]
pub fn localize(_nodes: &mut [AppNode]) {}

#[cfg(windows)]
mod imp {
    use super::AppNode;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::{
        CoInitializeEx, CoTaskMemFree, CoUninitialize, IBindCtx, COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, SHGetFileInfoW,
        SHGetKnownFolderItem, KF_FLAG_DEFAULT, SHFILEINFOW, SHGFI_DISPLAYNAME, SIGDN,
        SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    };

    // COM を初期化して f を実行する（呼ばれるのはワーカースレッド）
    fn with_com<T>(f: impl FnOnce() -> T) -> T {
        unsafe {
            let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let r = f();
            if inited {
                CoUninitialize();
            }
            r
        }
    }

    unsafe fn name_of(item: &IShellItem, kind: SIGDN) -> Option<String> {
        let p = item.GetDisplayName(kind).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const core::ffi::c_void));
        s
    }

    pub fn enumerate() -> Result<Vec<(String, String)>, String> {
        with_com(|| unsafe {
            let folder: IShellItem =
                SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)
                    .map_err(|e| format!("SHGetKnownFolderItem: {e}"))?;
            let en: IEnumShellItems = folder
                .BindToHandler(None::<&IBindCtx>, &BHID_EnumItems)
                .map_err(|e| format!("BindToHandler: {e}"))?;
            let mut out = Vec::new();
            loop {
                let mut items: [Option<IShellItem>; 1] = [None];
                let mut fetched = 0u32;
                en.Next(&mut items, Some(&mut fetched as *mut u32))
                    .map_err(|e| format!("Next: {e}"))?;
                if fetched == 0 {
                    break;
                }
                let Some(item) = items[0].take() else { break };
                let name = name_of(&item, SIGDN_NORMALDISPLAY);
                let id = name_of(&item, SIGDN_PARENTRELATIVEPARSING);
                if let (Some(n), Some(i)) = (name, id) {
                    out.push((n, i));
                }
            }
            Ok(out)
        })
    }

    // Windows が表示している名前（日本語化されたフォルダ名など）を返す
    unsafe fn display_name(path: &str) -> Option<String> {
        let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(std::ptr::addr_of_mut!(sfi)),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        );
        if r == 0 {
            return None;
        }
        let len = sfi.szDisplayName.iter().position(|&c| c == 0).unwrap_or(0);
        let mut s = String::from_utf16_lossy(&sfi.szDisplayName[..len]);
        // 拡張子が付いて返ることがあるので外す
        for ext in [".lnk", ".url"] {
            let cut = s.len().saturating_sub(ext.len());
            if s.len() >= ext.len() && s.is_char_boundary(cut) && s[cut..].eq_ignore_ascii_case(ext)
            {
                s.truncate(cut);
            }
        }
        let s = s.trim().to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }

    pub fn localize(nodes: &mut [AppNode]) {
        with_com(|| localize_inner(nodes));
    }

    fn localize_inner(nodes: &mut [AppNode]) {
        for n in nodes.iter_mut() {
            if let Some(name) = unsafe { display_name(&n.path) } {
                n.name = name;
            }
            if let Some(c) = n.children.as_mut() {
                localize_inner(c);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_nodes_filters_and_dedupes() {
        let existing = vec![AppNode {
            name: "Notepad".into(),
            path: "x".into(),
            children: None,
        }];
        let raw = vec![
            (
                "Calculator".to_string(),
                "Microsoft.WindowsCalculator_8wekyb3d8bbwe!App".to_string(),
            ),
            (
                "Notepad".to_string(),
                "Microsoft.WindowsNotepad_8wekyb3d8bbwe!App".to_string(),
            ), // 既にある
            ("Chrome".to_string(), "Chrome".to_string()), // ! がない
            (
                "Tool".to_string(),
                "{7C5A40EF-A0FB-4BFC-874A-C0F2E0B9FA8E}\\Tool\\t.exe".to_string(),
            ), // パス
            ("".to_string(), "A.B_c!App".to_string()),    // 名前なし
            ("calculator".to_string(), "Other_x!App".to_string()),
            (
                "ms-resource:DisplayName".to_string(),
                "Pkg_x!App".to_string(),
            ), // 名前が未解決 // 同名
        ];
        let out = store_nodes(raw, &existing);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "Calculator");
        assert_eq!(
            out[0].path,
            "shell:AppsFolder\\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App"
        );
    }
}
