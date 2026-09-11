//! exe アイコン埋め込み（タスク #24）。
//!
//! Java 版と同じ `icon.ico` を exe リソースとして埋め込む。Windows ターゲットの
//! ときだけ動作し、`rc.exe` が見つからない環境では警告のみ出してビルドを成功させる。

fn main() {
    // アイコンを差し替えたら build.rs を再実行させる。
    println!("cargo:rerun-if-changed=assets/icon.ico");

    #[cfg(windows)]
    embed_icon();
}

/// `assets/icon.ico` を exe に埋め込む。
///
/// `rc.exe` が見つからない場合は警告を出してスキップする（他環境でビルド不能にしない）。
#[cfg(windows)]
fn embed_icon() {
    let icon = "assets/icon.ico";
    if !std::path::Path::new(icon).is_file() {
        println!("cargo:warning={icon} が見つからないため exe アイコンを埋め込みません");
        return;
    }

    let Some(rc_exe) = find_rc_exe() else {
        println!(
            "cargo:warning=rc.exe が見つからないため exe アイコンを埋め込みません \
             (Windows SDK をインストールすると有効になります)"
        );
        return;
    };
    println!("exe icon resource compiler: {}", rc_exe.display());

    let mut res = winresource::WindowsResource::new();
    // winresource の MSVC 実装は rc.exe を PATH ではなく toolkit_path から解決するため、
    // 検出した rc.exe のディレクトリを公式 API で明示的に渡す。
    let bin_dir = rc_exe
        .parent()
        .expect("rc.exe には親ディレクトリがある")
        .to_str()
        .expect("rc.exe のパスは UTF-8");
    res.set_toolkit_path(bin_dir);
    res.set_icon(icon);
    // rc.exe は見つかったのに埋め込みに失敗する場合は実際の異常なのでビルドを失敗させる。
    if let Err(e) = res.compile() {
        panic!("exe アイコンの埋め込みに失敗しました: {e}");
    }
}

/// `rc.exe` を探す。見つからなければ `None`。
///
/// ① PATH 上の `rc.exe`
/// ② Windows SDK（`Windows Kits\10\bin\<version>\x64\rc.exe`）のうち最高バージョン
#[cfg(windows)]
fn find_rc_exe() -> Option<std::path::PathBuf> {
    find_rc_in_path().or_else(find_rc_in_windows_kits)
}

/// `PATH` の各ディレクトリから `rc.exe` を探す。
#[cfg(windows)]
fn find_rc_in_path() -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join("rc.exe"))
        .find(|rc| rc.is_file())
}

/// `%ProgramFiles(x86)%` / `%ProgramFiles%` 配下の Windows SDK を探索し、
/// バージョンディレクトリが最も新しい `x64\rc.exe` を返す。
#[cfg(windows)]
fn find_rc_in_windows_kits() -> Option<std::path::PathBuf> {
    let mut best: Option<(Vec<u64>, std::path::PathBuf)> = None;
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        let Some(base) = std::env::var_os(var) else {
            continue;
        };
        let bin_root = std::path::PathBuf::from(base)
            .join("Windows Kits")
            .join("10")
            .join("bin");
        let Ok(entries) = std::fs::read_dir(&bin_root) else {
            continue;
        };
        for entry in entries.flatten() {
            let rc = entry.path().join("x64").join("rc.exe");
            if !rc.is_file() {
                continue;
            }
            let key = version_key(&entry.file_name().to_string_lossy());
            if best.as_ref().is_none_or(|(best_key, _)| key > *best_key) {
                best = Some((key, rc));
            }
        }
    }
    best.map(|(_, rc)| rc)
}

/// `10.0.26100.0` のようなバージョンディレクトリ名を数値列に変換する。
#[cfg(windows)]
fn version_key(name: &str) -> Vec<u64> {
    name.split('.')
        .map(|part| part.parse().unwrap_or(0))
        .collect()
}
