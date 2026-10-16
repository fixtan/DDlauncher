# ビルドの注意メモ

## アイコンだけ変えても exe に反映されない（2026-10-16）
- 症状: `src-tauri/icons/icon.ico` を差し替えて `npm run installer` を実行しても、exe のアイコンが古いまま。再起動してもキャッシュ削除をしても直らなかった。
- 原因（tauri-build 2.7.3 のソースを読んだ結果）: アイコンファイルを `cargo:rerun-if-changed` に出していない。アイコンだけの変更では build.rs が再実行されず、埋め込み済みのリソースが使い回される。`tauri.conf.json` を変えたときだけ再実行される。
- 対処: `src-tauri/build.rs` に `rerun-if-changed=icons/icon.ico` と `icons/icon.png` を追加（これで直った）。
- 保険: それでも直らなければ `cd src-tauri` → `cargo clean` → `cd ..` → `npm run installer`。
- 見分け方: トレイのアイコンは埋め込んだ画像から作るので、Explorer のアイコンキャッシュを通らない。トレイが古ければビルド側、トレイが新しいのに Explorer だけ古ければキャッシュ。
- Windows のアイコンキャッシュ削除: `ie4uinit.exe -show`、それでもだめなら explorer 再起動と `IconCache.db` の削除。

## その他
- zip を上書きして使うと、`.gitignore` も上書きされる。`.history/`（VS Code の Local History）を `.gitignore` に入れておくこと。
- ビルドの前に、起動中の DDlauncher を終了しておく（`アクセスが拒否されました (os error 5)`）。
- NSIS インストーラーは `npm run installer`。出力: `src-tauri/target/release/bundle/nsis/DDlauncher_<version>_x64-setup.exe`。
