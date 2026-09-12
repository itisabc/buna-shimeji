//! タスク #9c: トレイ + settings.toml 永続化の契約テスト（実装済み・GREEN）。
//!
//! 設計正本: design.md §3 L528-548（トレイ項目一式・settings.toml の scale
//! 形状・Reload・§1.10 (d) 9c の決定記録は orch コミット時に追記）。
//! Java 正本: .tmp/java-ref/Settings.java
//! （6 トグル既定 true L89-94・disabledBehaviors Map<set,List>・load L61-137 /
//! save L194-267・ファイル不在時は既定）・Main.java createMascot L466-505
//! （無引数版 = ランダム set `(int)(length*Math.random())`・0 set で no-op）・
//! Mascot.java popup L478-602（pauseItem L559・Dismiss L562・SetBehaviour L517-522）。
//!
//! SetAllowed の値の意味論（muda 0.19.3 実物照合・
//! platform_impl/windows/mod.rs L1195-1198）: muda は CheckMenuItem クリック時に
//! MenuEvent 発行**前**にチェック状態を自動トグルするため、MenuEvent 受信時の
//! `is_checked()` は既に「クリック後の新値」。`command_of` はこの値をそのまま
//! SetAllowed の適用値として返す（否定しない）・トグルコマンド適用後は
//! `sync_allowed` が muda の状態と settings を冪等に整合させる。
//! テストはこの新値（= 現在の checked そのまま）を pin する。
//! build 直後は実 UI クリックが無いため checked = allowed 初期値のまま不変。
//!
//! pin する API 契約（coder への指示・シグネチャは tests が固定する）:
//!
//! ```text
//! // ---- src/tray.rs ----
//! pub enum AllowedKind { Breeding, Transients, Transformation, Throwing, Sounds, Multiscreen }
//!
//! // 6 フィールド bool・全て既定 true（Java Settings.java L89-94）
//! pub struct AllowedSettings {
//!     pub breeding: bool, pub transients: bool, pub transformation: bool,
//!     pub throwing: bool, pub sounds: bool, pub multiscreen: bool,
//! }
//!
//! pub struct ImagesetsSettings { pub scale: BTreeMap<String, f64> }
//!
//! pub struct Settings {
//!     pub allowed: AllowedSettings,
//!     pub disabled_behaviors: BTreeMap<String, Vec<String>>,
//!     pub imagesets: ImagesetsSettings,
//! }
//! impl Settings {
//!     // ファイル不在 → Ok(既定)・欠落セクション/フィールド → 既定・未知キーは無視
//!     pub fn load(path: &Path) -> Result<Settings, SettingsError>
//!     // 上書き保存
//!     pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError>
//!     // load_materials の第 3 引数注入用
//!     pub fn scales(&self) -> &BTreeMap<String, f64>
//! }
//! // thiserror・3 経路（toml パース / toml シリアライズ / io）・Display は英語
//! pub enum SettingsError { /* ... */ }
//!
//! pub enum TrayCommand {
//!     Spawn(Option<String>), FollowCursor, ReduceToOne, RestoreWindows,
//!     SetAllowed(AllowedKind, bool), TogglePauseAll, DismissAll, Reload,
//!     SetBehaviorFor(usize, String), TogglePauseFor(usize), DismissFor(usize),
//! }
//!
//! pub struct TrayContext { pub conf_dir: PathBuf, pub img_dir: PathBuf, pub image_sets: Vec<String> }
//!
//! pub struct TrayMenuModel { /* 非公開フィールド */ }
//! impl TrayMenuModel {
//!     pub fn build_tray(image_sets: &[String], allowed: &AllowedSettings, lang: &Lang) -> TrayMenuModel
//!     pub fn build_popup(index: usize, image_sets: &[String], menu_items: &BehaviorMenu, is_paused: bool, lang: &Lang) -> TrayMenuModel
//!     pub fn menu(&self) -> &tray_icon::menu::Menu
//!     pub fn command_of(&self, id: &MenuId) -> Option<TrayCommand>
//!     pub fn sync_allowed(&self, allowed: &AllowedSettings)   // 全 CheckMenuItem へ set_checked
//! }
//!
//! pub fn apply_tray_command(manager: &mut Manager, settings: &mut Settings, command: TrayCommand, context: &TrayContext)
//!
//! // ---- src/app/manager.rs 追加 ----
//! impl Manager {
//!     // 空スライス → log::warn + no-op + rng 不消費 / 非空 → rng.unit() を選択に
//!     // ちょうど 1 回消費し idx = (unit * len) as usize（Java (int) 切り捨て逐語）で
//!     // set を選び request_spawn(その set)（look_right の 1 回は request_spawn 内）
//!     pub fn request_spawn_random(&mut self, image_sets: &[String])
//!     pub fn set_breeding_allowed(&mut self, allowed: bool)
//!     pub fn set_transients_enabled(&mut self, enabled: bool)
//!     pub fn set_transformation_allowed(&mut self, allowed: bool)
//!     pub fn set_throwing_allowed(&mut self, allowed: bool)
//!     pub fn set_multiscreen(&mut self, multiscreen: bool)
//!     // 単一マスコット版 setBehavior（Java popup L517-522 相当）。
//!     // その mascot の set の table で build_behavior → set_behavior・
//!     // 構築/実行 Err → log + その mascot のみ dispose（削除は次 tick）・
//!     // index 範囲外 → log::warn + no-op
//!     pub fn set_behavior_at(&mut self, index: usize, name: &str)
//!     // Java popup pauseItem L559 相当（set_paused(!is_paused)）・範囲外 → warn no-op
//!     pub fn toggle_pause_at(&mut self, index: usize)
//!     // Java popup Dismiss L562 相当（dispose・remove_pending・削除は次 tick）・
//!     // 範囲外 → warn no-op
//!     pub fn dismiss_at(&mut self, index: usize)
//! }
//!
//! // ---- src/app/environment.rs 追加 ----
//! impl Environment {
//!     // 全体置換（既存 map を clear して注入内容で再構築・settings.toml 復元注入用）。
//!     // 直後に behavior_disabled 読みへ反映される
//!     pub fn set_disabled_behaviors(&mut self, disabled: BTreeMap<String, Vec<String>>)
//! }
//! ```
//!
//! テストでは TrayIcon は作らない（タスクトレイ汚染防止）。muda `Menu`
//! オブジェクトの構築のみテストする（tray-icon 経由の re-export
//! `tray_icon::menu`）。自己完結（tests/common 不使用）。
//! app_manager_ext_test.rs の deterministic Rng fake / FakeSource /
//! ScriptedFactory パターンと、app_reload_test.rs の temp dir + 実 XML + PNG
//! 生成パターンを踏襲する。

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use simeji::app::environment::{Environment, OsSource};
use simeji::app::manager::{BehaviorMenu, Manager};
use simeji::config::{BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap};
use simeji::i18n::Lang;
use simeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use simeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use simeji::render::imageset::{Frame, ImageSet};
use simeji::tray::{
    apply_tray_command, AllowedKind, AllowedSettings, GeneralSettings, ImagesetsSettings,
    InteractiveWindowsSettings, Settings, SettingsError, TrayCommand, TrayContext, TrayMenuModel,
};
use tray_icon::menu::{CheckMenuItem, MenuId, MenuItemKind, Submenu};

// =====================================================================
// 合成データヘルパ（自己完結・app_reload_test.rs 踏襲）
// =====================================================================

/// 出荷辞書 `conf/lang/ja.toml` を読んだ `Lang`（tray/popup のラベル期待値の正本）。
fn ja_lang() -> Lang {
    Lang::load(
        &Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("conf")
            .join("lang"),
        "ja",
    )
}

/// 矩形ヘルパ。
fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

fn empty_image_set(name: &str) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

/// 単一フレームの ImageSet（フレーム寸法で新旧 Arc を判別する）。
fn image_set_with(name: &str, file: &str, width: u32, height: u32) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::from([(
            file.to_string(),
            Frame {
                width,
                height,
                rgba: vec![0; width as usize * height as usize * 4],
            },
        )]),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

/// 名前から Arc<ImageSet> を引く resolver（固定 map・テスト用）。
fn resolver_for(names: &[&str]) -> impl FnMut(&str) -> Option<Arc<ImageSet>> {
    let mut map = HashMap::new();
    for name in names {
        map.insert(name.to_string(), empty_image_set(name));
    }
    move |requested: &str| map.get(requested).cloned()
}

// =====================================================================
// Environment — fake source（app_manager_ext_test.rs 踏襲）
// =====================================================================

/// OS 供給の状態。
#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, Rect)>,
    windows: Vec<(i64, Rect)>,
    moved: Vec<(i64, i32, i32)>,
    raised: Vec<i64>,
}

struct FakeSource {
    state: Rc<RefCell<FakeState>>,
}

impl OsSource for FakeSource {
    fn monitors(&self) -> Vec<(Rect, Rect)> {
        self.state.borrow().monitors.clone()
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        self.state.borrow().cursor
    }

    fn active_window(&self) -> Option<(i64, Rect)> {
        self.state.borrow().active_window
    }

    fn move_window(&self, id: i64, x: i32, y: i32) {
        self.state.borrow_mut().moved.push((id, x, y));
    }

    fn windows(&self) -> Vec<(i64, Rect)> {
        self.state.borrow().windows.clone()
    }

    fn raise_window(&self, id: i64) {
        self.state.borrow_mut().raised.push(id);
    }
}

type EnvHandle = Rc<RefCell<FakeState>>;

/// 単一モニタ構成の Environment（monitor (0,0,1920,1080) / work area (0,0,1920,1040)）。
fn single_monitor_env() -> (Environment, EnvHandle) {
    let state = Rc::new(RefCell::new(FakeState {
        monitors: vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))],
        cursor: None,
        active_window: None,
        windows: Vec::new(),
        moved: Vec::new(),
        raised: Vec::new(),
    }));
    (
        Environment::new(FakeSource {
            state: state.clone(),
        }),
        state,
    )
}

// =====================================================================
// Manager fixture（scripted action + 合成 config）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、枯渇 = 過剰消費として
/// panic する（rng 消費回数の pin に使う・app_reload_test 踏襲）。
struct BoxedRng {
    values: Vec<f64>,
    consumed: usize,
}

impl Rng for BoxedRng {
    fn unit(&mut self) -> f64 {
        let v = *self
            .values
            .get(self.consumed)
            .unwrap_or_else(|| panic!("BoxedRng 枯渇（{} 回要求）", self.consumed + 1));
        self.consumed += 1;
        v
    }
}

fn fixed_rng(values: Vec<f64>) -> Box<BoxedRng> {
    Box::new(BoxedRng {
        values,
        consumed: 0,
    })
}

/// 既定乱数（すべて 0.5・64 回分）。
fn unit_rng() -> Box<BoxedRng> {
    fixed_rng(vec![0.5; 64])
}

/// 既定 = 常時継続・正常・rng を消費しない action。
/// `has_next_true_times = usize::MAX` で常時 true / `1` で 1 回のみ true
///（app_manager_ext_test の transition_once パターン・2 回目の has_next で
/// 画面外判定経路ではなく遷移経路に分岐させる）。
struct ScriptedAction {
    has_next_true_times: usize,
    has_next_calls: usize,
}

impl Action for ScriptedAction {
    fn init(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }

    fn has_next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.has_next_calls += 1;
        Ok(self.has_next_calls <= self.has_next_true_times)
    }

    fn next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}

/// config 駆動ファクトリ。`fail_on` に含まれる action 参照名は構築 Err を返す
///（set_behavior_at の dispose 経路 pin 用）。
/// `transition_once` に含まれる action 参照名は has_next を 1 回だけ true で返す
///（spawn 直後 tick の画面外判定経路を避けて遷移経路に分岐させる・9b 踏襲）。
struct ScriptedFactory {
    fail_on: HashSet<String>,
    transition_once: HashSet<String>,
}

impl ScriptedFactory {
    fn new() -> Self {
        ScriptedFactory {
            fail_on: HashSet::new(),
            transition_once: HashSet::new(),
        }
    }

    fn with_fail(mut self, names: &[&str]) -> Self {
        self.fail_on.extend(names.iter().map(|n| (*n).to_string()));
        self
    }

    fn with_transitions(mut self, names: &[&str]) -> Self {
        self.transition_once
            .extend(names.iter().map(|n| (*n).to_string()));
        self
    }
}

impl BehaviorFactory for ScriptedFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { name, .. } => {
                if self.fail_on.contains(name) {
                    return Err(BehaviorError::UnknownBehavior(format!(
                        "{name}: テスト用構築失敗"
                    )));
                }
                let times = if self.transition_once.contains(name) {
                    1
                } else {
                    usize::MAX
                };
                Ok(Box::new(ScriptedAction {
                    has_next_true_times: times,
                    has_next_calls: 0,
                }))
            }
            SequenceChild::Inline(_) => Err(BehaviorError::UnknownBehavior(
                "(inline は本テストで未使用)".to_string(),
            )),
        }
    }
}

fn row_entry(name: &str, frequency: i32, hidden: bool, toggleable: bool) -> BehaviorEntry {
    BehaviorEntry::Single(BehaviorDef {
        name: name.to_string(),
        frequency,
        hidden,
        toggleable,
        action: SequenceChild::Ref {
            name: name.to_string(),
            attrs: VarMap::new(),
        },
        next: None,
    })
}

/// 行動名と action 参照名が異なる行（構築 Err 経路の pin 用）。
fn row_with_action(name: &str, action_name: &str, frequency: i32) -> BehaviorEntry {
    BehaviorEntry::Single(BehaviorDef {
        name: name.to_string(),
        frequency,
        hidden: false,
        toggleable: false,
        action: SequenceChild::Ref {
            name: action_name.to_string(),
            attrs: VarMap::new(),
        },
        next: None,
    })
}

/// 非 toggleable・非 hidden の行 1 つ。
fn row(name: &str, frequency: i32) -> BehaviorEntry {
    row_entry(name, frequency, false, false)
}

fn table(entries: Vec<BehaviorEntry>) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig { entries })
}

fn make_manager_with_rng(
    env: Environment,
    tbl: BehaviorTable,
    factory: ScriptedFactory,
    rng: Box<dyn Rng>,
) -> Manager {
    let mut manager = Manager::new(env, tbl, Box::new(factory), rng);
    manager.set_exit_on_last_removed(false);
    manager
}

fn make_manager(env: Environment, tbl: BehaviorTable, factory: ScriptedFactory) -> Manager {
    make_manager_with_rng(env, tbl, factory, unit_rng())
}

/// 指定 set の新規 Mascot（fresh・behavior 無し・空 ImageSet）。
fn mascot_of_set(image_set: &str, anchor: (i32, i32)) -> Mascot {
    Mascot::new(image_set, empty_image_set(image_set), anchor)
}

/// 単一フレーム付き Mascot（Arc 交換の観測用）。
fn mascot_of_set_with(
    image_set: &str,
    file: &str,
    width: u32,
    height: u32,
    anchor: (i32, i32),
) -> Mascot {
    Mascot::new(
        image_set,
        image_set_with(image_set, file, width, height),
        anchor,
    )
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
/// index 順 = Manager 内部の mascots 順。
#[derive(Debug, Clone)]
struct MascotView {
    image_set: String,
    behavior: Option<String>,
    anchor: (i32, i32),
    look_right: bool,
    paused: bool,
    /// image_set() の先頭フレーム寸法（Arc 交換の観測点・空なら None）。
    frame_dims: Option<(u32, u32)>,
    remove_pending: bool,
}

fn snapshot(manager: &mut Manager) -> Vec<MascotView> {
    let mut out: Vec<MascotView> = Vec::new();
    manager.apply_all(|m| {
        out.push(MascotView {
            image_set: m.image_set_name().to_string(),
            behavior: m.behavior_name().map(|n| n.to_string()),
            anchor: m.anchor(),
            look_right: m.look_right(),
            paused: m.is_paused(),
            frame_dims: m
                .image_set()
                .frames
                .values()
                .next()
                .map(|f| (f.width, f.height)),
            remove_pending: m.remove_pending(),
        });
    });
    out
}

// =====================================================================
// temp dir fixture（app_reload_test.rs の TempAssets 踏襲・conf/img は必要時生成）
// =====================================================================

const MINIMAL_ACTIONS_XML: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n",
    "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
    "  <ActionList>\n",
    "    <Action Name=\"Walk\" Type=\"Stay\">\n",
    "      <Animation>\n",
    "        <Pose Image=\"/walk1.png\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/>\n",
    "      </Animation>\n",
    "    </Action>\n",
    "  </ActionList>\n",
    "</Mascot>\n"
);

const MINIMAL_BEHAVIORS_XML: &str = concat!(
    "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
    "  <BehaviorList>\n",
    "    <Behavior Name=\"ChaseMouse\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Fall\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Dragged\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Thrown\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Walk\" Frequency=\"1\"/>\n",
    "  </BehaviorList>\n",
    "</Mascot>\n"
);

/// 使い捨てディレクトリ（Drop で再帰削除）。
struct TempHome {
    root: PathBuf,
}

impl TempHome {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("simeji_t9c_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // 前回残留の掃除
        std::fs::create_dir_all(&root).expect("ルートディレクトリを作れる");
        TempHome { root }
    }

    fn settings_path(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    fn conf_dir(&self) -> PathBuf {
        let dir = self.root.join("conf");
        std::fs::create_dir_all(&dir).expect("conf ディレクトリを作れる");
        dir
    }

    fn img_dir(&self) -> PathBuf {
        let dir = self.root.join("img");
        std::fs::create_dir_all(&dir).expect("img ディレクトリを作れる");
        dir
    }

    fn write_conf(&self, name: &str, content: &str) {
        std::fs::write(self.conf_dir().join(name), content).expect("conf ファイルを書ける");
    }

    /// 最小有効な actions.xml + behaviors.xml（必須 4 種含む）を書き込む。
    fn write_valid_conf(&self) {
        self.write_conf("actions.xml", MINIMAL_ACTIONS_XML);
        self.write_conf("behaviors.xml", MINIMAL_BEHAVIORS_XML);
    }

    /// 単色 PNG を書き込む（img/<set>/<file>）。
    fn write_png(&self, set: &str, file: &str, width: u32, height: u32) {
        let dir = self.img_dir().join(set);
        std::fs::create_dir_all(&dir).expect("set ディレクトリを作れる");
        image::RgbaImage::from_pixel(width, height, image::Rgba([255, 0, 0, 255]))
            .save(dir.join(file))
            .expect("テンポラリ PNG を書ける");
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// apply_tray_command 用 TrayContext（conf/img ともに home ルート・Reload 用テストは
/// conf_dir/img_dir を個別に指定する）。
fn make_apply_context(home: &TempHome, image_sets: &[&str]) -> TrayContext {
    TrayContext {
        conf_dir: home.root.clone(),
        img_dir: home.root.clone(),
        image_sets: image_sets.iter().map(|s| (*s).to_string()).collect(),
    }
}

// =====================================================================
// AllowedKind / AllowedSettings のテスト内対照表（契約固定のラベル順・対応表）
// =====================================================================

/// Allowed Behaviours サブメニューのラベル順（design §3-11・増殖/変身/投げ/
/// 画面間移動/効果音枠/Transients）と AllowedKind の対応。
const ALLOWED_LABELS: [&str; 6] = [
    "しめじを増やす動作",
    "スキン変更、変身",
    "ウインドウを投げる行為",
    "マルチモニターの場合しめじが複数の画面で動作するのを許可",
    "効果音の許可",
    "特殊効果",
];

fn allowed_kinds_in_menu_order() -> [AllowedKind; 6] {
    [
        AllowedKind::Breeding,
        AllowedKind::Transformation,
        AllowedKind::Throwing,
        AllowedKind::Multiscreen,
        AllowedKind::Sounds,
        AllowedKind::Transients,
    ]
}

/// AllowedKind → AllowedSettings フィールドの契約対応（テスト側の対照表）。
fn allowed_field(allowed: &AllowedSettings, kind: &AllowedKind) -> bool {
    match kind {
        AllowedKind::Breeding => allowed.breeding,
        AllowedKind::Transients => allowed.transients,
        AllowedKind::Transformation => allowed.transformation,
        AllowedKind::Throwing => allowed.throwing,
        AllowedKind::Sounds => allowed.sounds,
        AllowedKind::Multiscreen => allowed.multiscreen,
    }
}

/// 種別名（kind_name の逆引き）→ AllowedKind の新規構築
///（AllowedKind の Clone/Copy を pin しないための tests 側ヘルパ）。
fn kind_by_name(name: &str) -> AllowedKind {
    match name {
        "Breeding" => AllowedKind::Breeding,
        "Transients" => AllowedKind::Transients,
        "Transformation" => AllowedKind::Transformation,
        "Throwing" => AllowedKind::Throwing,
        "Sounds" => AllowedKind::Sounds,
        _ => AllowedKind::Multiscreen,
    }
}

/// 種別名 → AllowedSettings フィールド（allowed_field の名前版）。
fn allowed_field_by_name(allowed: &AllowedSettings, name: &str) -> bool {
    match name {
        "Breeding" => allowed.breeding,
        "Transients" => allowed.transients,
        "Transformation" => allowed.transformation,
        "Throwing" => allowed.throwing,
        "Sounds" => allowed.sounds,
        _ => allowed.multiscreen,
    }
}

fn same_kind(a: &AllowedKind, b: &AllowedKind) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
}

/// メッセージ用の種別名（AllowedKind の Debug/Display は pin しない）。
fn kind_name(kind: &AllowedKind) -> &'static str {
    if same_kind(kind, &AllowedKind::Breeding) {
        "Breeding"
    } else if same_kind(kind, &AllowedKind::Transients) {
        "Transients"
    } else if same_kind(kind, &AllowedKind::Transformation) {
        "Transformation"
    } else if same_kind(kind, &AllowedKind::Throwing) {
        "Throwing"
    } else if same_kind(kind, &AllowedKind::Sounds) {
        "Sounds"
    } else {
        "Multiscreen"
    }
}

/// 6 フィールドを明示して AllowedSettings を作る（AllowedSettings::default の
/// pin は Settings::default 経由のみに限定するため）。
#[allow(clippy::too_many_arguments)]
fn allowed(
    breeding: bool,
    transients: bool,
    transformation: bool,
    throwing: bool,
    sounds: bool,
    multiscreen: bool,
) -> AllowedSettings {
    AllowedSettings {
        breeding,
        transients,
        transformation,
        throwing,
        sounds,
        multiscreen,
    }
}

/// Settings 同士の全フィールド比較（PartialEq derive を pin しないため手書き）。
fn assert_settings_eq(expected: &Settings, actual: &Settings) {
    for kind in allowed_kinds_in_menu_order() {
        let field = allowed_field(&expected.allowed, &kind);
        assert_eq!(
            allowed_field(&actual.allowed, &kind),
            field,
            "allowed.{}",
            kind_name(&kind)
        );
    }
    assert_eq!(actual.disabled_behaviors, expected.disabled_behaviors);
    assert_eq!(actual.scales(), expected.scales());
    assert_eq!(
        actual.general.show_console, expected.general.show_console,
        "general.show_console"
    );
    assert_eq!(
        actual.general.language, expected.general.language,
        "general.language"
    );
    assert_eq!(
        actual.interactive_windows.whitelist, expected.interactive_windows.whitelist,
        "interactive_windows.whitelist"
    );
    assert_eq!(
        actual.interactive_windows.blacklist, expected.interactive_windows.blacklist,
        "interactive_windows.blacklist"
    );
}

// =====================================================================
// 契約 A: 設定永続化（Settings / AllowedSettings / ImagesetsSettings）
// =====================================================================

/// Settings::default() = 6 トグル全て true（Java Settings.java L89-94）+
/// 空 map + scales() accessor。
#[test]
fn settings_default_is_all_true_with_empty_maps() {
    let settings = Settings::default();
    assert!(settings.allowed.breeding);
    assert!(settings.allowed.transients);
    assert!(settings.allowed.transformation);
    assert!(settings.allowed.throwing);
    assert!(settings.allowed.sounds);
    assert!(settings.allowed.multiscreen);
    assert!(settings.disabled_behaviors.is_empty(), "既定は空 map");
    assert!(settings.imagesets.scale.is_empty(), "既定は空 map");
    assert!(
        settings.scales().is_empty(),
        "scales() は imagesets.scale と同じ"
    );
    assert!(
        !settings.general.show_console,
        "general 既定は show_console false"
    );
}

/// ファイル不在 → Ok(既定)（Java は load 時に isRegularFile チェックで既定適用）。
/// 空ファイル（有効な空 TOML）も既定。
#[test]
fn settings_load_missing_and_empty_files_yield_defaults() {
    let home = TempHome::new("settings_missing");

    // ファイル不在
    let missing = Settings::load(&home.settings_path()).expect("不在ファイルはエラーでなく既定");
    assert!(missing.allowed.breeding);
    assert!(missing.allowed.sounds);
    assert!(missing.disabled_behaviors.is_empty());
    assert!(missing.scales().is_empty());

    // 空ファイル
    std::fs::write(home.settings_path(), "").expect("空ファイルを書ける");
    let empty = Settings::load(&home.settings_path()).expect("空 TOML は既定");
    assert!(empty.allowed.multiscreen);
    assert!(empty.disabled_behaviors.is_empty());
    assert!(empty.scales().is_empty());
}

/// 欠落セクション / 欠落フィールド → 既定値の補完（allowed の 1 フィールドのみ記載）。
#[test]
fn settings_load_partial_toml_fills_missing_defaults() {
    let home = TempHome::new("settings_partial");
    std::fs::write(home.settings_path(), "[allowed]\nthrowing = false\n")
        .expect("部分 TOML を書ける");

    let settings = Settings::load(&home.settings_path()).expect("部分 TOML を読める");
    assert!(!settings.allowed.throwing, "記載済みフィールドは読まれる");
    assert!(settings.allowed.breeding, "欠落フィールドは既定 true");
    assert!(settings.allowed.transients);
    assert!(settings.allowed.transformation);
    assert!(settings.allowed.sounds);
    assert!(settings.allowed.multiscreen);
    assert!(
        settings.disabled_behaviors.is_empty(),
        "欠落セクションは空 map"
    );
    assert!(settings.scales().is_empty(), "欠落セクションは空 map");
}

/// 手書きサンプル TOML の解釈（design §3-14 例の `[imagesets] scale = { ... }` 形状 +
/// 6 トグルの一部 false + disabled_behaviors 2 set）。
#[test]
fn settings_load_handwritten_sample_toml() {
    let home = TempHome::new("settings_sample");
    let sample = concat!(
        "[allowed]\n",
        "breeding = false\n",
        "transformation = false\n",
        "throwing = false\n",
        "\n",
        "[disabled_behaviors]\n",
        "Shimeji = [\"Walk\", \"SitDown\"]\n",
        "Kuro = [\"Dance\"]\n",
        "\n",
        "[imagesets]\n",
        "scale = { Shimeji = 0.5, HiRes = 0.25 }\n",
    );
    std::fs::write(home.settings_path(), sample).expect("サンプル TOML を書ける");

    let settings = Settings::load(&home.settings_path()).expect("サンプル TOML を解釈できる");
    assert!(!settings.allowed.breeding);
    assert!(settings.allowed.transients, "未記載は既定 true");
    assert!(!settings.allowed.transformation);
    assert!(!settings.allowed.throwing);
    assert!(settings.allowed.sounds);
    assert!(settings.allowed.multiscreen);
    assert_eq!(settings.disabled_behaviors.len(), 2, "2 set 分のエントリ");
    assert_eq!(
        settings
            .disabled_behaviors
            .get("Shimeji")
            .map(Vec::as_slice),
        Some(&["Walk".to_string(), "SitDown".to_string()][..]),
        "disabled_behaviors のリスト内容"
    );
    assert_eq!(
        settings.disabled_behaviors.get("Kuro").map(Vec::as_slice),
        Some(&["Dance".to_string()][..])
    );
    assert_eq!(settings.scales().len(), 2, "2 set 分の scale");
    assert_eq!(settings.scales().get("Shimeji"), Some(&0.5));
    assert_eq!(settings.scales().get("HiRes"), Some(&0.25));
}

/// 未知キー / 未知セクションは無視（エラーにしない）。
#[test]
fn settings_unknown_keys_are_ignored() {
    let home = TempHome::new("settings_unknown");
    let sample = concat!(
        "[allowed]\n",
        "breeding = false\n",
        "future_toggle = \"x\"\n",
        "\n",
        "[imagesets]\n",
        "scale = { Shimeji = 0.5 }\n",
        "future_key = 1\n",
        "\n",
        "[future_section]\n",
        "whatever = true\n",
    );
    std::fs::write(home.settings_path(), sample).expect("サンプル TOML を書ける");

    let settings = Settings::load(&home.settings_path()).expect("未知キーは無視して読める");
    assert!(!settings.allowed.breeding);
    assert!(settings.allowed.transients, "未知キーは allowed に入らない");
    assert_eq!(settings.scales().get("Shimeji"), Some(&0.5));
    assert_eq!(settings.scales().len(), 1);
}

/// 壊れた TOML → SettingsError（parse 経路）/ 保存先不在 → SettingsError（io 経路）。
/// Display は非空文字列（文言は pin しない）。
#[test]
fn settings_corrupt_toml_and_io_errors_are_settings_error() {
    let home = TempHome::new("settings_broken");
    std::fs::write(home.settings_path(), "[allowed\nbreeding = true\n")
        .expect("壊れた TOML を書ける");
    let err: SettingsError = Settings::load(&home.settings_path()).expect_err("壊れた TOML は Err");
    assert!(
        !err.to_string().is_empty(),
        "Display は英語メッセージ（非空）"
    );

    // io 経路: 保存先ディレクトリ不在
    let bad_dir = home.root.join("no_such_dir");
    let err: SettingsError = Settings::save(&bad_dir.join("settings.toml"), &Settings::default())
        .expect_err("保存先不在は Err");
    assert!(
        !err.to_string().is_empty(),
        "Display は英語メッセージ（非空）"
    );
}

/// round-trip: save → load で全フィールド同値。save は 2 回行っても同値
///（BTreeMap 順序の安定性・byte 一致）。
#[test]
fn settings_round_trip_preserves_all_values() {
    let home = TempHome::new("settings_roundtrip");

    let mut disabled = BTreeMap::new();
    disabled.insert(
        "Shimeji".to_string(),
        vec!["Walk".to_string(), "Stare".to_string()],
    );
    disabled.insert("Kuro".to_string(), vec!["Dance".to_string()]);
    let mut scale = BTreeMap::new();
    scale.insert("Shimeji".to_string(), 0.5);
    scale.insert("HiRes".to_string(), 0.25);
    let original = Settings {
        allowed: allowed(false, true, false, true, false, true),
        disabled_behaviors: disabled,
        imagesets: ImagesetsSettings { scale },
        general: GeneralSettings {
            show_console: true,
            ..Default::default()
        },
        interactive_windows: InteractiveWindowsSettings::default(),
    };

    let path = home.settings_path();
    Settings::save(&path, &original).expect("save できる");
    let loaded = Settings::load(&path).expect("load できる");
    assert_settings_eq(&original, &loaded);

    // もう一度 save → load しても同値
    Settings::save(&path, &loaded).expect("再 save できる");
    let reloaded = Settings::load(&path).expect("再 load できる");
    assert_settings_eq(&original, &reloaded);

    // scales() accessor 経由でも同内容
    assert_eq!(loaded.scales().get("Shimeji"), Some(&0.5));
    assert_eq!(loaded.scales().get("HiRes"), Some(&0.25));
}

/// save は BTreeMap 順序で安定出力（同じ設定の 2 回の save は byte 一致・
/// disabled / scale の set 名は辞書順に出る）。
#[test]
fn settings_save_is_deterministic_with_sorted_keys() {
    let home = TempHome::new("settings_sorted");

    // 挿入順をバラバラにしても出力は BTreeMap 順（辞書順）で安定する
    let mut disabled = BTreeMap::new();
    disabled.insert("Zoo".to_string(), vec!["Walk".to_string()]);
    disabled.insert("Alpha".to_string(), vec!["SitDown".to_string()]);
    disabled.insert("Mango".to_string(), vec!["Dance".to_string()]);
    let mut scale = BTreeMap::new();
    scale.insert("Delta".to_string(), 2.0);
    scale.insert("Beta".to_string(), 0.5);
    let settings = Settings {
        allowed: allowed(true, true, true, true, true, true),
        disabled_behaviors: disabled,
        imagesets: ImagesetsSettings { scale },
        general: GeneralSettings::default(),
        interactive_windows: InteractiveWindowsSettings::default(),
    };

    let path = home.settings_path();
    Settings::save(&path, &settings).expect("save できる");
    let text1 = std::fs::read_to_string(&path).expect("保存結果を読める");
    Settings::save(&path, &settings).expect("再 save できる");
    let text2 = std::fs::read_to_string(&path).expect("再保存結果を読める");
    assert_eq!(
        text1, text2,
        "同じ設定の 2 回の save は byte 一致（順序安定）"
    );

    let alpha = text1.find("Alpha").expect("Alpha が出力される");
    let mango = text1.find("Mango").expect("Mango が出力される");
    let zoo = text1.find("Zoo").expect("Zoo が出力される");
    assert!(
        alpha < mango && mango < zoo,
        "disabled の set 名は辞書順で出力される"
    );
    let beta = text1.find("Beta").expect("Beta が出力される");
    let delta = text1.find("Delta").expect("Delta が出力される");
    assert!(beta < delta, "scale の set 名も辞書順で出力される");

    let reloaded = Settings::load(&path).expect("load できる");
    assert_settings_eq(&settings, &reloaded);
}

// =====================================================================
// 契約 A-2: settings.toml の [general] show_console + 初回自動生成
// =====================================================================

/// 後方互換: 既存（旧形式）TOML には [general] セクションが無い。
/// load すると show_console は既定 false に補完され、他セクションの解釈は不変。
#[test]
fn settings_load_without_general_section_defaults_show_console_false() {
    let home = TempHome::new("settings_no_general");
    let legacy = concat!(
        "[allowed]\n",
        "throwing = false\n",
        "\n",
        "[imagesets]\n",
        "scale = { Shimeji = 0.5 }\n",
    );
    std::fs::write(home.settings_path(), legacy).expect("旧形式 TOML を書ける");

    let settings = Settings::load(&home.settings_path()).expect("旧形式 TOML を読める");
    assert!(
        !settings.general.show_console,
        "general セクション欠落は既定 false（後方互換）"
    );
    assert!(
        !settings.allowed.throwing,
        "旧セクションは従来どおり解釈される"
    );
    assert_eq!(settings.scales().get("Shimeji"), Some(&0.5));
}

/// 手書き TOML の `[general] show_console = true` を load → true。
/// （save→load の自己整合では TOML キー名の誤りを検出できないため、
/// 実キー名を独立に固定する。）
#[test]
fn settings_load_general_show_console_true() {
    let home = TempHome::new("settings_general_true");
    std::fs::write(home.settings_path(), "[general]\nshow_console = true\n")
        .expect("[general] TOML を書ける");

    let settings = Settings::load(&home.settings_path()).expect("[general] TOML を読める");
    assert!(
        settings.general.show_console,
        "show_console = true が読み取られる"
    );
}

/// create_default_if_missing: 不在パス → Ok(true) で既定値の settings.toml を
/// 新規生成し、そのファイルが Settings::load で読める
///（6 トグル true・空 map・show_console false）。
#[test]
fn create_default_if_missing_writes_readable_defaults() {
    let home = TempHome::new("create_missing");
    let path = home.settings_path();
    assert!(!path.exists(), "前提: ファイルは不在");

    let created = Settings::create_default_if_missing(&path).expect("不在パスへ生成できる");
    assert!(created, "新規生成時は Ok(true)");
    assert!(path.is_file(), "ファイルが実在する");

    let loaded = Settings::load(&path).expect("生成されたファイルを load できる");
    assert!(loaded.allowed.breeding);
    assert!(loaded.allowed.transients);
    assert!(loaded.allowed.transformation);
    assert!(loaded.allowed.throwing);
    assert!(loaded.allowed.sounds);
    assert!(loaded.allowed.multiscreen);
    assert!(
        !loaded.general.show_console,
        "生成既定は show_console false"
    );
    assert!(loaded.disabled_behaviors.is_empty(), "生成既定は空 map");
    assert!(loaded.scales().is_empty(), "生成既定は空 map");
}

/// create_default_if_missing: 生成される settings.toml に `[interactive_windows]`
/// セクション（空 whitelist/blacklist）とコメントアウトされた記入例が含まれ、
/// 生成物がそのまま load できる（コメントは解釈を妨げない）。
#[test]
fn create_default_if_missing_includes_interactive_windows_section_and_help() {
    let home = TempHome::new("create_interactive");
    let path = home.settings_path();
    Settings::create_default_if_missing(&path).expect("生成できる");

    let text = std::fs::read_to_string(&path).expect("生成物を読める");
    assert!(
        text.contains("[interactive_windows]"),
        "セクション見出しが出力される"
    );
    assert!(text.contains("whitelist = []"), "空 whitelist が出力される");
    assert!(text.contains("blacklist = []"), "空 blacklist が出力される");
    let comment_lines: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .collect();
    assert!(
        comment_lines.iter().any(|l| l.contains("whitelist")),
        "whitelist の記入例コメントがある"
    );
    assert!(
        comment_lines.iter().any(|l| l.contains("blacklist")),
        "blacklist の記入例コメントがある"
    );

    let loaded = Settings::load(&path).expect("記入例コメント付きでも load できる");
    assert!(loaded.interactive_windows.whitelist.is_empty());
    assert!(loaded.interactive_windows.blacklist.is_empty());
}

/// create_default_if_missing: 生成される settings.toml の `[general]` 付近に
/// 対応言語（en / ja）の記入例コメントが含まれ、生成物がそのまま load できる。
#[test]
fn create_default_if_missing_includes_language_help() {
    let home = TempHome::new("create_language");
    let path = home.settings_path();
    Settings::create_default_if_missing(&path).expect("生成できる");

    let text = std::fs::read_to_string(&path).expect("生成物を読める");
    let comment_lines: Vec<&str> = text
        .lines()
        .filter(|l| l.trim_start().starts_with('#'))
        .collect();
    assert!(
        comment_lines
            .iter()
            .any(|l| l.contains("language") && l.contains("en") && l.contains("ja")),
        "対応言語（en/ja）の記入例コメントがある"
    );

    let loaded = Settings::load(&path).expect("記入例コメント付きでも load できる");
    assert_eq!(loaded.general.language, "en", "既定言語は en");
}

/// create_default_if_missing: 既存ファイル → Ok(false) かつ内容を一切変更しない
///（手編集を上書きしない）。
#[test]
fn create_default_if_missing_leaves_existing_file_unchanged() {
    let home = TempHome::new("create_existing");
    let path = home.settings_path();
    let hand_edited = "[general]\nshow_console = true\n";
    std::fs::write(&path, hand_edited).expect("手編集済みファイルを書ける");
    let before = std::fs::read_to_string(&path).expect("内容を読める");

    let created = Settings::create_default_if_missing(&path).expect("既存ファイルでは Ok");
    assert!(!created, "既存ファイルでは Ok(false)");

    let after = std::fs::read_to_string(&path).expect("内容を再読できる");
    assert_eq!(after, before, "既存ファイルの内容は不変（上書きしない）");
}

/// create_default_if_missing: 書き込み不能パス（親ディレクトリ不在）→ Err
///（panic せず SettingsError を返す）。
#[test]
fn create_default_if_missing_errors_on_unwritable_path() {
    let home = TempHome::new("create_unwritable");
    let path = home.root.join("no_such_dir").join("settings.toml");

    let err = Settings::create_default_if_missing(&path).expect_err("親ディレクトリ不在は Err");
    assert!(!err.to_string().is_empty(), "Display は非空メッセージ");
    assert!(!path.exists(), "失敗時にファイルを残さない");
}

// =====================================================================
// 契約 B-1: Environment::set_disabled_behaviors（全体置換）
// =====================================================================

/// 全体置換: 既存 map は消え、注入内容だけで再構築される。直後に
/// behavior_disabled 読みへ反映される（settings.toml 復元注入用の別経路・
/// Java Main L526-544 の per-key 変異 set_behavior_enabled とは住み分け）。
#[test]
fn environment_set_disabled_behaviors_replaces_whole_map() {
    let (mut env, _) = single_monitor_env();
    // 既存エントリを用意（9b の per-key 変異経路）
    env.set_behavior_enabled("SetA", "OldPose", false);
    {
        let view: &dyn EnvironmentView = &env;
        assert!(
            view.behavior_disabled("SetA", "OldPose"),
            "事前: 旧エントリが有効"
        );
    }

    // 全体置換
    let mut disabled = BTreeMap::new();
    disabled.insert(
        "SetB".to_string(),
        vec!["Pose".to_string(), "Spin".to_string()],
    );
    env.set_disabled_behaviors(disabled);
    {
        let view: &dyn EnvironmentView = &env;
        assert!(
            !view.behavior_disabled("SetA", "OldPose"),
            "旧エントリは消滅する（全体置換・マージではない）"
        );
        assert!(
            view.behavior_disabled("SetB", "Pose"),
            "新エントリが直後に反映される"
        );
        assert!(view.behavior_disabled("SetB", "Spin"));
        assert!(!view.behavior_disabled("SetB", "Walk"), "非含有名は false");
        assert!(
            !view.behavior_disabled("SetA", "Pose"),
            "旧 set の新規名も false"
        );
    }

    // 空 map 注入 = 全消し（settings 復元時に無効行動が無いケース）
    env.set_disabled_behaviors(BTreeMap::new());
    {
        let view: &dyn EnvironmentView = &env;
        assert!(!view.behavior_disabled("SetB", "Pose"), "空注入 = 全有効");
    }
}

/// set_disabled_behaviors で注入した無効行動は Manager の既存経路
///（behavior_menu_items / is_behavior_enabled の同一式）からも観測される
///（settings 復元 → トグル UI への反映の接続確認）。
#[test]
fn environment_disabled_behaviors_flow_into_manager_menu_gating() {
    let (mut env, _) = single_monitor_env();
    let mut disabled = BTreeMap::new();
    disabled.insert("TestSet".to_string(), vec!["Pose".to_string()]);
    env.set_disabled_behaviors(disabled);

    let manager = make_manager(
        env,
        table(vec![row("Walk", 100), row_entry("Pose", 100, false, true)]),
        ScriptedFactory::new(),
    );
    let menu = manager.behavior_menu_items("TestSet");
    assert_eq!(
        menu.toggleable,
        [("Pose".to_string(), false)],
        "無効化トグルは checked = false で出る"
    );
    assert_eq!(
        menu.selectable,
        ["Walk"],
        "無効な toggleable は selectable から外れる"
    );
}

// =====================================================================
// 契約 B-2: Manager::request_spawn_random（Java Main.createMascot() L466-505）
// =====================================================================

/// 空スライス → no-op + rng 不消費（rng 値 0 個 = どの消費も panic する pin）。
#[test]
fn manager_request_spawn_random_with_empty_slice_is_noop_without_rng() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new(),
        fixed_rng(vec![]),
    );
    manager.set_image_set_resolver(resolver_for(&["Alpha"]));

    manager.request_spawn_random(&[]);
    manager.request_spawn_random(&[]); // 何度呼んでも no-op
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "空スライスは spawn 要求を出さない");
}

/// 非空スライス → spawn 1 体につき rng.unit() を選択に 1 回（+ request_spawn 内の
/// look_right 1 回）消費し、idx = (unit * len) as usize（Java (int) 切り捨て逐語）
/// で set を選ぶ:
/// - 1 体目: 選択 0.999 → (0.999*4) as usize = 3（四捨五入なら 4 で範囲外 panic）→ "Delta"
///   ・look_right 0.3 → true（0.3 < 0.5）
/// - 2 体目: 選択 0.0 → "Alpha"・look_right 0.6 → false（0.6 は 0.5 未満でない）
///
/// 固定値 4 個（+tick 分の 0.5）で過剰消費があれば枯渇 panic で失敗する。
/// action は transition_once（app_manager_ext_test 踏襲）: 2 回目の has_next で
/// 遷移経路に分岐させ、spawn 直後 tick の画面外再配置
///（behavior.rs L296-322・Java L185-191 逐語）を発生させない。
#[test]
fn manager_request_spawn_random_consumes_one_unit_per_spawn_verbatim() {
    let (env, _) = single_monitor_env();
    let mut values = vec![0.999, 0.3, 0.0, 0.6];
    values.extend(vec![0.5; 32]);
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new().with_transitions(&["Walk"]),
        fixed_rng(values),
    );
    let sets = [
        "Alpha".to_string(),
        "Beta".to_string(),
        "Gamma".to_string(),
        "Delta".to_string(),
    ];
    manager.set_image_set_resolver(resolver_for(&["Alpha", "Beta", "Gamma", "Delta"]));

    manager.request_spawn_random(&sets);
    manager.request_spawn_random(&sets);
    manager.tick(Instant::now());

    assert_eq!(manager.count(), 2, "2 体とも spawn される");
    assert_eq!(manager.count_of("Delta"), 1, "0.999*4 の切り捨て → index 3");
    assert_eq!(manager.count_of("Alpha"), 1, "0.0*4 → index 0");
    assert_eq!(manager.count_of("Beta"), 0);
    assert_eq!(manager.count_of("Gamma"), 0);

    let snap = snapshot(&mut manager);
    for view in &snap {
        // Java L490: setLookRight(Math.random() < 0.5) — Delta の lr 値 0.3 → true /
        // Alpha の lr 値 0.6 → false
        let expected = view.image_set == "Delta";
        assert_eq!(
            view.look_right, expected,
            "{}: look_right = rng.unit() < 0.5 の極性",
            view.image_set
        );
        assert_eq!(view.anchor, (-4000, -4000), "createMascot の anchor 逐語");
    }
}

// =====================================================================
// 契約 B-3: Allowed passthrough 5 種（Sounds は作らない）
// =====================================================================

/// 5 種の passthrough が Environment の既存 setter 群へ届く
///（environment_view() の getter で観測・双方方向）。
/// Sounds の passthrough は契約に存在しない（Phase 1 no-op・UI + 永続化のみ）。
#[test]
fn manager_allowed_passthroughs_reach_environment_getters() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());

    // 既定 = Java Settings.java L89-94 相当（Environment 既定 true）
    {
        let view = manager.environment_view();
        assert!(view.breeding_allowed() && view.transients_enabled());
        assert!(view.transformation_allowed() && view.throwing_allowed());
        assert!(view.multiscreen());
    }

    manager.set_breeding_allowed(false);
    manager.set_transients_enabled(false);
    manager.set_transformation_allowed(false);
    manager.set_throwing_allowed(false);
    manager.set_multiscreen(false);
    {
        let view = manager.environment_view();
        assert!(!view.breeding_allowed(), "breeding が Environment へ届く");
        assert!(
            !view.transients_enabled(),
            "transients が Environment へ届く"
        );
        assert!(
            !view.transformation_allowed(),
            "transformation が Environment へ届く"
        );
        assert!(!view.throwing_allowed(), "throwing が Environment へ届く");
        assert!(!view.multiscreen(), "multiscreen が Environment へ届く");
    }

    // 戻す（passthrough は双方方向で Environment に届く）
    manager.set_breeding_allowed(true);
    manager.set_multiscreen(true);
    {
        let view = manager.environment_view();
        assert!(view.breeding_allowed());
        assert!(view.multiscreen());
        assert!(!view.transients_enabled(), "触っていない種別は不変");
    }
}

// =====================================================================
// 契約 B-4: set_behavior_at / toggle_pause_at / dismiss_at（popup L517-562）
// =====================================================================

/// 単一マスコット版 setBehavior: index の mascot のみ「その mascot の set の
/// table」で構築される。構築 Err → その mascot のみ dispose（削除は次 tick）。
/// index 範囲外 → warn + no-op。
#[test]
fn manager_set_behavior_at_targets_single_mascot_with_own_set_table() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(
        env,
        table(vec![
            row("Walk", 100),
            row_with_action("BreakWalk", "FailAction", 1),
        ]),
        ScriptedFactory::new().with_fail(&["FailAction"]),
    );
    manager.set_behavior_table("AltSet", table(vec![row("AltWalk", 100)]));
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.add(mascot_of_set("AltSet", (20, 600)));
    manager.add(mascot_of_set("TestSet", (30, 700)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 3);

    // index 1（AltSet）のみ: 自分の set の table で構築（base table に無い名前）
    manager.set_behavior_at(1, "AltWalk");
    let snap = snapshot(&mut manager);
    assert_eq!(
        snap[1].behavior.as_deref(),
        Some("AltWalk"),
        "index 1 は自 set の table で構築される"
    );
    assert_eq!(snap[0].behavior, None, "index 0 は無傷");
    assert_eq!(snap[2].behavior, None, "index 2 は無傷");
    assert!(
        snap.iter().all(|v| !v.remove_pending),
        "成功時は dispose しない"
    );

    // 構築 Err（action 参照が factory で失敗）→ その mascot のみ dispose
    manager.set_behavior_at(2, "BreakWalk");
    let snap = snapshot(&mut manager);
    assert!(
        snap[2].remove_pending,
        "構築 Err → その mascot のみ dispose（削除は次 tick）"
    );
    assert!(
        !snap[0].remove_pending && !snap[1].remove_pending,
        "他は無傷（setBehaviorAll L291-310 の catch 準拠）"
    );

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2, "失敗 mascot は次 tick で除去");

    // index 範囲外 → warn + no-op（panic しない・何も壊さない）。
    // 事前状態: index 0（TestSet）= behavior 無し・index 1（AltSet）= 先ほどの AltWalk
    manager.set_behavior_at(9, "Walk");
    let snap = snapshot(&mut manager);
    assert_eq!(snap.len(), 2, "範囲外指定で mascot は増減しない");
    assert!(snap.iter().all(|v| !v.remove_pending));
    assert_eq!(snap[0].behavior, None, "範囲外指定の影響を受けない");
    assert_eq!(
        snap[1].behavior.as_deref(),
        Some("AltWalk"),
        "範囲外指定は既存 behavior も変更しない（no-op）"
    );
}

/// toggle_pause_at = set_paused(!is_paused)（Java popup pauseItem L559 逐語）・
/// dismiss_at = dispose（remove_pending・削除は次 tick・Java popup Dismiss L562）。
/// どちらも index 範囲外は warn + no-op。
#[test]
fn manager_toggle_pause_at_and_dismiss_at_affect_only_that_mascot() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.add(mascot_of_set("TestSet", (20, 600)));
    manager.add(mascot_of_set("TestSet", (30, 700)));
    manager.tick(Instant::now());

    manager.toggle_pause_at(1);
    let snap = snapshot(&mut manager);
    assert!(!snap[0].paused);
    assert!(snap[1].paused, "index 1 のみ pause");
    assert!(!snap[2].paused);

    // 再トグルで戻る（set_paused(!is_paused) 逐語）
    manager.toggle_pause_at(1);
    let snap = snapshot(&mut manager);
    assert!(!snap[1].paused, "再トグルで復帰");

    manager.dismiss_at(0);
    let snap = snapshot(&mut manager);
    assert!(
        snap[0].remove_pending,
        "dismiss は dispose（remove_pending）"
    );
    assert!(
        !snap[1].remove_pending && !snap[2].remove_pending,
        "他は無傷"
    );
    assert_eq!(manager.count(), 3, "削除は次 tick");

    // 範囲外 → warn + no-op（事前状態: index 0 のみ dismiss_at(0) で pending）
    manager.toggle_pause_at(9);
    manager.dismiss_at(9);
    let snap = snapshot(&mut manager);
    assert_eq!(
        snap.iter().filter(|v| v.remove_pending).count(),
        1,
        "範囲外の dismiss は追加の dispose をしない（index 0 のみ）"
    );
    assert!(
        snap[0].remove_pending,
        "pending は先ほどの dismiss_at(0) のみ"
    );
    assert!(
        snap.iter().all(|v| !v.paused),
        "範囲外の toggle は何もしない"
    );

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2, "index 0 の mascot が次 tick で除去");
}

// =====================================================================
// 契約 C-1: TrayMenuModel — tray メニュー構成（design §3-11 の挿入順）
// =====================================================================

fn kind_label(kind: &MenuItemKind) -> &'static str {
    match kind {
        MenuItemKind::MenuItem(_) => "item",
        MenuItemKind::Submenu(_) => "submenu",
        MenuItemKind::Predefined(_) => "separator",
        MenuItemKind::Check(_) => "check",
        MenuItemKind::Icon(_) => "icon",
    }
}

fn expect_submenu<'a>(kind: &'a MenuItemKind, what: &str) -> &'a Submenu {
    kind.as_submenu()
        .unwrap_or_else(|| panic!("{what} は Submenu であること"))
}

fn expect_check<'a>(kind: &'a MenuItemKind, what: &str) -> &'a CheckMenuItem {
    kind.as_check_menuitem()
        .unwrap_or_else(|| panic!("{what} は CheckMenuItem であること"))
}

/// テキスト付き項目（MenuItem / CheckMenuItem）のラベル。
fn expect_item_text(kind: &MenuItemKind, what: &str) -> String {
    if let Some(item) = kind.as_menuitem() {
        return item.text();
    }
    if let Some(check) = kind.as_check_menuitem() {
        return check.text();
    }
    panic!("{what} はテキスト付き項目であること");
}

fn expect_separator(kind: &MenuItemKind, what: &str) {
    assert!(
        kind.as_predefined_menuitem().is_some(),
        "{what} は separator（PredefinedMenuItem）であること"
    );
}

/// tray メニューの全項目が design §3-11 の順で存在し、全 id が対応する
/// TrayCommand に写像される（Menu::items() 走査 + Submenu 子走査）。
/// SetAllowed の値は muda 自動トグル済みの新値 = 現在の checked そのままを pin
///（実物照合 L1195-1198・build 直後は実 UI クリックが無いため
/// checked = allowed 初期値のまま）。
#[test]
fn tray_menu_structure_and_commands_match_design_3_11() {
    let sets = vec!["Shimeji".to_string(), "Kuro".to_string()];
    let model = TrayMenuModel::build_tray(
        &sets,
        &allowed(true, true, true, true, true, true),
        &ja_lang(),
    );
    let items = model.menu().items();
    assert_eq!(
        items.len(),
        10,
        "tray は 10 項目（submenu 3 + item 5 + separator 2）"
    );

    let pattern: Vec<&str> = items.iter().map(kind_label).collect();
    assert_eq!(
        pattern,
        [
            "submenu",
            "item",
            "item",
            "item",
            "submenu",
            "separator",
            "item",
            "item",
            "separator",
            "item",
        ],
        "挿入順（design §3-11 の 1〜10）"
    );

    // 1. 「呼ぶ」Submenu: 先頭「（ランダム）」= Spawn(None) + 各 set 名 = Spawn(Some(set))
    let spawn = expect_submenu(&items[0], "呼ぶ");
    assert_eq!(spawn.text(), "しめじを呼ぶ");
    let children = spawn.items();
    assert_eq!(
        children.len(),
        3,
        "（ランダム）+ set 名分（tray-icon 経由の挿入順保持）"
    );
    assert_eq!(expect_item_text(&children[0], "ランダム"), "（ランダム）");
    assert!(
        matches!(
            model.command_of(children[0].id()),
            Some(TrayCommand::Spawn(None))
        ),
        "「（ランダム）」→ Spawn(None)"
    );
    assert_eq!(expect_item_text(&children[1], "set 1"), "Shimeji");
    assert!(
        matches!(
            model.command_of(children[1].id()),
            Some(TrayCommand::Spawn(Some(s))) if s == "Shimeji"
        ),
        "set 名項目 → Spawn(Some(set))"
    );
    assert_eq!(expect_item_text(&children[2], "set 2"), "Kuro");
    assert!(
        matches!(
            model.command_of(children[2].id()),
            Some(TrayCommand::Spawn(Some(s))) if s == "Kuro"
        ),
        "image_sets 順に Submenu 子が並ぶ"
    );

    // 2-4. Follow Cursor / Reduce to One / Restore Windows
    assert_eq!(
        expect_item_text(&items[1], "Follow"),
        "カーソルを追っかける"
    );
    assert!(matches!(
        model.command_of(items[1].id()),
        Some(TrayCommand::FollowCursor)
    ));
    assert_eq!(expect_item_text(&items[2], "Reduce"), "一つだけにする");
    assert!(matches!(
        model.command_of(items[2].id()),
        Some(TrayCommand::ReduceToOne)
    ));
    assert_eq!(
        expect_item_text(&items[3], "Restore"),
        "ウインドウをもとに戻す"
    );
    assert!(matches!(
        model.command_of(items[3].id()),
        Some(TrayCommand::RestoreWindows)
    ));

    // 5. Allowed Behaviours Submenu: CheckMenuItem 6 種（ラベル順固定・checked = allowed）
    let allowed_sub = expect_submenu(&items[4], "許可する行為");
    assert_eq!(allowed_sub.text(), "許可する行為");
    let checks = allowed_sub.items();
    assert_eq!(checks.len(), 6, "トグルは 6 種");
    let kinds = allowed_kinds_in_menu_order();
    for index in 0..6 {
        let check = expect_check(&checks[index], "トグル項目");
        assert_eq!(
            check.text(),
            ALLOWED_LABELS[index],
            "ラベル順（増殖/変身/投げ/画面間移動/効果音枠/Transients）"
        );
        assert!(
            check.is_checked(),
            "{}: allowed 既定 true が初期 checked に反映",
            ALLOWED_LABELS[index]
        );
        // SetAllowed の値 = MenuEvent 受信時の適用値 = 現在の checked そのまま
        //（muda 自動トグル済みの新値を pin・実物照合 L1195-1198）。
        // build 直後は実 UI クリックが無いため checked = allowed 初期値のまま
        let applied = check.is_checked();
        assert!(
            matches!(
                model.command_of(check.id()),
                Some(TrayCommand::SetAllowed(kind, value))
                    if same_kind(&kind, &kinds[index]) && value == applied
            ),
            "{} → SetAllowed(対応種別, 現在の checked = 適用値)",
            ALLOWED_LABELS[index]
        );
    }

    // 6. separator
    expect_separator(&items[5], "許可する行為 の後");

    // 7-8. 一時停止 / Dismiss All
    assert_eq!(expect_item_text(&items[6], "一時停止"), "一時停止");
    assert!(matches!(
        model.command_of(items[6].id()),
        Some(TrayCommand::TogglePauseAll)
    ));
    assert_eq!(expect_item_text(&items[7], "Dismiss All"), "すべて消す");
    assert!(matches!(
        model.command_of(items[7].id()),
        Some(TrayCommand::DismissAll)
    ));

    // 9. separator
    expect_separator(&items[8], "Dismiss All の後");

    // 10. Reload
    assert_eq!(expect_item_text(&items[9], "Reload"), "再読み込み");
    assert!(matches!(
        model.command_of(items[9].id()),
        Some(TrayCommand::Reload)
    ));
}

/// CheckMenuItem の初期 checked = allowed の初期値（既定 true でない値を注入して pin）。
/// set 名 0 件でも「呼ぶ」は（ランダム）のみで成立する。
#[test]
fn tray_menu_allowed_check_items_reflect_initial_allowed_values() {
    let empty: Vec<String> = vec![];
    let initial = allowed(false, true, false, true, true, false);
    let model = TrayMenuModel::build_tray(&empty, &initial, &ja_lang());
    let items = model.menu().items();

    let checks = expect_submenu(&items[4], "許可する行為").items();
    let kinds = allowed_kinds_in_menu_order();
    for index in 0..6 {
        let check = expect_check(&checks[index], "トグル項目");
        assert_eq!(
            check.is_checked(),
            allowed_field(&initial, &kinds[index]),
            "{}: 初期 checked = allowed の値",
            ALLOWED_LABELS[index]
        );
    }

    // 「呼ぶ」: set 0 件 →（ランダム）のみ
    let children = expect_submenu(&items[0], "呼ぶ").items();
    assert_eq!(children.len(), 1, "set 0 件 →（ランダム）のみ");
    assert!(matches!(
        model.command_of(children[0].id()),
        Some(TrayCommand::Spawn(None))
    ));
}

/// sync_allowed は全 CheckMenuItem へ set_checked を反映し、command_of は
/// sync 後も有効（SetAllowed の値は反映後の現在の checked = 適用値に更新される・
/// muda 自動トグル済みの新値を pin・実物照合 L1195-1198）。
#[test]
fn tray_menu_sync_allowed_updates_all_check_items() {
    let sets = vec!["Shimeji".to_string()];
    let model = TrayMenuModel::build_tray(
        &sets,
        &allowed(true, true, true, true, true, true),
        &ja_lang(),
    );

    let updated = allowed(false, true, true, true, false, false);
    model.sync_allowed(&updated);

    let items = model.menu().items();
    let checks = expect_submenu(&items[4], "許可する行為").items();
    let kinds = allowed_kinds_in_menu_order();
    for index in 0..6 {
        let check = expect_check(&checks[index], "トグル項目");
        assert_eq!(
            check.is_checked(),
            allowed_field(&updated, &kinds[index]),
            "{}: sync 後の checked が allowed に一致",
            ALLOWED_LABELS[index]
        );
        // SetAllowed の値 = sync 後の現在の checked（= 適用値）そのまま
        //（muda 自動トグル済みの新値を pin・実物照合 L1195-1198）
        let applied = check.is_checked();
        assert!(
            matches!(
                model.command_of(check.id()),
                Some(TrayCommand::SetAllowed(kind, value))
                    if same_kind(&kind, &kinds[index]) && value == applied
            ),
            "{}: sync 後も command_of が有効（値 = 現在の checked = 適用値）",
            ALLOWED_LABELS[index]
        );
    }
}

// =====================================================================
// 契約 C-2: TrayMenuModel — popup メニュー構成（design §3-11 最小セット）
// =====================================================================

/// popup: 呼ぶ（tray と同内容）/ 個別行動指定（selectable のみ・toggleable 専用は
/// 出さない）/ separator / 一時停止 or 再開（is_paused で切替）/ 消す。
/// index は build_popup に渡した値が TrayCommand に載る。
#[test]
fn popup_menu_structure_selectable_only_and_paused_label() {
    let sets = vec!["Shimeji".to_string(), "Kuro".to_string()];
    let menu_items = BehaviorMenu {
        selectable: vec!["Walk".to_string(), "Stare".to_string(), "Jump".to_string()],
        toggleable: vec![
            ("Stare".to_string(), true),
            ("Jump".to_string(), true),
            ("Spin".to_string(), false),
        ],
    };
    let index = 2usize;
    let model = TrayMenuModel::build_popup(index, &sets, &menu_items, false, &ja_lang());
    let items = model.menu().items();
    assert_eq!(
        items.len(),
        5,
        "popup は 5 項目（submenu 2 + item 2 + separator 1）"
    );

    let pattern: Vec<&str> = items.iter().map(kind_label).collect();
    assert_eq!(pattern, ["submenu", "submenu", "separator", "item", "item"]);

    // 1. 「同じしめじを呼ぶ」（popup 専用キー CallAnother・tray の「しめじを呼ぶ」とは別）
    let spawn = expect_submenu(&items[0], "同じしめじを呼ぶ");
    assert_eq!(spawn.text(), "同じしめじを呼ぶ");
    let children = spawn.items();
    assert_eq!(children.len(), 3);
    assert!(matches!(
        model.command_of(children[0].id()),
        Some(TrayCommand::Spawn(None))
    ));
    assert!(matches!(
        model.command_of(children[1].id()),
        Some(TrayCommand::Spawn(Some(s))) if s == "Shimeji"
    ),);
    assert!(matches!(
        model.command_of(children[2].id()),
        Some(TrayCommand::Spawn(Some(s))) if s == "Kuro"
    ),);

    // 2. 「行為の設定」: selectable のみ（toggleable 専用の Spin は出ない）
    let behavior = expect_submenu(&items[1], "行為の設定");
    assert_eq!(behavior.text(), "行為の設定");
    let rows = behavior.items();
    assert_eq!(rows.len(), 3, "toggleable 専用（Spin）は popup に出さない");
    for (position, name) in ["Walk", "Stare", "Jump"].into_iter().enumerate() {
        assert_eq!(expect_item_text(&rows[position], "行動"), name);
        assert!(
            matches!(
                model.command_of(rows[position].id()),
                Some(TrayCommand::SetBehaviorFor(target, ref got))
                    if target == index && got == name
            ),
            "行動名 → SetBehaviorFor(index, name)"
        );
    }

    // 3. separator
    expect_separator(&items[2], "行為の設定の後");

    // 4-5. 一時停止（未 pause）・消す
    assert_eq!(expect_item_text(&items[3], "一時停止"), "一時停止");
    assert!(
        matches!(model.command_of(items[3].id()), Some(TrayCommand::TogglePauseFor(target)) if target == index),
        "一時停止 → TogglePauseFor(index)"
    );
    assert_eq!(expect_item_text(&items[4], "消す"), "消す");
    assert!(
        matches!(model.command_of(items[4].id()), Some(TrayCommand::DismissFor(target)) if target == index),
        "消す → DismissFor(index)"
    );

    // pause 中は「再開」ラベル（command は同一）
    let paused_model = TrayMenuModel::build_popup(index, &sets, &menu_items, true, &ja_lang());
    let paused_items = paused_model.menu().items();
    assert_eq!(
        expect_item_text(&paused_items[3], "再開"),
        "再開する",
        "is_paused でラベルが切替"
    );
    assert!(
        matches!(paused_model.command_of(paused_items[3].id()), Some(TrayCommand::TogglePauseFor(target)) if target == index),
    );
    assert_eq!(expect_item_text(&paused_items[4], "消す"), "消す");
    assert!(
        matches!(paused_model.command_of(paused_items[4].id()), Some(TrayCommand::DismissFor(target)) if target == index),
    );
}

/// 未知 id / 非コマンド id → None（Option 契約の None 側）。
#[test]
fn command_of_unknown_id_returns_none() {
    let empty: Vec<String> = vec![];
    let model = TrayMenuModel::build_tray(
        &empty,
        &allowed(true, true, true, true, true, true),
        &ja_lang(),
    );
    let unknown = MenuId::new("tray_test_unknown_id");
    assert!(model.command_of(&unknown).is_none(), "未知 id → None");
}

// =====================================================================
// 契約 C-3: apply_tray_command — TrayCommand の Manager / Settings への適用
// =====================================================================

/// Spawn(Some(set)) → manager.request_spawn(&set)（次 tick で drain・該当 set）。
#[test]
fn apply_spawn_some_spawns_requested_set() {
    let home = TempHome::new("apply_spawn_some");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.set_image_set_resolver(resolver_for(&["Alpha", "Beta"]));
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &["Alpha", "Beta"]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::Spawn(Some("Beta".to_string())),
        &context,
    );
    manager.tick(Instant::now());

    assert_eq!(
        manager.count_of("Beta"),
        1,
        "要求 set の spawn が次 tick で反映"
    );
    assert_eq!(manager.count(), 1);
}

/// Spawn(None) → manager.request_spawn_random(&context.image_sets)。
/// unit rng（全て 0.5）: 0.5*2 = 1.0 → (int) 1 → "Beta"（先頭 set 固定の
/// 誤実装なら Alpha になり失敗する）。
#[test]
fn apply_spawn_none_uses_random_selection_over_context_sets() {
    let home = TempHome::new("apply_spawn_none");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.set_image_set_resolver(resolver_for(&["Alpha", "Beta"]));
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &["Alpha", "Beta"]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::Spawn(None),
        &context,
    );
    manager.tick(Instant::now());

    assert_eq!(
        manager.count_of("Beta"),
        1,
        "context の image_sets からランダム選択"
    );
    assert_eq!(manager.count(), 1);
}

/// Spawn(None) + context.image_sets 空 → request_spawn_random の no-op 経路
///（rng 不消費 = 空 rng で panic しない）。
#[test]
fn apply_spawn_none_with_empty_sets_is_noop_without_rng() {
    let home = TempHome::new("apply_spawn_none_empty");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new(),
        fixed_rng(vec![]),
    );
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::Spawn(None),
        &context,
    );
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "set 0 件 → no-op");
}

/// FollowCursor → manager.set_behavior_all("ChaseMouse")（全員）。
#[test]
fn apply_follow_cursor_sets_chase_mouse_on_all() {
    let home = TempHome::new("apply_follow");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(
        env,
        table(vec![row("Walk", 100), row("ChaseMouse", 100)]),
        ScriptedFactory::new(),
    );
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.add(mascot_of_set("TestSet", (20, 600)));
    manager.tick(Instant::now());
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::FollowCursor,
        &context,
    );
    let snap = snapshot(&mut manager);
    assert!(
        snap.iter()
            .all(|v| v.behavior.as_deref() == Some("ChaseMouse")),
        "全員に ChaseMouse が指示される"
    );
}

/// ReduceToOne → manager.remain_one()（先頭 1 体を残し他は dispose・削除は次 tick）。
#[test]
fn apply_reduce_to_one_keeps_first_mascot() {
    let home = TempHome::new("apply_reduce");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.add(mascot_of_set("TestSet", (20, 600)));
    manager.add(mascot_of_set("TestSet", (30, 700)));
    manager.tick(Instant::now());
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::ReduceToOne,
        &context,
    );

    let snap = snapshot(&mut manager);
    assert!(!snap[0].remove_pending, "先頭 1 体は残る");
    assert_eq!(
        snap.iter().filter(|v| v.remove_pending).count(),
        2,
        "残り 2 体は dispose 扱い"
    );
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 1, "次 tick で 1 体になる");
}

/// RestoreWindows → manager.restore_windows()（画面外の窓を work area へ）。
#[test]
fn apply_restore_windows_moves_offscreen_windows() {
    let home = TempHome::new("apply_restore");
    let (env, state) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.tick(Instant::now()); // screen union 更新（restore の intersects 判定用）
    state.borrow_mut().windows = vec![(7, rect(5000, 100, 5400, 300))];
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::RestoreWindows,
        &context,
    );
    assert_eq!(
        *state.borrow().moved,
        [(7, 25, 25)],
        "画面外の窓が work area + 25 へ"
    );
    assert_eq!(*state.borrow().raised, [7]);
}

/// SetAllowed（env 連動 5 種）: ①settings 更新 ②settings.toml への即時保存
///（実パースで pin）③Environment への passthrough。対象以外は既定 true のまま。
#[test]
fn apply_set_allowed_updates_settings_saves_and_passes_through() {
    type Getter = fn(&dyn EnvironmentView) -> bool;
    let cases: [(&'static str, Getter); 5] = [
        ("Breeding", |v: &dyn EnvironmentView| v.breeding_allowed()),
        ("Transients", |v: &dyn EnvironmentView| {
            v.transients_enabled()
        }),
        ("Transformation", |v: &dyn EnvironmentView| {
            v.transformation_allowed()
        }),
        ("Throwing", |v: &dyn EnvironmentView| v.throwing_allowed()),
        ("Multiscreen", |v: &dyn EnvironmentView| v.multiscreen()),
    ];
    let all_names = [
        "Breeding",
        "Transients",
        "Transformation",
        "Throwing",
        "Sounds",
        "Multiscreen",
    ];
    for (name, getter) in cases {
        let home = TempHome::new(&format!("apply_allowed_{name}"));
        let (env, _) = single_monitor_env();
        let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
        let mut settings = Settings::default();
        let context = make_apply_context(&home, &[]);

        let command = TrayCommand::SetAllowed(kind_by_name(name), false);
        apply_tray_command(&mut manager, &mut settings, command, &context);

        assert!(
            !getter(manager.environment_view()),
            "{name}: Environment へ passthrough される"
        );
        assert!(
            !allowed_field_by_name(&settings.allowed, name),
            "{name}: settings の対応フィールドが更新される"
        );

        // 即時保存されたファイルを実パースして pin
        let saved = Settings::load(&home.settings_path())
            .expect("SetAllowed で settings.toml が即時保存される");
        assert!(
            !allowed_field_by_name(&saved.allowed, name),
            "{name}: ファイルへ即時保存されている"
        );
        for other in all_names {
            if other != name {
                assert!(
                    allowed_field_by_name(&saved.allowed, other),
                    "{name}: 対象外の {other} は既定 true のまま"
                );
            }
        }
    }
}

/// SetAllowed(Sounds): env passthrough は存在しない（Phase 1 no-op）が
/// settings 更新 + 即時保存は行われる。
#[test]
fn apply_set_allowed_sounds_updates_settings_and_file_without_env_change() {
    let home = TempHome::new("apply_allowed_sounds");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetAllowed(AllowedKind::Sounds, false),
        &context,
    );

    assert!(
        !settings.allowed.sounds,
        "Sounds は settings のみ更新（env passthrough は契約に無い）"
    );
    let saved = Settings::load(&home.settings_path()).expect("即時保存される");
    assert!(!saved.allowed.sounds);
    assert!(
        manager.environment_view().breeding_allowed(),
        "無関係の env 状態は不変"
    );
}

/// save 失敗（保存先ディレクトリ不在）でも panic しない: settings 更新と
/// passthrough は続行する（Err → log::error して続行）。
#[test]
fn apply_set_allowed_survives_save_failure_without_panic() {
    let home = TempHome::new("apply_allowed_save_err");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    let mut settings = Settings::default();
    let context = TrayContext {
        conf_dir: home.root.join("no_such_dir"), // 保存先不在 → save は Err
        img_dir: home.root.clone(),
        image_sets: vec![],
    };

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetAllowed(AllowedKind::Throwing, false),
        &context,
    );

    assert!(!settings.allowed.throwing, "settings 更新は反映される");
    assert!(
        !manager.environment_view().throwing_allowed(),
        "save 失敗後も passthrough は続行する"
    );
    assert!(
        !home.root.join("no_such_dir").join("settings.toml").exists(),
        "失敗した保存でファイルが半端に作られない"
    );
}

/// TogglePauseAll → 全員 pause / 再適用で復帰。DismissAll → 全員 dispose 扱い
///（削除は次 tick）・apply 内では exit 操作しない（should_exit は false）。
#[test]
fn apply_toggle_pause_all_then_dismiss_all_pends_everyone() {
    let home = TempHome::new("apply_pause_dismiss");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.add(mascot_of_set("TestSet", (20, 600)));
    manager.tick(Instant::now());
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::TogglePauseAll,
        &context,
    );
    assert!(manager.is_paused(), "全員 pause");
    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::TogglePauseAll,
        &context,
    );
    assert!(!manager.is_paused(), "2 回で復帰");

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::DismissAll,
        &context,
    );
    let snap = snapshot(&mut manager);
    assert!(snap.iter().all(|v| v.remove_pending), "全員 dispose 扱い");
    assert_eq!(manager.count(), 2, "削除は次 tick");
    assert!(
        !manager.should_exit(),
        "apply 内では exit 操作しない（exit は次 tick の should_exit 経由）"
    );
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "次 tick で全員除去");
}

/// Reload: load_materials(conf_dir, img_dir, settings.scales()) → manager.reload。
/// 実素材（temp conf/img）+ scale 0.5 で mascot が新画像セットへ付け替わる
///（32x24 → 16x12 = scales 注入の pin・注入漏れなら 32x24 のまま）。
#[test]
fn apply_reload_reloads_materials_with_scales_from_settings() {
    let home = TempHome::new("apply_reload_ok");
    home.write_valid_conf();
    home.write_png("SetA", "walk1.png", 32, 24);
    home.write_png("SetB", "b1.png", 8, 8);

    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set_with("SetA", "pose.png", 64, 64, (10, 500)));
    manager.tick(Instant::now());

    let mut settings = Settings::default();
    settings.imagesets.scale.insert("SetA".to_string(), 0.5);
    let context = TrayContext {
        conf_dir: home.conf_dir(),
        img_dir: home.img_dir(),
        image_sets: vec!["SetA".to_string(), "SetB".to_string()],
    };

    apply_tray_command(&mut manager, &mut settings, TrayCommand::Reload, &context);

    let snap = snapshot(&mut manager);
    assert_eq!(snap.len(), 1, "Reload で mascot は消えない");
    assert_eq!(
        snap[0].frame_dims,
        Some((16, 12)),
        "Reload 後は実 PNG（32x24）へ付け替わり、settings.scales()（0.5）が効く（16x12）"
    );
    assert!(!snap[0].remove_pending);
}

/// Reload の load_materials Err（conf 破損）→ 現状維持（manager へ何もしない・
/// panic しない）。
#[test]
fn apply_reload_keeps_manager_untouched_on_load_error() {
    let home = TempHome::new("apply_reload_err");
    home.write_valid_conf();
    home.write_conf("actions.xml", "<WrongRoot/>"); // conf 失敗 = 素材全体の中止
    home.write_png("SetA", "walk1.png", 32, 24);

    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set_with("SetA", "pose.png", 64, 64, (10, 500)));
    manager.tick(Instant::now());

    let mut settings = Settings::default();
    let context = TrayContext {
        conf_dir: home.conf_dir(),
        img_dir: home.img_dir(),
        image_sets: vec!["SetA".to_string()],
    };

    apply_tray_command(&mut manager, &mut settings, TrayCommand::Reload, &context);

    let snap = snapshot(&mut manager);
    assert_eq!(snap.len(), 1, "Err → 現状維持");
    assert_eq!(
        snap[0].frame_dims,
        Some((64, 64)),
        "Err → manager へは何もしない"
    );
    assert!(!snap[0].remove_pending);
}

/// popup 個別コマンド: SetBehaviorFor / TogglePauseFor / DismissFor は
/// index の mascot のみに効く（他は無傷・削除は次 tick）。
#[test]
fn apply_individual_commands_target_only_that_mascot() {
    let home = TempHome::new("apply_individual");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.add(mascot_of_set("TestSet", (20, 600)));
    manager.add(mascot_of_set("TestSet", (30, 700)));
    manager.tick(Instant::now());
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetBehaviorFor(1, "Walk".to_string()),
        &context,
    );
    let snap = snapshot(&mut manager);
    assert_eq!(
        snap[1].behavior.as_deref(),
        Some("Walk"),
        "index 1 のみ setBehavior"
    );
    assert_eq!(snap[0].behavior, None);
    assert_eq!(snap[2].behavior, None);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::TogglePauseFor(0),
        &context,
    );
    let snap = snapshot(&mut manager);
    assert!(snap[0].paused, "index 0 のみ pause");
    assert!(!snap[1].paused && !snap[2].paused);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::DismissFor(2),
        &context,
    );
    let snap = snapshot(&mut manager);
    assert!(snap[2].remove_pending, "index 2 のみ dispose 扱い");
    assert!(!snap[0].remove_pending && !snap[1].remove_pending);

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2, "index 2 が次 tick で除去");
}

/// popup 個別コマンドの index 範囲外 → warn + no-op（panic しない・何も壊さない）。
#[test]
fn apply_individual_commands_out_of_range_are_noop() {
    let home = TempHome::new("apply_individual_oor");
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env, table(vec![row("Walk", 100)]), ScriptedFactory::new());
    manager.add(mascot_of_set("TestSet", (10, 500)));
    manager.tick(Instant::now());
    let mut settings = Settings::default();
    let context = make_apply_context(&home, &[]);

    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::SetBehaviorFor(9, "Walk".to_string()),
        &context,
    );
    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::TogglePauseFor(9),
        &context,
    );
    apply_tray_command(
        &mut manager,
        &mut settings,
        TrayCommand::DismissFor(9),
        &context,
    );

    let snap = snapshot(&mut manager);
    assert_eq!(snap.len(), 1, "mascot は増減しない");
    assert_eq!(snap[0].behavior, None, "behavior は変わらない");
    assert!(!snap[0].paused);
    assert!(!snap[0].remove_pending);
}

// =====================================================================
// タスク #20: settings.toml の scale 検証（非有限 / ≤ 0 → Err）
// =====================================================================

/// `[imagesets] scale = { Shimeji = <value> }` のみの settings.toml を書く。
fn write_single_scale(home: &TempHome, value: &str) {
    let text = format!("[imagesets]\nscale = {{ Shimeji = {value} }}\n");
    std::fs::write(home.settings_path(), text).expect("settings.toml を書ける");
}

/// 契約: scale が 0 または負 → SettingsError（設定不正として起動エラー経路）。
#[test]
fn settings_load_rejects_non_positive_scale() {
    for (index, value) in ["0.0", "-1.5", "-0.0"].iter().enumerate() {
        let home = TempHome::new(&format!("scale_nonpos_{index}"));
        write_single_scale(&home, value);
        assert!(
            Settings::load(&home.settings_path()).is_err(),
            "scale {value} は Err になるべき"
        );
    }
}

/// 契約: scale が非有限（NaN / +inf / -inf）→ SettingsError。
#[test]
fn settings_load_rejects_non_finite_scale() {
    for (index, value) in ["nan", "inf", "-inf"].iter().enumerate() {
        let home = TempHome::new(&format!("scale_nonfinite_{index}"));
        write_single_scale(&home, value);
        assert!(
            Settings::load(&home.settings_path()).is_err(),
            "scale {value}（非有限）は Err になるべき"
        );
    }
}

/// 回帰: 正の有限 scale（0.5 / 2.0）は Ok で値も保持される。
/// 上限は設けない仕様のため巨大な有限値（1e9）も Ok（フレーム単位スキップで安全化）。
#[test]
fn settings_load_accepts_positive_finite_scale_without_upper_bound() {
    let home = TempHome::new("scale_ok");
    let text = concat!(
        "[imagesets]\n",
        "scale = { A = 0.5, B = 2.0, C = 1000000000.0 }\n",
    );
    std::fs::write(home.settings_path(), text).expect("settings.toml を書ける");

    let settings = Settings::load(&home.settings_path()).expect("正の有限 scale は Ok");
    assert_eq!(settings.scales().get("A"), Some(&0.5));
    assert_eq!(settings.scales().get("B"), Some(&2.0));
    assert_eq!(settings.scales().get("C"), Some(&1e9), "上限は設けない");
}

// =====================================================================
// タスク #23: トレイアイコンの Java 準拠化（Main.getIcon L764-792）
// =====================================================================
//
// Java `Main.getIcon()`（.tmp/java-ref/Main.java L764-792）の仕様:
// ① `img/icon.png` があればユーザーカスタムとして読込
// ② 無ければ jar 同梱 `/icon.png`（16×16・32bpp ARGB）を使用
// ③ 両方失敗時は空 16×16
//
// Rust 側の公開契約（coder が `src/tray.rs` に実装・シグネチャは本テストが固定）:
//   pub fn load_tray_icon_rgba(custom_path: &Path) -> (Vec<u8>, u32, u32)
//     - カスタム PNG が存在しデコード可能 → その RGBA8 と (幅, 高さ)
//     - 存在しない / デコード失敗 → 埋め込み既定（assets/icon.png・16×16）の RGBA8
//     - 常に有効な RGBA を返し panic しない
//
// ここで pin するのは「入力 → 戻り値」の振る舞いのみ:
// - 有効なカスタム PNG（16×16 と区別できる 2×2・既知ピクセル）
//   → その寸法とピクセルが返る（カスタム優先の証明）
// - 存在しないパス → 既定 16×16・RGBA 長 1024
// - 壊れた入力 → panic せず既定 16×16・RGBA 長 1024
// エラーメッセージ・log 呼び出し・内部構造は検証しない（testing-guidelines §1）。

/// カスタム tray アイコン用 PNG を temp dir 配下に書き、そのパスを返す。
fn write_icon_png(home: &TempHome, name: &str, image: &image::RgbaImage) -> PathBuf {
    let path = home.root.join(name);
    image.save(&path).expect("テンポラリ PNG を書ける");
    path
}

/// 有効なカスタム PNG（2×2・既知の異なるピクセル）→ その寸法とピクセルが返る。
/// 既定は 16×16 のため、2×2 が返ることはカスタム優先の証明になる。
#[test]
fn tray_icon_uses_valid_custom_png_pixels_and_dimensions() {
    let home = TempHome::new("icon_custom");
    let mut custom = image::RgbaImage::new(2, 2);
    custom.put_pixel(0, 0, image::Rgba([1, 2, 3, 255]));
    custom.put_pixel(1, 0, image::Rgba([4, 5, 6, 255]));
    custom.put_pixel(0, 1, image::Rgba([7, 8, 9, 128]));
    custom.put_pixel(1, 1, image::Rgba([10, 11, 12, 0]));
    let path = write_icon_png(&home, "icon.png", &custom);

    let (rgba, width, height) = simeji::tray::load_tray_icon_rgba(&path);

    assert_eq!(
        (width, height),
        (2, 2),
        "カスタム PNG の寸法が返る（既定 16×16 と区別できる）"
    );
    assert_eq!(
        rgba,
        vec![1, 2, 3, 255, 4, 5, 6, 255, 7, 8, 9, 128, 10, 11, 12, 0],
        "カスタム PNG の RGBA8（行優先）が返る"
    );
}

/// カスタムパスが存在しない → 埋め込み既定（16×16・RGBA 長 1024）が返る。
/// 存在しないディレクトリ配下でも panic しない。
#[test]
fn tray_icon_missing_custom_falls_back_to_embedded_default() {
    let home = TempHome::new("icon_missing");
    let missing = home.root.join("no_such_dir").join("icon.png");

    let (rgba, width, height) = simeji::tray::load_tray_icon_rgba(&missing);

    assert_eq!((width, height), (16, 16), "既定アイコンは 16×16");
    assert_eq!(rgba.len(), 16 * 16 * 4, "既定アイコンは RGBA 長 1024");
}

/// カスタムが壊れている（PNG でないバイト列）→ panic せず埋め込み既定
/// （16×16・RGBA 長 1024）が返る（デコード失敗のフォールバック）。
#[test]
fn tray_icon_corrupt_custom_falls_back_without_panic() {
    let home = TempHome::new("icon_corrupt");
    let path = home.root.join("icon.png");
    std::fs::write(&path, b"this is definitely not a PNG file").expect("壊れたバイト列を書ける");

    let (rgba, width, height) = simeji::tray::load_tray_icon_rgba(&path);

    assert_eq!((width, height), (16, 16), "デコード失敗 → 既定 16×16");
    assert_eq!(
        rgba.len(),
        16 * 16 * 4,
        "デコード失敗 → 既定の RGBA 長 1024"
    );
}

// =====================================================================
// 契約 D: interactive_windows（アクティブウィンドウ選別の whitelist/blacklist）
// =====================================================================
//
// 追加される公開インターフェース（coder が src/tray.rs に実装・シグネチャは
// 本テストが固定）:
// ```text
// #[derive(Debug, Default, Serialize, Deserialize)]
// #[serde(default)]
// pub struct InteractiveWindowsSettings {
//     pub whitelist: Vec<String>,
//     pub blacklist: Vec<String>,
// }
// // Settings に追加:
// #[serde(default)]
// pub interactive_windows: InteractiveWindowsSettings,
// ```
// TOML 形状:
// ```toml
// [interactive_windows]
// whitelist = ["メモ帳", "Visual Studio Code"]
// blacklist = []
// ```
//
// Win32 選別（`is_interactive_by_title`）は tests/os_source_test.rs が既存で網羅
// 済みのため本節では扱わない（重複禁止）。ここは設定スキーマ（TOML 入出力）の
// 契約のみを pin する。

/// [interactive_windows] の whitelist/blacklist が記載順のまま
/// `Settings.interactive_windows` に入る（Vec の順序を保存・並べ替えや除去をしない）。
#[test]
fn settings_load_reads_interactive_windows_lists_in_order() {
    let home = TempHome::new("iw_load_order");
    let sample = concat!(
        "[interactive_windows]\n",
        "whitelist = [\"メモ帳\", \"Visual Studio Code\", \"Notepad\"]\n",
        "blacklist = [\"Chrome\", \"Explorer\"]\n",
    );
    std::fs::write(home.settings_path(), sample).expect("settings.toml を書ける");

    let settings = Settings::load(&home.settings_path()).expect("interactive_windows を読める");
    assert_eq!(
        settings.interactive_windows.whitelist,
        [
            "メモ帳".to_string(),
            "Visual Studio Code".to_string(),
            "Notepad".to_string()
        ],
        "whitelist は記載順のまま（並べ替え・除去なし）"
    );
    assert_eq!(
        settings.interactive_windows.blacklist,
        ["Chrome".to_string(), "Explorer".to_string()],
        "blacklist は記載順のまま"
    );
}

/// 後方互換: [interactive_windows] を持たない既存形式 TOML を load しても
/// エラーにならず、whitelist/blacklist は両方空・既存フィールドは従来どおり読める。
#[test]
fn settings_load_without_interactive_windows_section_is_backward_compatible() {
    let home = TempHome::new("iw_legacy");
    let legacy = concat!(
        "[general]\n",
        "language = \"en\"\n",
        "\n",
        "[allowed]\n",
        "throwing = false\n",
        "\n",
        "[disabled_behaviors]\n",
        "Shimeji = [\"Walk\"]\n",
        "\n",
        "[imagesets]\n",
        "scale = { Shimeji = 0.5 }\n",
    );
    std::fs::write(home.settings_path(), legacy).expect("旧形式 TOML を書ける");

    let settings = Settings::load(&home.settings_path()).expect("旧形式 TOML を読める");
    assert!(
        settings.interactive_windows.whitelist.is_empty(),
        "セクション欠落 → whitelist 空"
    );
    assert!(
        settings.interactive_windows.blacklist.is_empty(),
        "セクション欠落 → blacklist 空"
    );
    assert_eq!(
        settings.general.language, "en",
        "general は従来どおり読める"
    );
    assert!(!settings.allowed.throwing, "allowed は従来どおり読める");
    assert!(settings.allowed.sounds, "未記載 allowed は既定 true");
    assert_eq!(
        settings
            .disabled_behaviors
            .get("Shimeji")
            .map(Vec::as_slice),
        Some(&["Walk".to_string()][..]),
        "disabled_behaviors は従来どおり読める"
    );
    assert_eq!(
        settings.scales().get("Shimeji"),
        Some(&0.5),
        "imagesets は従来どおり読める"
    );
}

/// Settings::default() および InteractiveWindowsSettings::default() は両方空。
#[test]
fn settings_default_interactive_windows_is_empty() {
    let settings = Settings::default();
    assert!(
        settings.interactive_windows.whitelist.is_empty(),
        "Settings::default() の whitelist は空"
    );
    assert!(
        settings.interactive_windows.blacklist.is_empty(),
        "Settings::default() の blacklist は空"
    );

    let iw = InteractiveWindowsSettings::default();
    assert!(
        iw.whitelist.is_empty(),
        "InteractiveWindowsSettings 既定 whitelist 空"
    );
    assert!(
        iw.blacklist.is_empty(),
        "InteractiveWindowsSettings 既定 blacklist 空"
    );
}

/// save → load の往復で interactive_windows が保存される（空・非空いずれも）。
#[test]
fn settings_round_trip_preserves_interactive_windows() {
    // 空リスト: 既定値の往復
    let home = TempHome::new("iw_roundtrip_empty");
    let default_path = home.settings_path();
    Settings::save(&default_path, &Settings::default()).expect("空設定を save できる");
    let loaded = Settings::load(&default_path).expect("空設定を load できる");
    assert!(
        loaded.interactive_windows.whitelist.is_empty()
            && loaded.interactive_windows.blacklist.is_empty(),
        "空リストも往復で空のまま"
    );

    // 非空リスト: 値がそのまま戻る
    let home2 = TempHome::new("iw_roundtrip_values");
    let original = Settings {
        interactive_windows: InteractiveWindowsSettings {
            whitelist: vec!["メモ帳".to_string(), "  padded  ".to_string()],
            blacklist: vec!["Chrome".to_string()],
        },
        ..Settings::default()
    };
    let path = home2.settings_path();
    Settings::save(&path, &original).expect("非空設定を save できる");
    let loaded = Settings::load(&path).expect("非空設定を load できる");
    assert_settings_eq(&original, &loaded);
    assert_eq!(
        loaded.interactive_windows.whitelist, original.interactive_windows.whitelist,
        "whitelist が往復で保存される"
    );
    assert_eq!(
        loaded.interactive_windows.blacklist, original.interactive_windows.blacklist,
        "blacklist が往復で保存される"
    );
}

/// セクションはあるが片方のキーだけ記載 → もう片方は空で補完される。
#[test]
fn settings_load_with_only_one_interactive_windows_key_leaves_other_empty() {
    // whitelist のみ
    let home = TempHome::new("iw_only_white");
    std::fs::write(
        home.settings_path(),
        "[interactive_windows]\nwhitelist = [\"Notepad\"]\n",
    )
    .expect("settings.toml を書ける");
    let settings = Settings::load(&home.settings_path()).expect("片キー TOML を読める");
    assert_eq!(
        settings.interactive_windows.whitelist,
        ["Notepad".to_string()],
        "記載済み whitelist は読まれる"
    );
    assert!(
        settings.interactive_windows.blacklist.is_empty(),
        "欠落 blacklist は空で補完"
    );

    // blacklist のみ
    let home2 = TempHome::new("iw_only_black");
    std::fs::write(
        home2.settings_path(),
        "[interactive_windows]\nblacklist = [\"Chrome\"]\n",
    )
    .expect("settings.toml を書ける");
    let settings = Settings::load(&home2.settings_path()).expect("片キー TOML を読める");
    assert!(
        settings.interactive_windows.whitelist.is_empty(),
        "欠落 whitelist は空で補完"
    );
    assert_eq!(
        settings.interactive_windows.blacklist,
        ["Chrome".to_string()],
        "記載済み blacklist は読まれる"
    );
}

/// 各項目はそのまま保持（trim・空要素除去などの加工をしない）。空白要素は許容し、
/// 一致判定上の扱いは `is_interactive_by_title` が担う（os_source_test 側で網羅）。
#[test]
fn settings_load_preserves_interactive_windows_entries_verbatim() {
    let home = TempHome::new("iw_verbatim");
    let sample = concat!(
        "[interactive_windows]\n",
        "whitelist = [\"  spaced  \", \"\", \"x\"]\n",
        "blacklist = [\" \", \"Chrome \"]\n",
    );
    std::fs::write(home.settings_path(), sample).expect("settings.toml を書ける");

    let settings = Settings::load(&home.settings_path()).expect("interactive_windows を読める");
    assert_eq!(
        settings.interactive_windows.whitelist,
        ["  spaced  ".to_string(), String::new(), "x".to_string()],
        "空白・空文字を含む項目も trim せずそのまま保持"
    );
    assert_eq!(
        settings.interactive_windows.blacklist,
        [" ".to_string(), "Chrome ".to_string()],
        "末尾空白を含む項目もそのまま保持"
    );
}
