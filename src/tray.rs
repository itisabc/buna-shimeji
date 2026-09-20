//! タスクトレイ + 設定永続化（タスク #9c・design §3-11 / §3-13 / §3-14）。
//!
//! トレイ項目は design §3-11（ユーザー承認済み）が正本
//! （Java 正本 TrayMenu.java / TrayMenuPanel.java は java-ref に存在しない）。
//! 各項目の適用先は既存 Manager API（#8〜#9d）で:
//! 呼ぶ = [`Manager::request_spawn`]（「（ランダム）」は
//! [`Manager::request_spawn_random`]）/ Follow Cursor = 全員 ChaseMouse /
//! Reduce to One = [`Manager::remain_one`] / Restore Windows =
//! [`Manager::restore_windows`] / Allowed Behaviours 6 トグル = passthrough +
//! settings.toml 即時保存 / 一時停止 / Dismiss All（exit は次 tick の
//! [`Manager::should_exit`] 経由）/ Reload = [`load_materials`] +
//! [`Manager::reload`]。SetAllowed の適用値は MenuEvent 受信時の checked
//! そのまま（muda 0.19.3 は MenuEvent 発行前に CheckMenuItem を自動トグル
//! する・`platform_impl/windows/mod.rs` L1195-1198 実物照合・
//! [`TrayMenuModel::command_of`] / [`TrayMenuModel::sync_allowed`] doc 参照）。
//!
//! 設定永続化（[`Settings`]）は Java `Settings.java`（load L61-137 / save
//! L194-267 / 6 トグル既定 true L89-94）を仕様として移植し、`properties` を
//! TOML に置き換えたもの（design §3-14 の形状・AGENTS §6）。ファイル不在時は
//! 既定適用（Java L62 `Files.isRegularFile` 逐語）・未知キーは無視・
//! 欠落セクション / フィールドは既定補完。
//!
//! 本モジュールはメニュー構築（[`TrayMenuModel`]）とコマンド適用
//!（[`apply_tray_command`]）のみを担う。トレイアイコン本体（TrayIconBuilder）と
//! MenuEvent 受信（`MenuEvent::receiver()`）の tao イベントループ結線は
//! **タスク #10 が行う**。
//!
//! Java との対応（#9c 分）:
//! - [`Manager::request_spawn_random`]: Main.createMascot() 無引数版 L466-473 逐語
//! - popup 単体操作: Mascot.java popup L517-562（SetBehaviour L517-522 /
//!   pauseItem L559-560 / Dismiss L562-563）
//! - 6 トグル既定 true: Settings.java L89-94 逐語
//! - Main.setMascotBehaviorEnabled L526-544 の per-key 変異は
//!   `Environment::set_behavior_enabled`（#9b）・settings 復元の全体注入は
//!   `Environment::set_disabled_behaviors`（#9c・別経路）

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem, Submenu};

use crate::app::manager::{BehaviorMenu, Manager};
use crate::app::reload::load_materials;
use crate::i18n::{Lang, UiKey, DEFAULT_LANGUAGE};

// =====================================================================
// トレイアイコン: Java Main.getIcon()（.tmp/java-ref/Main.java L764-792）
// =====================================================================

/// 埋め込み既定トレイアイコン（Java `Main.class.getResourceAsStream("/icon.png")`
/// 相当。上流 @dea8952 の `icon.png`・16×16）。
const EMBEDDED_TRAY_ICON: &[u8] = include_bytes!("../assets/icon.png");

/// トレイアイコンのピクセル（RGBA8 行優先）と寸法 (幅, 高さ) を返す。
///
/// Java `Main.getIcon()`（.tmp/java-ref/Main.java L764-792）の仕様:
/// ① `custom_path` が存在しデコード可能 → その RGBA8（ユーザーカスタム優先）
/// ② 存在しない（あるいはファイルでない）→ 無言で埋め込み既定
///    [`EMBEDDED_TRAY_ICON`] の RGBA8（Java L770 `Files.isRegularFile` 逐語）
/// ③ ファイルとして存在するがデコード失敗 → `log::warn!` のうえ埋め込み既定
///    （Java L775-777 `Failed to load custom icon file`）
/// ④ 埋め込み既定のデコードも失敗 → 16×16 透明 RGBA（固定フォールバック）
///
/// 常に有効な RGBA を返し panic しない。
pub fn load_tray_icon_rgba(custom_path: &Path) -> (Vec<u8>, u32, u32) {
    if custom_path.is_file() {
        match image::open(custom_path) {
            Ok(decoded) => {
                let rgba = decoded.to_rgba8();
                let (width, height) = rgba.dimensions();
                return (rgba.into_raw(), width, height);
            }
            // Java L775-777: カスタム読み込みの例外時のみ warn
            Err(err) => log::warn!(
                "failed to load custom tray icon {}; using default icon: {err}",
                custom_path.display()
            ),
        }
    } else {
        // Java L770: isRegularFile で無ければ無言で既定へ（debug のみ）
        log::debug!(
            "custom tray icon {} does not exist; using default icon",
            custom_path.display()
        );
    }
    match image::load_from_memory(EMBEDDED_TRAY_ICON) {
        Ok(decoded) => {
            let rgba = decoded.to_rgba8();
            let (width, height) = rgba.dimensions();
            (rgba.into_raw(), width, height)
        }
        Err(err) => {
            log::error!("failed to decode embedded default tray icon: {err}");
            let (width, height) = (16u32, 16u32);
            (vec![0; (width * height * 4) as usize], width, height)
        }
    }
}

// =====================================================================
// 契約 A: 設定永続化（conf/settings.toml・design §3-14 形状）
// =====================================================================

/// settings.toml の入出力エラー（パース / シリアライズ / io の 3 経路）。
#[derive(Debug, Error)]
pub enum SettingsError {
    /// settings.toml の読み込み（パース）に失敗（Java L63-67 の IOException 相当）。
    #[error("failed to load settings.toml: {0}")]
    Parse(#[from] toml::de::Error),
    /// settings.toml の書き出し（シリアライズ）に失敗。
    #[error("failed to write settings.toml: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// settings.toml の読み書き（io）に失敗（保存先ディレクトリ不在等）。
    #[error("settings.toml I/O failed: {0}")]
    Io(#[from] std::io::Error),
    /// imagesets.scale が非有限（NaN / ±inf）または 0 以下。
    /// 設定不正として起動エラー経路に載せる（上限は設けない）。
    #[error("invalid settings.toml imagesets.scale ({0} = {1}): specify a positive finite value")]
    InvalidScale(String, f64),
}

/// Allowed Behaviours トグルの種別（トレイ「Allowed Behaviours」サブメニューの
/// 各項目 → [`TrayCommand::SetAllowed`] の対象）。
#[derive(Clone, Copy)]
pub enum AllowedKind {
    /// 増殖（Java `Breeding`）。
    Breeding,
    /// Transients（Java `Transients`）。
    Transients,
    /// 変身（Java `Transformation`）。
    Transformation,
    /// 投げ（Java `Throwing`）。
    Throwing,
    /// 効果音（Java `Sounds`・#36 で `Manager::set_sounds_enabled` へ実配線）。
    Sounds,
    /// 画面間移動（Java `Multiscreen`）。
    Multiscreen,
    /// ドロップしたウィンドウを最前面固定（Rust 独自拡張・機能 #30・design §1.10(z)）。
    PinDroppedWindow,
}

/// Allowed Behaviours 6 トグル（Java Settings.java L89-94 逐語・全て既定 true）。
/// TOML は design §3-14 の `[allowed]` セクション。欠落フィールドは既定 true 補完。
/// `Debug` は [`Settings`] の derive 連鎖（`expect_err` 表示）要件。
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct AllowedSettings {
    /// Java L89 `Breeding`（増殖）。
    pub breeding: bool,
    /// Java L90 `Transients`。
    pub transients: bool,
    /// Java L91 `Transformation`（変身）。
    pub transformation: bool,
    /// Java L92 `Throwing`（投げ）。
    pub throwing: bool,
    /// Java L93 `Sounds`（効果音。#36 で `Environment::sounds_enabled` へ実配線）。
    pub sounds: bool,
    /// Java L94 `Multiscreen`（画面間移動）。
    pub multiscreen: bool,
    /// ドロップしたウィンドウを最前面固定（Rust 独自拡張・機能 #30・**既定 false**）。
    /// Java の Allowed Behaviours には無い項目で、ユーザーの自発ドロップ時のみ発動する。
    pub pin_dropped_window: bool,
}

impl Default for AllowedSettings {
    /// Java Settings.java L89-94 逐語（全て true）。
    fn default() -> AllowedSettings {
        AllowedSettings {
            breeding: true,
            transients: true,
            transformation: true,
            throwing: true,
            sounds: true,
            multiscreen: true,
            pin_dropped_window: false,
        }
    }
}

/// set 単位の画像 scale（design §3-14 の `[imagesets] scale = { ... }`）。
/// Reload 時に [`load_materials`] へ注入する（[`Settings::scales`]）。
/// `Debug` は [`Settings`] の derive 連鎖（`expect_err` 表示）要件。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ImagesetsSettings {
    /// set 名 → scale（BTreeMap で save 出力を辞書順に固定）。
    pub scale: BTreeMap<String, f64>,
}

/// アクティブウィンドウ選別の whitelist / blacklist（design §3-14 の
/// `[interactive_windows]`）。Java `Settings` の `interactiveWindows` /
/// `interactiveWindowsBlacklist` 相当で、`Win32OsSource` の
/// `is_interactive_by_title` へ注入される。
///
/// 各項目は load 時に加工しない（trim・空要素除去・正規化なし・verbatim 保持）。
/// trim 後空判定や部分一致は下流の `is_interactive_by_title` が担う。
/// `#[serde(default)]` でセクション欠落・キー欠落の双方を空リストに補完する。
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct InteractiveWindowsSettings {
    /// 反応対象タイトルの部分一致リスト（空 = 非使用）。
    pub whitelist: Vec<String>,
    /// 反応除外タイトルの部分一致リスト（空 = 非使用）。
    pub blacklist: Vec<String>,
}

/// 一般設定（design §3-14 の `[general]`）。TOML 出力で先頭セクションに置くため
/// [`Settings`] の最初のフィールドに据える。欠落メンバは `#[serde(default)]` で
/// 補完（`show_console` 既定 false・`language` 既定 [`DEFAULT_LANGUAGE`]・後方互換）。
#[derive(Debug, Serialize, Deserialize)]
pub struct GeneralSettings {
    /// コンソール表示（既定 false。main が起動時の窓表示制御に参照）。
    #[serde(default)]
    pub show_console: bool,
    /// UI 言語コード（既定 [`DEFAULT_LANGUAGE`] = "en"。`conf/lang/<code>.toml` を選択）。
    #[serde(default = "default_language")]
    pub language: String,
}

/// `language` の serde 既定値（[`DEFAULT_LANGUAGE`] を共用・後方互換）。
fn default_language() -> String {
    DEFAULT_LANGUAGE.to_string()
}

impl Default for GeneralSettings {
    /// `show_console = false` / `language = `[`DEFAULT_LANGUAGE`]。
    fn default() -> GeneralSettings {
        GeneralSettings {
            show_console: false,
            language: default_language(),
        }
    }
}

/// settings.toml の強型（design §3-14 形状・Java `Settings` のうち Phase 1 が
/// 持つ部分のみ）。`#[serde(default)]` で欠落セクション / フィールドを既定補完・
/// 未知キーは無視（`deny_unknown_fields` は付けない・前方互換）。
/// `Debug` はテストの `expect_err`（Ok 側表示）要件。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Settings {
    /// 一般設定（先頭 = TOML 出力の `[general]` を最初に出す・design §3-14）。
    #[serde(default)]
    pub general: GeneralSettings,
    /// 6 トグル（既定 true・[`AllowedSettings::default`]）。
    #[serde(default)]
    pub allowed: AllowedSettings,
    /// set 名 → 無効 Behavior 名リスト（Java L31 `disabledBehaviors` 相当・
    /// design §3-14 の `[disabled_behaviors]`。BTreeMap で save 出力を辞書順に固定）。
    #[serde(default)]
    pub disabled_behaviors: BTreeMap<String, Vec<String>>,
    /// set 単位 scale（design §3-14 の `[imagesets] scale = { ... }`）。
    #[serde(default)]
    pub imagesets: ImagesetsSettings,
    /// アクティブウィンドウ選別の whitelist / blacklist
    /// （design §3-14 の `[interactive_windows]`・[`Win32OsSource`] へ注入）。
    /// 後方互換: セクション欠落は空リスト補完。
    ///
    /// [`Win32OsSource`]: crate::win::os_source::Win32OsSource
    #[serde(default)]
    pub interactive_windows: InteractiveWindowsSettings,
}

// =====================================================================
// settings.toml の説明コメント
// =====================================================================
// serde シリアライザはコメントを出力できないため、生成した TOML 本文へ手で挿入する。
// [`with_help`] を通した本文が「初回生成物」と「保存物」の共通形なので、
// 同梱テンプレート `conf/settings.default.toml` もこの形に一致させる。

/// `[general]` 見出しの直後へ添える説明。
const GENERAL_HELP: &str = "\
# --- [general] ---
# show_console : true にするとログ表示用のコンソールウィンドウを確保する（既定 false・通常は不要）
# language     : UI 文言の言語。同梱は \"en\"（英語・既定） / \"ja\"（日本語）。反映は次回起動時
# 記入例: language = \"ja\"
";

/// `[allowed]` 見出しの直後へ添える説明（トレイの Allowed Behaviours と同義）。
const ALLOWED_HELP: &str = "\
# --- [allowed] 許可する行為（トレイの Allowed Behaviours と同じ・true で許可）---
# トレイから切り替えると、このファイルへその場で保存される。
# breeding           : しめじを増やす動作（分裂）
# transients         : 特殊効果（一定時間で消える増殖個体の発生）
# transformation     : スキン変更、変身
# throwing           : ウィンドウを投げる行為
# sounds             : 効果音の許可
# multiscreen        : マルチモニターで複数の画面をまたいで動作するのを許可
# pin_dropped_window : ドロップしたウィンドウを最前面に固定（既定 false・Rust 版独自）
";

/// `[disabled_behaviors]` 見出しの直後へ添える説明と記入例。
const DISABLED_BEHAVIORS_HELP: &str = "\
# --- [disabled_behaviors] 特定の Behavior を止める（任意）---
# 形式は `set 名 = [\"Behavior 名\", ...]`。Behavior 名は conf/behaviors.xml の名前（英語表記）。
# 記入例: Shimeji = [\"SitDown\", \"SplitIntoTwo\"]
";

/// `[imagesets.scale]` 見出しの直後へ添える説明と記入例。
const IMAGESETS_SCALE_HELP: &str = "\
# --- [imagesets.scale] 画像セットごとの拡大率（任意）---
# 形式は `set 名 = 倍率`。既定は 1.0（等倍）。0 より大きい有限値のみ有効。
# 記入例: Shimeji = 0.5   # 解像度 2 倍の画像セットを 128px 相当で使う
";

/// `[interactive_windows]` の末尾へ添える説明と記入例。
///
/// `[interactive_windows]` は [`Settings`] の最後のフィールドなので、末尾追記で
/// 同セクションの説明として読める。各例はコメントアウトしてあり、
/// 有効化する場合は該当行の先頭 `#` を外す（部分一致・大文字小文字を区別）。
const INTERACTIVE_WINDOWS_HELP: &str = "\
# --- [interactive_windows] 記入例 ---
# whitelist = [\"メモ帳\", \"Visual Studio Code\"]   # タイトル部分一致で反応対象にする
# blacklist = [\"タスク マネージャー\"]              # 部分一致で除外する（whitelist より優先）
# 注意: whitelist と blacklist の両方が空のときは、どのウィンドウにも反応しません。
";

/// TOML 本文へ上記の説明コメントを挿入する（初回生成とトグル操作時の保存で共用）。
///
/// 見出し行の直後・`[interactive_windows]` は末尾へ入れるため、値の並びは変わらない
/// （コメントは TOML の解釈に影響しない）。保存のたびに同じ挿入を行うので、
/// トレイ操作で設定が上書き保存されても説明はファイルに残る。
fn with_help(mut text: String) -> String {
    for (header, help) in [
        ("[general]\n", GENERAL_HELP),
        ("[allowed]\n", ALLOWED_HELP),
        ("[disabled_behaviors]\n", DISABLED_BEHAVIORS_HELP),
        ("[imagesets.scale]\n", IMAGESETS_SCALE_HELP),
    ] {
        if let Some(pos) = text.find(header) {
            text.insert_str(pos + header.len(), help);
        }
    }
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(INTERACTIVE_WINDOWS_HELP);
    text
}

impl Settings {
    /// settings.toml を読み込む（Java Settings.java load L61-68 相当）。
    /// ファイル不在は `Ok(既定値)`（Java L62 `Files.isRegularFile` 逐語・
    /// 読み飛ばし = 既定適用）。パース失敗は [`SettingsError::Parse`]。
    /// 欠落セクション / フィールドは既定補完・未知キーは無視。
    /// `imagesets.scale` が非有限または 0 以下なら [`SettingsError::InvalidScale`]。
    pub fn load(path: &Path) -> Result<Settings, SettingsError> {
        // Java L62: if (Files.isRegularFile(path)) — 無ければ既定のまま
        if !path.is_file() {
            return Ok(Settings::default());
        }
        let text = std::fs::read_to_string(path)?;
        // 欠落補完は serde default・未知キーは serde が無視する
        let settings: Settings = toml::from_str(&text)?;
        settings.validate_scales()?;
        Ok(settings)
    }

    /// `imagesets.scale` の各値を検証する（異常入力の防御・タスク #20）。
    /// 非有限（NaN / ±inf）または 0 以下は設定不正として Err。
    /// 上限は設けない（巨大 scale はフレーム単位の寸法ガードで安全化）。
    fn validate_scales(&self) -> Result<(), SettingsError> {
        for (set, scale) in &self.imagesets.scale {
            if !scale.is_finite() || *scale <= 0.0 {
                return Err(SettingsError::InvalidScale(set.clone(), *scale));
            }
        }
        Ok(())
    }

    /// settings.toml を上書き保存する（Java Settings.java save L194-267 相当）。
    /// 出力は決定的（構造体フィールド順 + BTreeMap 辞書順）。
    /// [`with_help`] を通すため、説明コメントは保存のたびに付き直す。
    pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
        let text = with_help(toml::to_string(settings)?);
        std::fs::write(path, text)?;
        Ok(())
    }

    /// settings.toml が無ければ既定値で新規生成する（初回起動時の土台作成）。
    /// - 不在 → [`Settings::default`] の TOML に [`with_help`] の説明コメントを添えて
    ///   生成し `Ok(true)`（同梱テンプレート `conf/settings.default.toml` と同一内容）
    /// - 既存 → 何もせず `Ok(false)`（手編集を上書きしない）
    /// - 書込失敗 → エラーをそのまま伝播
    pub fn create_default_if_missing(path: &Path) -> Result<bool, SettingsError> {
        if path.is_file() {
            return Ok(false);
        }
        let text = with_help(toml::to_string(&Settings::default())?);
        std::fs::write(path, text)?;
        Ok(true)
    }

    /// set 単位 scale の参照（[`load_materials`] 注入用・design §3-14）。
    pub fn scales(&self) -> &BTreeMap<String, f64> {
        &self.imagesets.scale
    }
}

// =====================================================================
// 契約 C: メニュー構築 + コマンド（design §3-11）
// =====================================================================

/// AllowedKind → [`AllowedSettings`] フィールド値の対応
///（build / sync / apply の 3 経路で共有する対応表）。
fn allowed_value(allowed: &AllowedSettings, kind: AllowedKind) -> bool {
    match kind {
        AllowedKind::Breeding => allowed.breeding,
        AllowedKind::Transients => allowed.transients,
        AllowedKind::Transformation => allowed.transformation,
        AllowedKind::Throwing => allowed.throwing,
        AllowedKind::Sounds => allowed.sounds,
        AllowedKind::Multiscreen => allowed.multiscreen,
        AllowedKind::PinDroppedWindow => allowed.pin_dropped_window,
    }
}

/// Allowed Behaviours サブメニューのラベル順（design §3-11・増殖 / 変身 / 投げ /
/// 画面間移動 / 効果音 / Transients / ドロップ窓固定）と [`AllowedKind`] /
/// 辞書キーの対応。
const ALLOWED_MENU_ITEMS: [(AllowedKind, UiKey); 7] = [
    (AllowedKind::Breeding, UiKey::BreedingCloning),
    (AllowedKind::Transformation, UiKey::Transformation),
    (AllowedKind::Throwing, UiKey::ThrowingWindows),
    (AllowedKind::Multiscreen, UiKey::Multiscreen),
    (AllowedKind::Sounds, UiKey::SoundEffects),
    (AllowedKind::Transients, UiKey::BreedingTransient),
    (AllowedKind::PinDroppedWindow, UiKey::PinDroppedWindow),
];

/// トレイ / ポップアップのメニュー項目に対応するコマンド（design §3-11）。
/// MenuEvent → コマンド変換は [`TrayMenuModel::command_of`]・適用は
/// [`apply_tray_command`]（#10 がこの 2 段をイベントループで呼ぶ）。
#[derive(Clone)]
pub enum TrayCommand {
    /// 呼ぶ。`None` = set ランダム選択（「（ランダム）」）/ `Some(set)` = set 指定。
    Spawn(Option<String>),
    /// Follow Cursor（全員に ChaseMouse 指示）。
    FollowCursor,
    /// Reduce to One（先頭 1 体を残す）。
    ReduceToOne,
    /// Restore Windows（画面外の窓を作業領域へ戻す）。
    RestoreWindows,
    /// Allowed Behaviours トグル。bool は**適用値** = MenuEvent 受信時の
    /// チェック状態。muda 0.19.3 は CheckMenuItem クリック時に MenuEvent
    /// 発行**前**にチェック状態を自動トグルする（`platform_impl/windows/mod.rs`
    /// L1195-1198 実物照合）ため、受信時点の `is_checked()` は既に
    /// 「クリック後の新値」。[`TrayMenuModel::command_of`] はその値を
    /// そのまま返す（否定しない）・トグル適用後の整合は
    /// [`TrayMenuModel::sync_allowed`] が担う。
    SetAllowed(AllowedKind, bool),
    /// 一時停止 / 再開（全員）。
    TogglePauseAll,
    /// Dismiss All（全員消去・exit は次 tick の [`Manager::should_exit`] 経由）。
    DismissAll,
    /// Reload（素材再ロード + 参照付け替え）。
    Reload,
    /// 個別行動指定（popup・`index` はメニュー構築時の mascot index）。
    SetBehaviorFor(usize, String),
    /// 個別一時停止 / 再開（popup）。
    TogglePauseFor(usize),
    /// 個別消す（popup）。
    DismissFor(usize),
}

/// [`apply_tray_command`] への文脈（パスと set 一覧・#10 が保持する）。
pub struct TrayContext {
    /// conf ディレクトリ（settings.toml の保存先・Reload の XML 読み出し元）。
    pub conf_dir: PathBuf,
    /// img ディレクトリ（Reload の画像読み出し元）。
    pub img_dir: PathBuf,
    /// 有効画像 set 一覧（「（ランダム）」spawn の選択元）。
    pub image_sets: Vec<String>,
}

/// トレイ / ポップアップ メニューのモデル（muda `Menu` とコマンド対応表の所有者）。
/// 1 メニュー = 1 モデルで、トレイ用（[`TrayMenuModel::build_tray`]）と
/// マスコット ポップアップ用（[`TrayMenuModel::build_popup`]）を構築時に選ぶ。
pub struct TrayMenuModel {
    /// 所有するメニュー（#10 が `TrayIconBuilder::with_menu` / popup 表示へ渡す）。
    menu: Menu,
    /// MenuId → 固定コマンド対応表。SetAllowed は checked 状態依存のため
    /// `allowed_checks` から動的に算出する（ここには載せない）。
    commands: HashMap<MenuId, TrayCommand>,
    /// Allowed Behaviours の 6 CheckMenuItem（`command_of` の動的変換と
    /// [`TrayMenuModel::sync_allowed`] 用。CheckMenuItem は Rc 共有 Clone）。
    allowed_checks: Vec<(AllowedKind, CheckMenuItem)>,
}

/// 「呼ぶ」サブメニュー（design §3-11 の先頭・tray / popup 共通構成）:
/// 先頭 [`UiKey::SpawnRandom`] = [`TrayCommand::Spawn`]`(None)` + 各 set 名 =
/// `Spawn(Some(set))`（`image_sets` の順で並べる）。サブメニュー名は `label`
///（tray = `CallShimeji` / popup = `CallAnother`）を辞書で解決する。
fn build_spawn_submenu(
    image_sets: &[String],
    commands: &mut HashMap<MenuId, TrayCommand>,
    lang: &Lang,
    label: UiKey,
) -> Submenu {
    let submenu = Submenu::new(lang.text(label), true);
    let random = MenuItem::new(lang.text(UiKey::SpawnRandom), true, None);
    commands.insert(random.id().clone(), TrayCommand::Spawn(None));
    submenu
        .append(&random)
        .expect("failed to append the (random) menu item");
    for image_set in image_sets {
        let item = MenuItem::new(image_set, true, None);
        commands.insert(
            item.id().clone(),
            TrayCommand::Spawn(Some(image_set.clone())),
        );
        submenu
            .append(&item)
            .expect("failed to append a call set menu item");
    }
    submenu
}

impl TrayMenuModel {
    /// トレイメニューを構築する（design §3-11 の 1〜10 の挿入順）:
    ///
    /// 1. Submenu「呼ぶ」
    /// 2. Follow Cursor
    /// 3. Reduce to One
    /// 4. Restore Windows
    /// 5. Submenu「Allowed Behaviours」（6 CheckMenuItem・ラベル順
    ///    [`ALLOWED_MENU_ITEMS`]・checked = `allowed` の対応値）
    /// 6. separator
    /// 7. 一時停止
    /// 8. Dismiss All
    /// 9. separator
    /// 10. Reload
    pub fn build_tray(
        image_sets: &[String],
        allowed: &AllowedSettings,
        lang: &Lang,
    ) -> TrayMenuModel {
        let menu = Menu::new();
        let mut commands = HashMap::new();

        // 1. 呼ぶ
        let spawn = build_spawn_submenu(image_sets, &mut commands, lang, UiKey::CallShimeji);
        // 2-4. Follow Cursor / Reduce to One / Restore Windows
        let follow = MenuItem::new(lang.text(UiKey::FollowCursor), true, None);
        commands.insert(follow.id().clone(), TrayCommand::FollowCursor);
        let reduce = MenuItem::new(lang.text(UiKey::ReduceToOne), true, None);
        commands.insert(reduce.id().clone(), TrayCommand::ReduceToOne);
        let restore = MenuItem::new(lang.text(UiKey::RestoreWindows), true, None);
        commands.insert(restore.id().clone(), TrayCommand::RestoreWindows);
        // 5. Allowed Behaviours（6 CheckMenuItem・checked = allowed の対応値）
        let allowed_menu = Submenu::new(lang.text(UiKey::AllowedBehaviours), true);
        let mut allowed_checks = Vec::with_capacity(ALLOWED_MENU_ITEMS.len());
        for (kind, key) in ALLOWED_MENU_ITEMS {
            let check =
                CheckMenuItem::new(lang.text(key), true, allowed_value(allowed, kind), None);
            allowed_checks.push((kind, check.clone()));
            allowed_menu
                .append(&check)
                .expect("failed to append a toggle menu item");
        }
        // 6 / 9. separator
        let separator1 = PredefinedMenuItem::separator();
        let separator2 = PredefinedMenuItem::separator();
        // 7-8. 一時停止 / Dismiss All
        let pause = MenuItem::new(lang.text(UiKey::PauseAnimations), true, None);
        commands.insert(pause.id().clone(), TrayCommand::TogglePauseAll);
        let dismiss = MenuItem::new(lang.text(UiKey::DismissAll), true, None);
        commands.insert(dismiss.id().clone(), TrayCommand::DismissAll);
        // 10. Reload
        let reload = MenuItem::new(lang.text(UiKey::Reload), true, None);
        commands.insert(reload.id().clone(), TrayCommand::Reload);

        menu.append_items(&[
            &spawn,
            &follow,
            &reduce,
            &restore,
            &allowed_menu,
            &separator1,
            &pause,
            &dismiss,
            &separator2,
            &reload,
        ])
        .expect("failed to build the tray menu");

        TrayMenuModel {
            menu,
            commands,
            allowed_checks,
        }
    }

    /// マスコット右クリック メニュー（design §3-11 最小セット）を構築する:
    /// ① Submenu「呼ぶ」（トレイと同構成）② Submenu「個別行動指定」
    ///（[`BehaviorMenu::selectable`] の各行動 = toggleable 専用項目は含めない・
    /// Java popup setBehaviorMenu L517-522 相当）③ separator
    /// ④ 一時停止 / 再開（`is_paused` 由来のラベル切替・Java popup pauseItem
    /// L559 相当）⑤ 消す（Java popup disposeMenu L562 相当）。
    /// `selectable` が空でも「個別行動指定」サブメニュー自体は作る。
    pub fn build_popup(
        index: usize,
        image_sets: &[String],
        menu_items: &BehaviorMenu,
        is_paused: bool,
        lang: &Lang,
    ) -> TrayMenuModel {
        let menu = Menu::new();
        let mut commands = HashMap::new();

        // ① 呼ぶ
        let spawn = build_spawn_submenu(image_sets, &mut commands, lang, UiKey::CallAnother);
        // ② 個別行動指定（selectable のみ）
        let behavior = Submenu::new(lang.text(UiKey::SetBehaviour), true);
        for name in &menu_items.selectable {
            let item = MenuItem::new(lang.behavior_text(name), true, None);
            commands.insert(
                item.id().clone(),
                TrayCommand::SetBehaviorFor(index, name.clone()),
            );
            behavior
                .append(&item)
                .expect("failed to append a behavior menu item");
        }
        // ③ separator
        let separator = PredefinedMenuItem::separator();
        // ④ 一時停止 / 再開（Java L559: isPaused ? Resume : Pause のラベル切替）
        let pause_key = if is_paused {
            UiKey::ResumeAnimations
        } else {
            UiKey::PauseAnimations
        };
        let pause = MenuItem::new(lang.text(pause_key), true, None);
        commands.insert(pause.id().clone(), TrayCommand::TogglePauseFor(index));
        // ⑤ 消す
        let dismiss = MenuItem::new(lang.text(UiKey::Dismiss), true, None);
        commands.insert(dismiss.id().clone(), TrayCommand::DismissFor(index));

        menu.append_items(&[&spawn, &behavior, &separator, &pause, &dismiss])
            .expect("failed to build the popup menu");

        TrayMenuModel {
            menu,
            commands,
            allowed_checks: Vec::new(),
        }
    }

    /// 所有するメニュー（#10 が TrayIcon / popup 表示へ渡す）。
    pub fn menu(&self) -> &Menu {
        &self.menu
    }

    /// MenuId → コマンド。Allowed Behaviours のトグル id は現在の checked
    ///（= 適用値）を [`TrayCommand::SetAllowed`] に焼き込んで返す。
    ///
    /// 根拠（muda 0.19.3 実物照合・`platform_impl/windows/mod.rs` L1195-1198）:
    /// muda は CheckMenuItem クリック時に MenuEvent 発行**前**にチェック状態を
    /// 自動トグルするため、MenuEvent 受信時点の `is_checked()` は既に
    /// 「クリック後の新値」である。よって [`TrayMenuModel::command_of`] は
    /// その値をそのまま返す（否定しない）。トグルコマンド適用後は
    /// [`TrayMenuModel::sync_allowed`] が muda の状態と settings を冪等に
    /// 整合させる。未知 id は `None`。
    pub fn command_of(&self, id: &MenuId) -> Option<TrayCommand> {
        for (kind, check) in &self.allowed_checks {
            if check.id() == id {
                return Some(TrayCommand::SetAllowed(*kind, check.is_checked()));
            }
        }
        self.commands.get(id).cloned()
    }

    /// 6 CheckMenuItem の checked を `allowed` に同期する（MenuEvent 受信後に
    /// #10 が呼ぶ想定・[`TrayCommand::SetAllowed`] 適用後の UI 反映）。
    /// [`TrayMenuModel::command_of`] は同期後の現在の checked（= 適用値）を
    /// 返す（sync 前後で意味論は不変・muda 自動トグルは実 UI クリック時のみ
    /// 発動するため sync による set_checked では発動しない）。
    pub fn sync_allowed(&self, allowed: &AllowedSettings) {
        for (kind, check) in &self.allowed_checks {
            check.set_checked(allowed_value(allowed, *kind));
        }
    }
}

/// [`TrayCommand::SetAllowed`] の settings 部分適用（6 トグル・Java Settings
/// L89-94 のフィールド対応）。
fn apply_allowed(settings: &mut Settings, kind: AllowedKind, value: bool) {
    match kind {
        AllowedKind::Breeding => settings.allowed.breeding = value,
        AllowedKind::Transients => settings.allowed.transients = value,
        AllowedKind::Transformation => settings.allowed.transformation = value,
        AllowedKind::Throwing => settings.allowed.throwing = value,
        AllowedKind::Sounds => settings.allowed.sounds = value,
        AllowedKind::Multiscreen => settings.allowed.multiscreen = value,
        AllowedKind::PinDroppedWindow => settings.allowed.pin_dropped_window = value,
    }
}

/// トレイ / ポップアップ コマンドを適用する（#10 が MenuEvent 受信後に呼ぶ）。
/// design §3-11 の適用先対応:
///
/// - Spawn: [`Manager::request_spawn`]（`None` は [`Manager::request_spawn_random`]）
/// - FollowCursor: 全員 ChaseMouse / ReduceToOne: [`Manager::remain_one`] /
///   RestoreWindows: [`Manager::restore_windows`]
/// - SetAllowed: settings 更新 → `conf_dir/settings.toml` への**即時保存**
///   （Err → log で続行・panic しない）→ Environment passthrough（`Sounds` は #36 で
///   `Manager::set_sounds_enabled` へ実配線）
/// - TogglePauseAll / DismissAll: 全員操作・exit は次 tick の
///   [`Manager::should_exit`] 経由（apply 内では exit 操作しない）
/// - Reload: [`load_materials`]（scales は [`Settings::scales`] 注入）→
///   [`Manager::reload`]・Err → log + 現状維持（manager 不変）
/// - 個別 3 種: [`Manager::set_behavior_at`] / [`Manager::toggle_pause_at`] /
///   [`Manager::dismiss_at`]
pub fn apply_tray_command(
    manager: &mut Manager,
    settings: &mut Settings,
    command: TrayCommand,
    context: &TrayContext,
) {
    match command {
        TrayCommand::Spawn(Some(image_set)) => manager.request_spawn(&image_set),
        TrayCommand::Spawn(None) => manager.request_spawn_random(&context.image_sets),
        TrayCommand::FollowCursor => manager.set_behavior_all("ChaseMouse"),
        TrayCommand::ReduceToOne => manager.remain_one(),
        TrayCommand::RestoreWindows => {
            manager.restore_windows();
            // #30 item 5: 復元時に pin を解除する（我々が付けた TOPMOST を剥がす）。
            manager.unpin_pinned_window();
        }
        TrayCommand::SetAllowed(kind, value) => {
            apply_allowed(settings, kind, value);
            // design §3-11: トグル状態は conf/settings.toml に即時永続化。
            // 保存失敗でもトグル適用は続行する（panic しない）
            if let Err(err) = Settings::save(&context.conf_dir.join("settings.toml"), settings) {
                log::error!("failed to save settings.toml: {err}");
            }
            match kind {
                AllowedKind::Breeding => manager.set_breeding_allowed(value),
                AllowedKind::Transients => manager.set_transients_enabled(value),
                AllowedKind::Transformation => manager.set_transformation_allowed(value),
                AllowedKind::Throwing => manager.set_throwing_allowed(value),
                AllowedKind::Multiscreen => manager.set_multiscreen(value),
                // 効果音（#36: settings 永続化 + Environment の再生バックエンドへ反映）
                AllowedKind::Sounds => manager.set_sounds_enabled(value),
                // ドロップ窓固定（機能 #30 item 1/5）: OFF 時は Manager が即 unpin する。
                AllowedKind::PinDroppedWindow => manager.set_pin_dropped_window_allowed(value),
            }
        }
        TrayCommand::TogglePauseAll => manager.toggle_pause_all(),
        TrayCommand::DismissAll => {
            manager.dispose_all();
            // #30 item 5: 終了時に pin を解除する（mascot 削除は次 tick でも解除は即座）。
            manager.unpin_pinned_window();
        }
        TrayCommand::Reload => {
            // scales は BTreeMap（Settings 契約）→ load_materials は HashMap 入力のため変換
            let scales: HashMap<String, f64> = settings
                .scales()
                .iter()
                .map(|(set, scale)| (set.clone(), *scale))
                .collect();
            match load_materials(&context.conf_dir, &context.img_dir, &scales) {
                Ok(materials) => manager.reload(materials),
                Err(err) => log::error!("reload failed; keeping current state: {err}"),
            }
        }
        TrayCommand::SetBehaviorFor(index, name) => manager.set_behavior_at(index, &name),
        TrayCommand::TogglePauseFor(index) => manager.toggle_pause_at(index),
        TrayCommand::DismissFor(index) => manager.dismiss_at(index),
    }
}
