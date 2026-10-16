fn main() {
    // tauri-build はアイコンの変更を検知しない。アイコンだけ差し替えても exe に反映されるようにする
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    tauri_build::build()
}
