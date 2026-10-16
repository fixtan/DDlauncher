// アイコン取得とディスクキャッシュ。
//
// 取り方（Windows）
//   - .lnk: 中身を読み（IShellLink）、アイコンの指定があればそのファイルから、なければ指す先
//     （exe など）から取る。.lnk そのものから取ると、ショートカットの矢印が付くため。
//   - exe・フォルダ・書類・ストアアプリ: IShellItemImageFactory で、要求サイズに近い画像を取る
//   - 上の方法が失敗したときだけ、従来の SHGetFileInfoW（32x32。.lnk だと矢印付き）
// 取った PNG は %LOCALAPPDATA%\com.fixtan.ddlauncher\icons\<キー>.png に保存し、次回からはそれを読む。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::{fs, path::Path, time::UNIX_EPOCH};

// 抽出方法を変えたときは、これを上げると古いキャッシュが使われなくなる
const CACHE_VERSION: &str = "v2";

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// パス + 更新日時 + ファイルサイズ + 要求サイズ から、キャッシュのファイル名を作る
fn cache_key(path: &Path, size: u32) -> String {
    let meta = fs::metadata(path).ok();
    let mtime = meta
        .as_ref()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let len = meta.map(|m| m.len()).unwrap_or(0);
    let s = format!(
        "{CACHE_VERSION}|{}|{mtime}|{len}|{size}",
        path.to_string_lossy().to_lowercase()
    );
    format!("{:016x}", fnv1a(s.as_bytes()))
}

// %NAME% を環境変数で置き換える（見つからないものはそのまま）
fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find('%') {
            Some(j) if j > 0 => match std::env::var(&after[..j]) {
                Ok(v) => {
                    out.push_str(&v);
                    rest = &after[j + 1..];
                }
                Err(_) => {
                    out.push('%');
                    rest = after;
                }
            },
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

// 画面にそのまま渡せる data URL を返す。キャッシュにあればそれを使う。size は表示したい大きさ（px）。
pub fn icon_data_url(cache_dir: &Path, path: &Path, size: u32) -> Result<String, String> {
    let size = size.clamp(16, 256);
    let file = cache_dir.join(format!("{}.png", cache_key(path, size)));
    let png = match fs::read(&file) {
        Ok(b) if !b.is_empty() => b,
        _ => {
            let b = extract_png(path, size)?;
            let _ = fs::create_dir_all(cache_dir);
            let tmp = file.with_extension("tmp");
            if fs::write(&tmp, &b).is_ok() {
                let _ = fs::rename(&tmp, &file);
            }
            b
        }
    };
    Ok(format!("data:image/png;base64,{}", STANDARD.encode(png)))
}

#[cfg(not(windows))]
fn extract_png(_path: &Path, _size: u32) -> Result<Vec<u8>, String> {
    Err("アイコン取得は Windows のみ".to_string())
}

#[cfg(windows)]
fn extract_png(path: &Path, size: u32) -> Result<Vec<u8>, String> {
    imp::extract(path, size as i32)
}

// Windows 実装
#[cfg(windows)]
mod imp {
    use std::path::{Path, PathBuf};
    use windows::core::{IUnknown, Interface, PCWSTR};
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::{
        DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO,
        BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HGDIOBJ,
    };
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, IBindCtx, IPersistFile,
        CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, STGM_READ,
    };
    use windows::Win32::UI::Shell::{
        IShellItemImageFactory, IShellLinkW, SHCreateItemFromParsingName, SHGetFileInfoW,
        ShellLink, SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
        SLGP_RAWPATH,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        DestroyIcon, GetIconInfo, PrivateExtractIconsW, HICON, ICONINFO,
    };

    // ワーカースレッドで呼ばれるので、そのスレッドで COM を初期化する
    pub fn extract(path: &Path, size: i32) -> Result<Vec<u8>, String> {
        unsafe {
            let inited = CoInitializeEx(None, COINIT_APARTMENTTHREADED).is_ok();
            let r = extract_sized(path, size);
            if inited {
                CoUninitialize();
            }
            r
        }
    }

    unsafe fn extract_sized(path: &Path, size: i32) -> Result<Vec<u8>, String> {
        let s = path.to_string_lossy().into_owned();
        // ストアアプリ（shell:AppsFolder\...）は、シェルの項目として画像を取る
        if s.starts_with(crate::appsfolder::PREFIX) {
            return shell_item_png(&s, size);
        }
        if !path.exists() {
            return Err("ファイル（フォルダ）が存在しない".to_string());
        }
        // どの方法がなぜ失敗したかを、最後にまとめて返す（ログで原因を追えるように）
        let mut errs: Vec<String> = Vec::new();
        let mut target: PathBuf = path.to_path_buf();
        let is_lnk = has_ext(path, &["lnk"]);
        if is_lnk {
            match resolve_lnk(path) {
                Some(info) => {
                    // ショートカット自身にアイコンの指定があれば、それを優先する
                    if let Some(f) = &info.icon_file {
                        match icon_from_file(f, info.icon_index, size) {
                            Ok(png) => return Ok(png),
                            Err(e) => errs.push(format!("lnkのアイコン指定 {}: {e}", f.display())),
                        }
                    }
                    if let Some(t) = info.target {
                        if t.exists() {
                            target = t;
                        } else {
                            errs.push(format!("lnkの指す先が存在しない: {}", t.display()));
                        }
                    }
                }
                None => errs.push("lnkを読めなかった".to_string()),
            }
        }
        match shell_item_png(&target.to_string_lossy(), size) {
            Ok(png) => return Ok(png),
            Err(e) => errs.push(format!("シェルの画像 {}: {e}", target.display())),
        }
        // exe・dll・ico は、ファイル自身のアイコン資源から直接取る
        if has_ext(&target, &["exe", "dll", "ico", "icl"]) {
            match icon_from_file(&target, 0, size) {
                Ok(png) => return Ok(png),
                Err(e) => errs.push(format!("アイコン資源: {e}")),
            }
        }
        // 最後の手段（32x32。.lnk の場合は矢印が付く）
        match extract_inner(path) {
            Ok(png) => Ok(png),
            Err(e) => {
                errs.push(format!("従来の方法: {e}"));
                Err(errs.join(" / "))
            }
        }
    }

    fn has_ext(p: &Path, exts: &[&str]) -> bool {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
    }

    struct LnkInfo {
        icon_file: Option<PathBuf>,
        icon_index: i32,
        target: Option<PathBuf>,
    }

    fn from_wide(buf: &[u16]) -> String {
        let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        String::from_utf16_lossy(&buf[..n])
    }

    // .lnk の中身（アイコンの指定と、指す先）を読む
    unsafe fn resolve_lnk(path: &Path) -> Option<LnkInfo> {
        let link: IShellLinkW =
            CoCreateInstance(&ShellLink, None::<&IUnknown>, CLSCTX_INPROC_SERVER).ok()?;
        let pf: IPersistFile = link.cast().ok()?;
        let wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        pf.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;

        let mut ibuf = [0u16; 260];
        let mut idx = 0i32;
        let icon_file = if link.GetIconLocation(&mut ibuf, &mut idx).is_ok() {
            let f = from_wide(&ibuf);
            if f.is_empty() {
                None
            } else {
                Some(PathBuf::from(super::expand_env(&f)))
            }
        } else {
            None
        };

        let mut tbuf = [0u16; 260];
        let target = if link
            .GetPath(&mut tbuf, std::ptr::null_mut(), SLGP_RAWPATH.0 as u32)
            .is_ok()
        {
            let t = from_wide(&tbuf);
            if t.is_empty() {
                None
            } else {
                Some(PathBuf::from(super::expand_env(&t)))
            }
        } else {
            None
        };
        Some(LnkInfo {
            icon_file,
            icon_index: idx,
            target,
        })
    }

    // exe・dll・ico などの指定番号のアイコンを、要求サイズで取る
    unsafe fn icon_from_file(file: &Path, index: i32, size: i32) -> Result<Vec<u8>, String> {
        let mut wbuf = [0u16; 260];
        for (i, c) in file.to_string_lossy().encode_utf16().take(259).enumerate() {
            wbuf[i] = c;
        }
        let mut icons = [HICON(std::ptr::null_mut())];
        let n = PrivateExtractIconsW(&wbuf, index, size, size, Some(&mut icons[..]), None, 0);
        if n == 0 || n == u32::MAX || icons[0].is_invalid() {
            return Err("PrivateExtractIconsW がアイコンを返さなかった".to_string());
        }
        hicon_to_png(icons[0])
    }

    // 従来の方法: SHGetFileInfoW（32x32）
    unsafe fn extract_inner(path: &Path) -> Result<Vec<u8>, String> {
        let wide: Vec<u16> = path
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut sfi = SHFILEINFOW::default();
        let r = SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(std::ptr::addr_of_mut!(sfi)),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON | SHGFI_LARGEICON,
        );
        if r == 0 || sfi.hIcon.is_invalid() {
            return Err("SHGetFileInfoW がアイコンを返さなかった".to_string());
        }
        hicon_to_png(sfi.hIcon)
    }

    // HICON を PNG にする。HICON は、ここで破棄する
    unsafe fn hicon_to_png(hicon: HICON) -> Result<Vec<u8>, String> {
        let mut info = ICONINFO::default();
        if GetIconInfo(hicon, &mut info).is_err() {
            let _ = DestroyIcon(hicon);
            return Err("GetIconInfo に失敗".to_string());
        }

        let result = bitmaps_to_png(info.hbmColor, info.hbmMask);

        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmColor.0));
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(HGDIOBJ(info.hbmMask.0));
        }
        let _ = DestroyIcon(hicon);
        result
    }

    // 指定ビットマップを 32bit の上から下の BGRA で読む
    unsafe fn read_bgra(hbm: HBITMAP, w: i32, h: i32) -> Result<Vec<u8>, String> {
        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // マイナス = 上から下
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let hdc = GetDC(None);
        let lines = GetDIBits(
            hdc,
            hbm,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        ReleaseDC(None, hdc);
        if lines == 0 {
            return Err("GetDIBits に失敗".to_string());
        }
        Ok(buf)
    }

    // シェルの項目（ファイル・フォルダ・shell:AppsFolder\...）の画像を、要求サイズ前後で取る
    unsafe fn shell_item_png(item: &str, size: i32) -> Result<Vec<u8>, String> {
        let wide: Vec<u16> = item.encode_utf16().chain(std::iter::once(0)).collect();
        let f: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None::<&IBindCtx>)
                .map_err(|e| format!("SHCreateItemFromParsingName: {e}"))?;
        let hbm = f
            .GetImage(
                SIZE { cx: size, cy: size },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .map_err(|e| format!("GetImage: {e}"))?;
        let r = shell_bitmap_to_png(hbm);
        let _ = DeleteObject(HGDIOBJ(hbm.0));
        r
    }

    unsafe fn shell_bitmap_to_png(hbm: HBITMAP) -> Result<Vec<u8>, String> {
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(hbm.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut _),
        );
        if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return Err("GetObjectW に失敗".to_string());
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut px = read_bgra(hbm, w, h)?;
        // 乗算済みアルファを元に戻し、BGRA -> RGBA にする
        for p in px.chunks_exact_mut(4) {
            let a = p[3] as u32;
            if a > 0 && a < 255 {
                for c in &mut p[..3] {
                    *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
                }
            }
            p.swap(0, 2);
        }
        encode_rgba(&px, w as u32, h as u32)
    }

    fn encode_rgba(px: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, w, h);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut wr = enc.write_header().map_err(|e| e.to_string())?;
            wr.write_image_data(px).map_err(|e| e.to_string())?;
        }
        Ok(out)
    }

    unsafe fn bitmaps_to_png(color: HBITMAP, mask: HBITMAP) -> Result<Vec<u8>, String> {
        if color.is_invalid() {
            // 白黒アイコン（色ビットマップがない）は対応しない
            return Err("色ビットマップがない（白黒アイコン）".to_string());
        }
        let mut bm = BITMAP::default();
        let got = GetObjectW(
            HGDIOBJ(color.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut BITMAP as *mut _),
        );
        if got == 0 || bm.bmWidth <= 0 || bm.bmHeight <= 0 {
            return Err("GetObjectW に失敗".to_string());
        }
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        let mut px = read_bgra(color, w, h)?;

        // アルファが全部 0 の古い形式のアイコンは、マスク（0 = 不透明）から透明度を作る
        if px.chunks_exact(4).all(|p| p[3] == 0) {
            let m = if mask.is_invalid() {
                None
            } else {
                read_bgra(mask, w, h).ok()
            };
            for (i, p) in px.chunks_exact_mut(4).enumerate() {
                let opaque = match &m {
                    Some(m) => m[i * 4] == 0,
                    None => true,
                };
                p[3] = if opaque { 255 } else { 0 };
            }
        }

        // BGRA -> RGBA。GetDIBits の値は乗算済みではないものとして扱う
        for p in px.chunks_exact_mut(4) {
            p.swap(0, 2);
        }

        encode_rgba(&px, w as u32, h as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_and_differs_by_path_and_size() {
        let a = cache_key(Path::new("C:\\a\\x.lnk"), 32);
        assert_eq!(a, cache_key(Path::new("C:\\A\\X.lnk"), 32)); // 大文字小文字は区別しない
        assert_ne!(a, cache_key(Path::new("C:\\a\\y.lnk"), 32));
        assert_ne!(a, cache_key(Path::new("C:\\a\\x.lnk"), 96)); // サイズ違いは別のキャッシュ
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn expand_env_replaces_known_and_keeps_unknown() {
        #[allow(unused_unsafe)]
        unsafe {
            std::env::set_var("DDL_TEST_ROOT", "C:\\Win");
        }
        assert_eq!(
            expand_env("%DDL_TEST_ROOT%\\sys\\a.dll"),
            "C:\\Win\\sys\\a.dll"
        );
        assert_eq!(expand_env("%DDL_NOPE_XYZ%\\a"), "%DDL_NOPE_XYZ%\\a");
        assert_eq!(expand_env("100%"), "100%");
        assert_eq!(expand_env("a%%b"), "a%%b");
        assert_eq!(expand_env("plain"), "plain");
    }
}
