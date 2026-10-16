// リリースビルドでコンソール窓を出さない
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// DDlauncher 最小プロジェクト
//
// 確かめたいこと（docs/open-close-test.md に観察項目あり）
//   1. タスクバーのピン止めアイコンを押すたびに、メニューが開閉するか
//   2. 開いた状態でアイコンを押したとき、「閉じてすぐ開き直す」ちらつきが出ないか
//   3. 表示位置（左/中央/右 × 上/下）を切り替えられるか
//   4. 透過が効くか
//
// 仕組み
//   - ウィンドウはタスクバーに出さない(skipTaskbar)。だからピン止めアイコンを押すと、
//     exe がもう一度起動される。
//   - 2回目の起動は single-instance プラグインが 1回目(常駐側)に知らせて終了する。
//     常駐側はそれを合図に「開く/閉じる」を切り替える。
//   - フォーカスが外れたら閉じる。ただし、開いているメニューを閉じるためにアイコンを押すと、
//     「フォーカス喪失で閉じる → 直後に2回目の起動で開き直す」が起きる。
//     そこで「閉じてから guardMs 以内の2回目の起動は無視する」ガードを入れている。

mod appsfolder;
mod backup;
mod icons;
mod script;
mod startmenu;
mod usage;

use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{
    AppHandle, DragDropEvent, Emitter, Manager, PhysicalPosition, State, WebviewWindow, WindowEvent,
};

// ---------------------------------------------------------------- 設定

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "kebab-case")]
enum Anchor {
    BottomLeft,
    BottomCenter,
    BottomRight,
    TopLeft,
    TopCenter,
    TopRight,
    Center, // 画面（作業領域）の中央
    Cursor, // 押した場所（ピン止めアイコンの位置）の真上
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase", default)]
struct Settings {
    anchor: Anchor,
    hide_on_blur: bool,
    guard_ms: u64,
    margin: i32,
    opacity: f64,
    hotkey: String, // どこからでも開くキー（例: "Ctrl+Alt+Space"）。空なら使わない
    search_on_hotkey: bool, // ホットキーで開いたとき、検索欄から始める
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            anchor: Anchor::BottomLeft,
            hide_on_blur: true,
            guard_ms: 800,
            margin: 8,
            opacity: 0.92,
            hotkey: String::new(),
            search_on_hotkey: true,
        }
    }
}

fn load_settings(path: &PathBuf) -> Settings {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// 一時ファイルに書いてから置き換える（書き込み中に落ちても元のファイルが壊れない）。
// 置き換える前に、1世代だけ .bak に残す。
fn safe_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("dat")
        .to_string();
    let tmp = path.with_extension(format!("{ext}.tmp"));
    fs::write(&tmp, bytes)?;
    if path.exists() {
        let _ = fs::copy(path, path.with_extension(format!("{ext}.bak")));
    }
    fs::rename(&tmp, path)
}

fn save_settings(path: &Path, s: &Settings) -> std::io::Result<()> {
    safe_write(path, serde_json::to_string_pretty(s).unwrap().as_bytes())
}

// ---------------------------------------------------------------- 状態とログ

#[derive(Serialize, Clone)]
struct LogLine {
    t: u64, // epoch ミリ秒
    msg: String,
}

struct AppState {
    settings: Mutex<Settings>,
    last_hidden: Mutex<Option<Instant>>,
    last_drop: Mutex<Option<Instant>>, // 最後にファイルをドロップされた時刻
    last_shown: Mutex<Option<Instant>>, // 最後に表示した時刻（直後のフォーカス喪失を無視する）
    logs: Mutex<Vec<LogLine>>,
    config_path: PathBuf,
    tiles_path: PathBuf,
    usage_path: PathBuf,
    rail_path: PathBuf,
    bookmarks_path: PathBuf,
    busy: std::sync::atomic::AtomicBool, // ファイル選択ダイアログを出している間（フォーカス喪失で閉じない）
}

// ログは、コンソール・内部バッファ・画面（イベント）の3か所に出す。
// 画面が開く前のログも、バッファから取り出せる。
fn log(app: &AppHandle, msg: impl Into<String>) {
    let line = LogLine {
        t: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        msg: msg.into(),
    };
    println!("[ddl] {}", line.msg);
    if let Some(st) = app.try_state::<AppState>() {
        let mut logs = st.logs.lock().unwrap();
        logs.push(line.clone());
        if logs.len() > 300 {
            logs.remove(0);
        }
    }
    let _ = app.emit("ddl-log", line);
}

// ---------------------------------------------------------------- 位置

// カーソルがあるモニターの「作業領域」（タスクバーを除いた領域）を返す。(left, top, right, bottom) 物理ピクセル。
// ピン止めアイコンを押した直後はカーソルがタスクバー上にあるので、押したモニターが対象になる。
#[cfg(windows)]
fn work_area_at_cursor() -> Option<(i32, i32, i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        let mut mi = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(mon, &mut mi).as_bool() {
            Some((
                mi.rcWork.left,
                mi.rcWork.top,
                mi.rcWork.right,
                mi.rcWork.bottom,
            ))
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn work_area_at_cursor() -> Option<(i32, i32, i32, i32)> {
    None
}

// 現在のカーソル位置（物理ピクセル）。ピン止めアイコンを押した直後は、アイコンの上にある。
#[cfg(windows)]
fn cursor_pos() -> Option<(i32, i32)> {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;

    unsafe {
        let mut pt = POINT::default();
        GetCursorPos(&mut pt).ok()?;
        Some((pt.x, pt.y))
    }
}

#[cfg(not(windows))]
fn cursor_pos() -> Option<(i32, i32)> {
    None
}

fn place(win: &WebviewWindow, s: &Settings, app: &AppHandle) {
    let size = match win.outer_size() {
        Ok(v) => v,
        Err(_) => return,
    };
    let (w, h) = (size.width as i32, size.height as i32);
    match work_area_at_cursor() {
        Some((l, t, r, b)) => {
            let m = s.margin;
            let (cx, cy) = cursor_pos().unwrap_or(((l + r) / 2, b));
            let x = match s.anchor {
                Anchor::BottomLeft | Anchor::TopLeft => l + m,
                Anchor::BottomCenter | Anchor::TopCenter | Anchor::Center => l + ((r - l) - w) / 2,
                Anchor::BottomRight | Anchor::TopRight => r - w - m,
                // カーソルを横の中心にして、画面の端からはみ出さないよう収める
                Anchor::Cursor => (cx - w / 2).clamp(l + m, (r - w - m).max(l + m)),
            };
            let y = match s.anchor {
                Anchor::BottomLeft | Anchor::BottomCenter | Anchor::BottomRight => b - h - m,
                Anchor::TopLeft | Anchor::TopCenter | Anchor::TopRight => t + m,
                Anchor::Center => t + ((b - t) - h) / 2,
                // カーソルが作業領域の下半分なら下端、上半分なら上端に寄せる
                Anchor::Cursor => {
                    if cy > (t + b) / 2 {
                        b - h - m
                    } else {
                        t + m
                    }
                }
            };
            let _ = win.set_position(PhysicalPosition::new(x, y));
            log(
                app,
                format!(
                    "place {:?} -> ({x},{y}) workarea=({l},{t},{r},{b}) size=({w},{h}) cursor=({cx},{cy})",
                    s.anchor
                ),
            );
        }
        None => {
            let _ = win.center();
            log(app, "place: 作業領域を取得できず、中央に配置");
        }
    }
}

// ---------------------------------------------------------------- 開閉

fn show_window(app: &AppHandle) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    let s = app.state::<AppState>().settings.lock().unwrap().clone();
    place(&win, &s, app);
    *app.state::<AppState>().last_shown.lock().unwrap() = Some(Instant::now());
    let _ = win.show();
    let _ = win.set_focus();
    log(
        app,
        format!("show; focused right after = {:?}", win.is_focused()),
    );

    // 他アプリが前面にいると、set_focus が効かないことがある。
    // フォーカスが取れていないと「外側をクリックして閉じる」が働かないので、200ms後にも確認する。
    let w2 = win.clone();
    let app2 = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        log(
            &app2,
            format!("focused after 200ms = {:?}", w2.is_focused()),
        );
    });
}

fn hide_window(app: &AppHandle, reason: &str) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        let _ = win.hide();
        let st = app.state::<AppState>();
        *st.last_hidden.lock().unwrap() = Some(Instant::now());
        log(app, format!("hide ({reason})"));
    }
}

fn toggle_window(app: &AppHandle, via: &str) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    if win.is_visible().unwrap_or(false) {
        log(app, format!("toggle via {via}: 表示中 -> 閉じる"));
        hide_window(app, "toggle");
        return;
    }
    let st = app.state::<AppState>();
    let guard_ms = st.settings.lock().unwrap().guard_ms;
    let since = st.last_hidden.lock().unwrap().map(|t| t.elapsed());
    if let Some(d) = since {
        if d < Duration::from_millis(guard_ms) {
            log(
                app,
                format!(
                    "toggle via {via}: 無視（閉じてから {} ms < ガード {guard_ms} ms）",
                    d.as_millis()
                ),
            );
            return;
        }
    }
    let ago = since
        .map(|d| d.as_millis().to_string())
        .unwrap_or_else(|| "-".to_string());
    log(
        app,
        format!("toggle via {via}: 非表示 -> 開く（前回閉じてから {ago} ms）"),
    );
    show_window(app);
}

// ---------------------------------------------------------------- フォーカス喪失

// 左マウスボタンが押されているか
#[cfg(windows)]
fn left_button_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    // VK_LBUTTON = 0x01。最上位ビットが押下中
    unsafe { (GetAsyncKeyState(0x01) as u16) & 0x8000 != 0 }
}

#[cfg(not(windows))]
fn left_button_down() -> bool {
    false
}

// フォーカスが外れたとき。ボタンを押したまま外れたのは、エクスプローラーのファイルを掴んで
// ドラッグしている最中かもしれない。その場で閉じるとドロップ先がなくなるので、
// ボタンを離すまで待ち、離した先がこのウィンドウへのドロップなら閉じない。
fn handle_blur(app: &AppHandle) {
    // バックアップのダイアログを出している間は、フォーカスが移るので閉じない
    if app
        .state::<AppState>()
        .busy
        .load(std::sync::atomic::Ordering::SeqCst)
    {
        log(app, "フォーカス喪失（ダイアログ表示中なので閉じない）");
        return;
    }
    let on = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .hide_on_blur;
    if !on {
        log(app, "フォーカス喪失（hideOnBlur オフなので閉じない）");
        return;
    }
    // 表示した直後（起動時など）に、他のウィンドウにフォーカスを奪われて一瞬で閉じることがある。
    // 表示から 300 ms 以内のフォーカス喪失は無視し、フォーカスを取り直す。
    let shown = app
        .state::<AppState>()
        .last_shown
        .lock()
        .unwrap()
        .map(|t| t.elapsed());
    if let Some(d) = shown {
        if d < Duration::from_millis(300) {
            log(
                app,
                format!(
                    "フォーカス喪失: 表示から {} ms なので無視して取り直す",
                    d.as_millis()
                ),
            );
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.set_focus();
            }
            return;
        }
    }
    if !left_button_down() {
        hide_window(app, "フォーカス喪失");
        return;
    }
    log(app, "フォーカス喪失（ボタン押下中）: 離すまで待つ");
    let app2 = app.clone();
    let t0 = Instant::now();
    std::thread::spawn(move || {
        while left_button_down() && t0.elapsed() < Duration::from_secs(60) {
            std::thread::sleep(Duration::from_millis(30));
        }
        // ドロップのイベントは、ボタンを離した直後に届く
        std::thread::sleep(Duration::from_millis(250));
        let dropped = app2
            .state::<AppState>()
            .last_drop
            .lock()
            .unwrap()
            .is_some_and(|t| t >= t0);
        let focused = app2
            .get_webview_window("main")
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false);
        if dropped || focused {
            log(&app2, "閉じない（ドロップを受けた／フォーカスが戻った）");
        } else {
            hide_window(&app2, "フォーカス喪失（ボタンを離した後）");
        }
    });
}

// ---------------------------------------------------------------- 画面から呼ぶコマンド

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.settings.lock().unwrap().clone()
}

#[tauri::command]
fn set_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<(), String> {
    save_settings(&state.config_path, &settings).map_err(|e| e.to_string())?;
    *state.settings.lock().unwrap() = settings;
    log(&app, "settings saved");
    Ok(())
}

#[tauri::command]
fn get_logs(state: State<'_, AppState>) -> Vec<LogLine> {
    state.logs.lock().unwrap().clone()
}

#[tauri::command]
fn hide_me(app: AppHandle) {
    hide_window(&app, "Esc");
}

// アプリを終了する。常駐アプリなので、閉じるだけでは終了しない。
// （終了しないと、ビルドで exe を置き換えるときに「アクセスが拒否されました」になる）
#[tauri::command]
fn quit_app(app: AppHandle) {
    log(&app, "quit");
    app.exit(0);
}

// 位置を選んだ直後に、その場で動かして見せる
#[tauri::command]
fn preview_position(app: AppHandle, state: State<'_, AppState>, anchor: Anchor) {
    // 「押した場所」は、設定画面の上にカーソルがある今は確かめられない。次に開くときに反映される。
    if anchor == Anchor::Cursor {
        log(
            &app,
            "アイコンの位置: 次にピン止めアイコンから開くときに反映",
        );
        return;
    }
    if let Some(win) = app.get_webview_window("main") {
        let mut s = state.settings.lock().unwrap().clone();
        s.anchor = anchor;
        place(&win, &s, &app);
    }
}

// ---------------------------------------------------------------- アプリ一覧・アイコン・起動

// スタートメニューの .lnk を集めて、フォルダ構造のまま返す。
// ディスクを読むので、画面を止めないよう別スレッドで実行する。
#[tauri::command]
async fn list_apps(app: AppHandle) -> Result<Vec<startmenu::AppNode>, String> {
    let t = Instant::now();
    let app2 = app.clone();
    let tree = tauri::async_runtime::spawn_blocking(move || {
        let mut tree = startmenu::scan(&startmenu::default_roots());
        // Windows が表示している名前（日本語など）に直す
        appsfolder::localize(&mut tree);
        // .lnk のないアプリ（ストアアプリ）を足す。失敗しても .lnk だけの一覧は出す
        match appsfolder::enumerate() {
            Ok(raw) => {
                let total = raw.len();
                let extra = appsfolder::store_nodes(raw, &tree);
                log(
                    &app2,
                    format!(
                        "アプリフォルダ: {total} 件中、ストアアプリ {} 件を追加",
                        extra.len()
                    ),
                );
                tree.extend(extra);
            }
            Err(e) => log(&app2, format!("アプリフォルダの列挙に失敗: {e}")),
        }
        tree
    })
    .await
    .map_err(|e| e.to_string())?;
    fn count(v: &[startmenu::AppNode]) -> usize {
        v.iter()
            .map(|n| n.children.as_ref().map_or(1, |c| count(c)))
            .sum()
    }
    log(
        &app,
        format!(
            "アプリ一覧: {} 件 ({} ms)",
            count(&tree),
            t.elapsed().as_millis()
        ),
    );
    Ok(tree)
}

// 1件のアイコンを PNG の data URL で返す（ディスクにキャッシュする）。size は表示したい大きさ（px、省略で32）
#[tauri::command]
async fn get_icon(app: AppHandle, path: String, size: Option<u32>) -> Result<String, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("icons");
    let size = size.unwrap_or(32);
    let path_for_log = path.clone();
    let r = tauri::async_runtime::spawn_blocking(move || {
        icons::icon_data_url(&dir, Path::new(&path), size)
    })
    .await
    .map_err(|e| e.to_string())?;
    if let Err(e) = &r {
        log(
            &app,
            format!("アイコン取得に失敗 ({size}px) {path_for_log}: {e}"),
        );
    }
    r
}

// .lnk を、ダブルクリックしたときと同じ扱いで開く
#[cfg(windows)]
fn shell_open(path: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(std::iter::once(0)).collect();
    // exe などは、そのフォルダを作業フォルダにして起動する（.lnk は自分の指定に任せる）
    let p = Path::new(path);
    let is_link = p
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("lnk") || e.eq_ignore_ascii_case("url"));
    let dir: Vec<u16> = match p.parent() {
        Some(d) if p.is_file() && !is_link => d
            .to_string_lossy()
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect(),
        _ => Vec::new(),
    };
    let dir_ptr = if dir.is_empty() {
        PCWSTR::null()
    } else {
        PCWSTR(dir.as_ptr())
    };
    let r = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            dir_ptr,
            SW_SHOWNORMAL,
        )
    };
    // 成功すると 32 より大きい値が返る
    let code = r.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("ShellExecuteW が失敗 (code {code})"))
    }
}

#[cfg(not(windows))]
fn shell_open(_path: &str) -> Result<(), String> {
    Err("起動は Windows のみ".to_string())
}

fn load_usage(path: &std::path::Path) -> Vec<usage::Entry> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

// 起動に成功したものを履歴に足す（失敗しても起動自体には影響させない）
fn record_usage(state: &AppState, path: &str) {
    // フォルダは「最近使ったアプリ」に入れない（ブックマークから開くフォルダなど）
    if Path::new(path).is_dir() {
        return;
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let list = usage::record(load_usage(&state.usage_path), path, now);
    if let Ok(json) = serde_json::to_vec(&list) {
        let _ = safe_write(&state.usage_path, &json);
    }
}

// 最近起動したもの（新しい順）
#[tauri::command]
fn get_recent(state: State<'_, AppState>, limit: Option<usize>) -> Vec<usage::Entry> {
    usage::recent(&load_usage(&state.usage_path), limit.unwrap_or(8).min(50))
}

#[tauri::command]
fn launch(app: AppHandle, state: State<'_, AppState>, path: String) -> Result<(), String> {
    match shell_open(&path) {
        Ok(()) => {
            log(&app, format!("起動: {path}"));
            record_usage(&state, &path);
            hide_window(&app, "起動した");
            Ok(())
        }
        Err(e) => {
            log(&app, format!("起動に失敗: {path} ({e})"));
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- タイル配置の保存

// 配置の中身（JSON 文字列）は画面側で検証する。ここでは「JSON として読めるか」だけ確かめる。
// 本体が壊れていたら、1世代前（.bak）を返す。
#[tauri::command]
fn load_tiles(app: AppHandle, state: State<'_, AppState>) -> Option<String> {
    let main = state.tiles_path.clone();
    let bak = main.with_extension("json.bak");
    for (label, path) in [("tiles.json", main), ("tiles.json.bak", bak)] {
        if let Ok(s) = fs::read_to_string(&path) {
            if serde_json::from_str::<serde_json::Value>(&s).is_ok() {
                log(
                    &app,
                    format!("タイル: {label} を読み込み ({} bytes)", s.len()),
                );
                return Some(s);
            }
            log(&app, format!("タイル: {label} が壊れている"));
        }
    }
    None
}

#[tauri::command]
fn save_tiles(app: AppHandle, state: State<'_, AppState>, json: String) -> Result<(), String> {
    if json.len() > 5_000_000 {
        return Err("大きすぎる".to_string());
    }
    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| e.to_string())?;
    safe_write(&state.tiles_path, json.as_bytes()).map_err(|e| e.to_string())?;
    log(&app, format!("タイルを保存 ({} bytes)", json.len()));
    Ok(())
}

// ドロップされたパスを、タイルにするための情報（表示名・フォルダかどうか）にする。存在しないものは除く。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PathInfo {
    path: String,
    name: String,
    is_dir: bool,
}

fn inspect_path(p: &str) -> Option<PathInfo> {
    let pb = Path::new(p);
    let is_dir = fs::metadata(pb).ok()?.is_dir();
    let fname = pb.file_name()?.to_string_lossy().into_owned();
    let name = if is_dir {
        fname
    } else {
        // 実行・ショートカット系は拡張子を外す。それ以外の書類などは、拡張子つきのまま
        let ext = pb
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        match ext.as_deref() {
            Some("exe" | "lnk" | "url" | "bat" | "cmd" | "com") => pb
                .file_stem()
                .map(|x| x.to_string_lossy().into_owned())
                .unwrap_or(fname),
            _ => fname,
        }
    };
    Some(PathInfo {
        path: p.to_string(),
        name,
        is_dir,
    })
}

#[tauri::command]
fn inspect_paths(paths: Vec<String>) -> Vec<PathInfo> {
    paths.iter().filter_map(|p| inspect_path(p)).collect()
}

// 画面側（JS）の出来事をログに残す
// ---------------------------------------------------------------- 左端アイコン列の保存

// 中身は画面側で検証する。ここでは「JSON として読めるか」と、1 世代前（.bak）への戻しだけ。
#[tauri::command]
fn load_rail(app: AppHandle, state: State<'_, AppState>) -> Option<String> {
    let main = state.rail_path.clone();
    let bak = main.with_extension("json.bak");
    for (label, path) in [("rail.json", main), ("rail.json.bak", bak)] {
        if let Ok(s) = fs::read_to_string(&path) {
            if serde_json::from_str::<serde_json::Value>(&s).is_ok() {
                log(
                    &app,
                    format!("左端: {label} を読み込み ({} bytes)", s.len()),
                );
                return Some(s);
            }
            log(&app, format!("左端: {label} が壊れている"));
        }
    }
    None
}

#[tauri::command]
fn save_rail(app: AppHandle, state: State<'_, AppState>, json: String) -> Result<(), String> {
    if json.len() > 1_000_000 {
        return Err("大きすぎる".to_string());
    }
    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| e.to_string())?;
    safe_write(&state.rail_path, json.as_bytes()).map_err(|e| e.to_string())?;
    log(&app, format!("左端を保存 ({} bytes)", json.len()));
    Ok(())
}

// ---------------------------------------------------------------- ブックマーク

#[tauri::command]
fn load_bookmarks(app: AppHandle, state: State<'_, AppState>) -> Option<String> {
    let main = state.bookmarks_path.clone();
    let bak = main.with_extension("json.bak");
    for (label, path) in [("bookmarks.json", main), ("bookmarks.json.bak", bak)] {
        if let Ok(s) = fs::read_to_string(&path) {
            if serde_json::from_str::<serde_json::Value>(&s).is_ok() {
                log(
                    &app,
                    format!("ブックマーク: {label} を読み込み ({} bytes)", s.len()),
                );
                return Some(s);
            }
            log(&app, format!("ブックマーク: {label} が壊れている"));
        }
    }
    None
}

#[tauri::command]
fn save_bookmarks(app: AppHandle, state: State<'_, AppState>, json: String) -> Result<(), String> {
    if json.len() > 2_000_000 {
        return Err("大きすぎる".to_string());
    }
    serde_json::from_str::<serde_json::Value>(&json).map_err(|e| e.to_string())?;
    safe_write(&state.bookmarks_path, json.as_bytes()).map_err(|e| e.to_string())?;
    log(&app, format!("ブックマークを保存 ({} bytes)", json.len()));
    Ok(())
}

// コマンドを、新しいコンソール（cmd.exe）で実行する。一時的な .bat に書き出して走らせる。
// keep_open が真なら終わっても閉じない（/K）。ビルドの出力を読みたいときなど。
#[cfg(windows)]
fn spawn_script(script: &str, keep_open: bool, cwd: &str) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    let dir = std::env::temp_dir().join("ddlauncher");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // 1日より古い一時ファイルは片付ける
    if let Ok(rd) = fs::read_dir(&dir) {
        for e in rd.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|d| d > Duration::from_secs(86_400));
            if old {
                let _ = fs::remove_file(e.path());
            }
        }
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let file = dir.join(format!("cmd-{nanos}.bat"));
    fs::write(&file, script).map_err(|e| e.to_string())?;
    let mut c = std::process::Command::new("cmd.exe");
    // cmd /K ""パス"" の形（パスに空白があっても通る）
    c.raw_arg(format!(
        "{} \"\"{}\"\"",
        if keep_open { "/K" } else { "/C" },
        file.display()
    ));
    if !cwd.trim().is_empty() && Path::new(cwd.trim()).is_dir() {
        c.current_dir(cwd.trim());
    }
    c.creation_flags(0x0000_0010); // CREATE_NEW_CONSOLE
    c.spawn().map(|_| ()).map_err(|e| e.to_string())
}

#[cfg(not(windows))]
fn spawn_script(_script: &str, _keep_open: bool, _cwd: &str) -> Result<(), String> {
    Err("コマンドの実行は Windows のみ".to_string())
}

#[tauri::command]
fn run_command(
    app: AppHandle,
    command: String,
    cwd: Option<String>,
    keep_open: bool,
) -> Result<(), String> {
    if command.trim().is_empty() {
        return Err("コマンドが空".to_string());
    }
    let cwd = cwd.unwrap_or_default();
    let script = script::build_script(&command, &cwd);
    match spawn_script(&script, keep_open, &cwd) {
        Ok(()) => {
            let first = command.lines().next().unwrap_or("");
            log(
                &app,
                format!(
                    "コマンドを実行: {first} …（{} 行）",
                    command.lines().count()
                ),
            );
            hide_window(&app, "コマンドを実行した");
            Ok(())
        }
        Err(e) => {
            log(&app, format!("コマンドの実行に失敗: {e}"));
            Err(e)
        }
    }
}

// ---------------------------------------------------------------- バックアップ（書き出し・読み込み）

fn read_opt(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

// ファイル選択ダイアログを出す間だけ busy を立てる（閉じる処理を止める）
struct Busy<'a>(&'a AppState);
impl<'a> Busy<'a> {
    fn on(state: &'a AppState) -> Self {
        state.busy.store(true, std::sync::atomic::Ordering::SeqCst);
        Busy(state)
    }
}
impl Drop for Busy<'_> {
    fn drop(&mut self) {
        self.0
            .busy
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

// 戻り値: 書き出したパス。キャンセルなら None。
// ダイアログは待つ処理なので、async のコマンドにする（同期だとメインスレッドを止めてしまう）
#[tauri::command]
async fn export_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let settings =
        serde_json::to_string(&*state.settings.lock().unwrap()).map_err(|e| e.to_string())?;
    let bundle = backup::build(
        env!("CARGO_PKG_VERSION"),
        Some(&settings),
        read_opt(&state.tiles_path).as_deref(),
        read_opt(&state.rail_path).as_deref(),
        read_opt(&state.bookmarks_path).as_deref(),
    )?;
    let picked = {
        let _busy = Busy::on(&state);
        app.dialog()
            .file()
            .set_title("バックアップの保存先")
            .set_file_name("ddlauncher-backup.json")
            .add_filter("JSON", &["json"])
            .blocking_save_file()
    };
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    fs::write(&path, bundle).map_err(|e| e.to_string())?;
    log(&app, format!("バックアップを書き出し: {}", path.display()));
    Ok(Some(path.display().to_string()))
}

// 戻り値: 読み込んだ内容の説明。キャンセルなら None。
// 中身を検証してから書く。書く前の内容は、各ファイルの .bak に残る。
#[tauri::command]
async fn import_backup(
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    let picked = {
        let _busy = Busy::on(&state);
        app.dialog()
            .file()
            .set_title("読み込むバックアップ")
            .add_filter("JSON", &["json"])
            .blocking_pick_file()
    };
    let Some(fp) = picked else { return Ok(None) };
    let path = fp.into_path().map_err(|e| e.to_string())?;
    let text = fs::read_to_string(&path).map_err(|e| format!("読めなかった: {e}"))?;
    if text.len() > 10_000_000 {
        return Err("大きすぎる".to_string());
    }
    let parts = backup::parse(&text)?;
    // 設定は、型に当てはまるかも確かめる（書く前に全部検証する）
    let new_settings: Option<Settings> = match &parts.settings {
        Some(v) => {
            Some(serde_json::from_value(v.clone()).map_err(|e| format!("設定が読めない: {e}"))?)
        }
        None => None,
    };
    let mut done = Vec::new();
    for (label, part, target) in [
        ("タイル", &parts.tiles, &state.tiles_path),
        ("左端", &parts.rail, &state.rail_path),
        ("ブックマーク", &parts.bookmarks, &state.bookmarks_path),
    ] {
        if let Some(v) = part {
            let bytes = serde_json::to_vec_pretty(v).map_err(|e| e.to_string())?;
            safe_write(target, &bytes).map_err(|e| format!("{label}を書けなかった: {e}"))?;
            done.push(label);
        }
    }
    if let Some(s) = new_settings {
        save_settings(&state.config_path, &s).map_err(|e| format!("設定を書けなかった: {e}"))?;
        let hk = s.hotkey.clone();
        *state.settings.lock().unwrap() = s;
        done.push("設定");
        // ホットキーも読み込んだ内容で登録し直す
        if let Err(e) = register_hotkey(&app, &hk) {
            log(&app, format!("ホットキー: 登録に失敗 {hk}: {e}"));
        }
    }
    let msg = format!("読み込みました: {}", done.join("・"));
    log(
        &app,
        format!("バックアップを読み込み: {} ({msg})", path.display()),
    );
    Ok(Some(msg))
}

#[tauri::command]
fn log_msg(app: AppHandle, msg: String) {
    log(&app, msg);
}

// トレイメニューの「自動起動」チェック。画面の設定から切り替えたときも同期する。
struct TrayAuto(tauri::menu::CheckMenuItem<tauri::Wry>);

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    use tauri_plugin_autostart::ManagerExt;
    app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enable: bool) -> Result<(), String> {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    let r = if enable { mgr.enable() } else { mgr.disable() };
    r.map_err(|e| e.to_string())?;
    log(
        &app,
        format!(
            "自動起動: {}（設定画面）",
            if enable { "オン" } else { "オフ" }
        ),
    );
    if let Some(t) = app.try_state::<TrayAuto>() {
        let _ = t.0.set_checked(enable);
    }
    Ok(())
}

// ---------------------------------------------------------------- グローバルホットキー

// 登録し直す。空なら解除だけ。他のアプリや OS とキーが重なると失敗するので、エラーを返して画面に出す。
fn register_hotkey(app: &AppHandle, accel: &str) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    let gs = app.global_shortcut();
    gs.unregister_all().map_err(|e| e.to_string())?;
    let accel = accel.trim();
    if accel.is_empty() {
        return Ok(());
    }
    gs.register(accel).map_err(|e| e.to_string())
}

#[tauri::command]
fn apply_hotkey(app: AppHandle, accel: String) -> Result<(), String> {
    match register_hotkey(&app, &accel) {
        Ok(()) => {
            log(
                &app,
                if accel.trim().is_empty() {
                    "ホットキー: 解除".to_string()
                } else {
                    format!("ホットキー: 登録 {accel}")
                },
            );
            Ok(())
        }
        Err(e) => {
            log(&app, format!("ホットキー: 登録に失敗 {accel}: {e}"));
            Err(e)
        }
    }
}

// ホットキーが押されたとき。閉じていれば開き（設定によっては検索欄から始め）、開いていれば閉じる。
fn on_hotkey(app: &AppHandle) {
    let visible = app
        .get_webview_window("main")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    toggle_window(app, "ホットキー");
    let search = app
        .state::<AppState>()
        .settings
        .lock()
        .unwrap()
        .search_on_hotkey;
    if !visible && search {
        let _ = app.emit("ddl-open-search", ());
    }
}

// ---------------------------------------------------------------- タスクトレイ

// 終了する。タイル配置の保存待ちがあれば、画面側に先に書かせてから終わる。
fn quit_with_flush(app: &AppHandle) {
    log(app, "quit (トレイ)");
    let _ = app.emit("ddl-before-quit", ());
    let a = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        a.exit(0);
    });
}

// トレイアイコン。左クリックで開閉、右クリックでメニュー（開閉・自動起動・終了）。
// ピン止めを外しても、ここから開けて、終了できる。
fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
    use tauri_plugin_autostart::ManagerExt;

    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let toggle = MenuItem::with_id(app, "toggle", "開く / 閉じる", true, None::<&str>)?;
    let auto = CheckMenuItem::with_id(
        app,
        "autostart",
        "Windows 起動時に自動で起動",
        true,
        autostart_on,
        None::<&str>,
    )?;
    let settings_item = MenuItem::with_id(app, "settings", "設定…", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "終了", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&toggle, &auto, &settings_item, &sep, &quit])?;

    app.manage(TrayAuto(auto.clone()));
    let auto_item = auto.clone();
    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("DDlauncher")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, ev| {
            let id: &str = ev.id().as_ref();
            match id {
                "toggle" => toggle_window(app, "トレイのメニュー"),
                "autostart" => {
                    // クリックされた時点で、チェックの状態は切り替わっている
                    let want = auto_item.is_checked().unwrap_or(false);
                    let mgr = app.autolaunch();
                    let r = if want { mgr.enable() } else { mgr.disable() };
                    match r {
                        Ok(()) => log(
                            app,
                            format!("自動起動: {}", if want { "オン" } else { "オフ" }),
                        ),
                        Err(e) => {
                            log(app, format!("自動起動の切り替えに失敗: {e}"));
                            let _ = auto_item.set_checked(!want);
                        }
                    }
                }
                "settings" => {
                    show_window(app);
                    let _ = app.emit("ddl-open-settings", ());
                }
                "quit" => quit_with_flush(app),
                _ => {}
            }
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                toggle_window(tray.app_handle(), "トレイのアイコン");
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

// ---------------------------------------------------------------- 起動

fn main() {
    tauri::Builder::default()
        // single-instance は最初に登録する。2回目の起動はここで受けて、2回目のプロセスは終了する。
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            log(app, format!("2回目の起動を検知 args={args:?}"));
            toggle_window(app, "2回目の起動");
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, ev| {
                    if ev.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        on_hotkey(app);
                    }
                })
                .build(),
        )
        // Windows 起動時の自動起動。--hidden を付けて起動し、表示せずに常駐だけする
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            set_settings,
            get_logs,
            hide_me,
            quit_app,
            preview_position,
            list_apps,
            get_icon,
            launch,
            get_recent,
            export_backup,
            import_backup,
            load_bookmarks,
            save_bookmarks,
            run_command,
            apply_hotkey,
            load_rail,
            save_rail,
            get_autostart,
            set_autostart,
            load_tiles,
            save_tiles,
            log_msg,
            inspect_paths
        ])
        .setup(|app| {
            let config_path = app.path().app_config_dir()?.join("settings.json");
            let settings = load_settings(&config_path);
            app.manage(AppState {
                settings: Mutex::new(settings),
                last_hidden: Mutex::new(None),
                last_drop: Mutex::new(None),
                last_shown: Mutex::new(None),
                logs: Mutex::new(Vec::new()),
                tiles_path: config_path.with_file_name("tiles.json"),
                usage_path: config_path.with_file_name("usage.json"),
                rail_path: config_path.with_file_name("rail.json"),
                bookmarks_path: config_path.with_file_name("bookmarks.json"),
                busy: std::sync::atomic::AtomicBool::new(false),
                config_path,
            });

            let win = app.get_webview_window("main").expect("main window");
            let handle = app.handle().clone();
            win.on_window_event(move |ev| match ev {
                WindowEvent::Focused(false) => handle_blur(&handle),
                WindowEvent::DragDrop(DragDropEvent::Drop { .. }) => {
                    // ドロップを受けた印を残す（フォーカス喪失で閉じる処理が見る）。
                    // ドロップ元（エクスプローラー）が前面のままなので、こちらへフォーカスを戻す。
                    *handle.state::<AppState>().last_drop.lock().unwrap() = Some(Instant::now());
                    if let Some(w) = handle.get_webview_window("main") {
                        let _ = w.set_focus();
                    }
                }
                _ => {}
            });

            let hk = app
                .state::<AppState>()
                .settings
                .lock()
                .unwrap()
                .hotkey
                .clone();
            if !hk.trim().is_empty() {
                match register_hotkey(app.handle(), &hk) {
                    Ok(()) => log(app.handle(), format!("ホットキー: 登録 {hk}")),
                    Err(e) => log(app.handle(), format!("ホットキー: 登録に失敗 {hk}: {e}")),
                }
            }
            if let Err(e) = setup_tray(app.handle()) {
                log(app.handle(), format!("トレイアイコンを作れなかった: {e}"));
            }

            // --hidden を付けて起動すると、常駐だけして表示しない（自動起動用の布石）
            if !std::env::args().any(|a| a == "--hidden") {
                show_window(app.handle());
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("DDlauncher の起動に失敗しました");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inspect_path_names() {
        let base = std::env::temp_dir().join(format!("ddl-inspect-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(base.join("My Tool")).unwrap();
        fs::write(base.join("app.EXE"), b"x").unwrap();
        fs::write(base.join("memo.txt"), b"x").unwrap();
        let s = |p: PathBuf| p.to_string_lossy().into_owned();

        let a = inspect_path(&s(base.join("app.EXE"))).unwrap();
        assert_eq!((a.name.as_str(), a.is_dir), ("app", false));
        let t = inspect_path(&s(base.join("memo.txt"))).unwrap();
        assert_eq!(t.name, "memo.txt");
        let d = inspect_path(&s(base.join("My Tool"))).unwrap();
        assert_eq!((d.name.as_str(), d.is_dir), ("My Tool", true));
        assert!(inspect_path(&s(base.join("none.exe"))).is_none());
        let _ = fs::remove_dir_all(&base);
    }
}
