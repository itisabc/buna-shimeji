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

// =====================================================================
// 契約 A: 設定永続化（conf/settings.toml・design §3-14 形状）
// =====================================================================

/// settings.toml の入出力エラー（パース / シリアライズ / io の 3 経路）。
#[derive(Debug, Error)]
pub enum SettingsError {
    /// settings.toml の読み込み（パース）に失敗（Java L63-67 の IOException 相当）。
    #[error("settings.toml の読み込みに失敗しました: {0}")]
    Parse(#[from] toml::de::Error),
    /// settings.toml の書き出し（シリアライズ）に失敗。
    #[error("settings.toml の書き出しに失敗しました: {0}")]
    Serialize(#[from] toml::ser::Error),
    /// settings.toml の読み書き（io）に失敗（保存先ディレクトリ不在等）。
    #[error("settings.toml の入出力に失敗しました: {0}")]
    Io(#[from] std::io::Error),
    /// imagesets.scale が非有限（NaN / ±inf）または 0 以下。
    /// 設定不正として起動エラー経路に載せる（上限は設けない）。
    #[error(
        "settings.toml の imagesets.scale が不正です（{0} = {1}）: 正の有限値を指定してください"
    )]
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
    /// 効果音枠（Java `Sounds`・Phase 1 は永続化のみ）。
    Sounds,
    /// 画面間移動（Java `Multiscreen`）。
    Multiscreen,
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
    /// Java L93 `Sounds`（効果音枠・Phase 1 は settings 永続化のみ）。
    pub sounds: bool,
    /// Java L94 `Multiscreen`（画面間移動）。
    pub multiscreen: bool,
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

/// settings.toml の強型（design §3-14 形状・Java `Settings` のうち Phase 1 が
/// 持つ部分のみ）。`#[serde(default)]` で欠落セクション / フィールドを既定補完・
/// 未知キーは無視（`deny_unknown_fields` は付けない・前方互換）。
/// `Debug` はテストの `expect_err`（Ok 側表示）要件。
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Settings {
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
    pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
        let text = toml::to_string(settings)?;
        std::fs::write(path, text)?;
        Ok(())
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
    }
}

/// Allowed Behaviours サブメニューのラベル順（design §3-11・増殖 / 変身 / 投げ /
/// 画面間移動 / 効果音枠 / Transients）と [`AllowedKind`] の対応。
const ALLOWED_MENU_ITEMS: [(AllowedKind, &str); 6] = [
    (AllowedKind::Breeding, "増殖"),
    (AllowedKind::Transformation, "変身"),
    (AllowedKind::Throwing, "投げ"),
    (AllowedKind::Multiscreen, "画面間移動"),
    (AllowedKind::Sounds, "効果音枠"),
    (AllowedKind::Transients, "Transients"),
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
/// 先頭「（ランダム）」= [`TrayCommand::Spawn`]`(None)` + 各 set 名 =
/// `Spawn(Some(set))`（`image_sets` の順で並べる）。
fn build_spawn_submenu(
    image_sets: &[String],
    commands: &mut HashMap<MenuId, TrayCommand>,
) -> Submenu {
    let submenu = Submenu::new("呼ぶ", true);
    let random = MenuItem::new("（ランダム）", true, None);
    commands.insert(random.id().clone(), TrayCommand::Spawn(None));
    submenu
        .append(&random)
        .expect("「（ランダム）」のメニュー追加に失敗しました");
    for image_set in image_sets {
        let item = MenuItem::new(image_set, true, None);
        commands.insert(
            item.id().clone(),
            TrayCommand::Spawn(Some(image_set.clone())),
        );
        submenu
            .append(&item)
            .expect("「呼ぶ」set 項目のメニュー追加に失敗しました");
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
    pub fn build_tray(image_sets: &[String], allowed: &AllowedSettings) -> TrayMenuModel {
        let menu = Menu::new();
        let mut commands = HashMap::new();

        // 1. 呼ぶ
        let spawn = build_spawn_submenu(image_sets, &mut commands);
        // 2-4. Follow Cursor / Reduce to One / Restore Windows
        let follow = MenuItem::new("Follow Cursor", true, None);
        commands.insert(follow.id().clone(), TrayCommand::FollowCursor);
        let reduce = MenuItem::new("Reduce to One", true, None);
        commands.insert(reduce.id().clone(), TrayCommand::ReduceToOne);
        let restore = MenuItem::new("Restore Windows", true, None);
        commands.insert(restore.id().clone(), TrayCommand::RestoreWindows);
        // 5. Allowed Behaviours（6 CheckMenuItem・checked = allowed の対応値）
        let allowed_menu = Submenu::new("Allowed Behaviours", true);
        let mut allowed_checks = Vec::with_capacity(ALLOWED_MENU_ITEMS.len());
        for (kind, label) in ALLOWED_MENU_ITEMS {
            let check = CheckMenuItem::new(label, true, allowed_value(allowed, kind), None);
            allowed_checks.push((kind, check.clone()));
            allowed_menu
                .append(&check)
                .expect("トグル項目のメニュー追加に失敗しました");
        }
        // 6 / 9. separator
        let separator1 = PredefinedMenuItem::separator();
        let separator2 = PredefinedMenuItem::separator();
        // 7-8. 一時停止 / Dismiss All
        let pause = MenuItem::new("一時停止", true, None);
        commands.insert(pause.id().clone(), TrayCommand::TogglePauseAll);
        let dismiss = MenuItem::new("Dismiss All", true, None);
        commands.insert(dismiss.id().clone(), TrayCommand::DismissAll);
        // 10. Reload
        let reload = MenuItem::new("Reload", true, None);
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
        .expect("トレイメニューの構築に失敗しました");

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
    ) -> TrayMenuModel {
        let menu = Menu::new();
        let mut commands = HashMap::new();

        // ① 呼ぶ
        let spawn = build_spawn_submenu(image_sets, &mut commands);
        // ② 個別行動指定（selectable のみ）
        let behavior = Submenu::new("個別行動指定", true);
        for name in &menu_items.selectable {
            let item = MenuItem::new(name, true, None);
            commands.insert(
                item.id().clone(),
                TrayCommand::SetBehaviorFor(index, name.clone()),
            );
            behavior
                .append(&item)
                .expect("行動項目のメニュー追加に失敗しました");
        }
        // ③ separator
        let separator = PredefinedMenuItem::separator();
        // ④ 一時停止 / 再開（Java L559: isPaused ? Resume : Pause のラベル切替）
        let pause_label = if is_paused { "再開" } else { "一時停止" };
        let pause = MenuItem::new(pause_label, true, None);
        commands.insert(pause.id().clone(), TrayCommand::TogglePauseFor(index));
        // ⑤ 消す
        let dismiss = MenuItem::new("消す", true, None);
        commands.insert(dismiss.id().clone(), TrayCommand::DismissFor(index));

        menu.append_items(&[&spawn, &behavior, &separator, &pause, &dismiss])
            .expect("ポップアップメニューの構築に失敗しました");

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
    }
}

/// トレイ / ポップアップ コマンドを適用する（#10 が MenuEvent 受信後に呼ぶ）。
/// design §3-11 の適用先対応:
///
/// - Spawn: [`Manager::request_spawn`]（`None` は [`Manager::request_spawn_random`]）
/// - FollowCursor: 全員 ChaseMouse / ReduceToOne: [`Manager::remain_one`] /
///   RestoreWindows: [`Manager::restore_windows`]
/// - SetAllowed: settings 更新 → `conf_dir/settings.toml` への**即時保存**
///   （Err → log で続行・panic しない）→ Environment passthrough。`Sounds` は
///   Phase 1 no-op（design §3-12）のため settings 更新 + 保存のみ
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
        TrayCommand::RestoreWindows => manager.restore_windows(),
        TrayCommand::SetAllowed(kind, value) => {
            apply_allowed(settings, kind, value);
            // design §3-11: トグル状態は conf/settings.toml に即時永続化。
            // 保存失敗でもトグル適用は続行する（panic しない）
            if let Err(err) = Settings::save(&context.conf_dir.join("settings.toml"), settings) {
                log::error!("settings.toml の保存に失敗しました: {err}");
            }
            match kind {
                AllowedKind::Breeding => manager.set_breeding_allowed(value),
                AllowedKind::Transients => manager.set_transients_enabled(value),
                AllowedKind::Transformation => manager.set_transformation_allowed(value),
                AllowedKind::Throwing => manager.set_throwing_allowed(value),
                AllowedKind::Multiscreen => manager.set_multiscreen(value),
                // 効果音は Phase 1 no-op（design §3-12・settings 永続化のみ）
                AllowedKind::Sounds => {}
            }
        }
        TrayCommand::TogglePauseAll => manager.toggle_pause_all(),
        TrayCommand::DismissAll => manager.dispose_all(),
        TrayCommand::Reload => {
            // scales は BTreeMap（Settings 契約）→ load_materials は HashMap 入力のため変換
            let scales: HashMap<String, f64> = settings
                .scales()
                .iter()
                .map(|(set, scale)| (set.clone(), *scale))
                .collect();
            match load_materials(&context.conf_dir, &context.img_dir, &scales) {
                Ok(materials) => manager.reload(materials),
                Err(err) => log::error!("Reload に失敗したため現状を維持します: {err}"),
            }
        }
        TrayCommand::SetBehaviorFor(index, name) => manager.set_behavior_at(index, &name),
        TrayCommand::TogglePauseFor(index) => manager.toggle_pause_at(index),
        TrayCommand::DismissFor(index) => manager.dismiss_at(index),
    }
}
