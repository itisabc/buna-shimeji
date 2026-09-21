//! タスク #9d: Reload（素材ローダ + 参照付け替え）の契約テスト（RED）。
//!
//! 設計正本: .tmp/design.md §1.10（#9 確定事項）+ §2 L370-380 付近（Reload 方針）。
//! Java `reloadAllImageSets`（.tmp/java-ref/Main.java L547-566）の「全消し + 再作成」は
//! **採用しない**（design 承認済みの意図的差異・参照付け替え路線）ため、
//! Java 全消しを pin するテストは書かない。
//!
//! 実装（#9d 分）未着手のため cargo test は compile error = RED が正常
//! （E0432: `shimeji::app::reload` の未存在 item / E0599:
//! `Mascot::rebind_image_set`・`Mascot::image_set`・`Manager::reload` の未存在メソッド）。
//!
//! pin する API 契約（coder への指示・シグネチャは tests が固定する）:
//!
//! ```text
//! // ---- src/app/reload.rs ----
//! pub struct ReloadMaterial {      // 公開フィールド
//!     pub name: String,
//!     pub image_set: Arc<ImageSet>,
//!     pub table: BehaviorTable,
//! }
//!
//! #[derive(Debug, Error)]          // thiserror・Display は日本語
//! pub enum MaterialError {
//!     #[error("...")]
//!     Config(#[from] ConfigError),
//!     #[error("...")]
//!     Imageset(#[from] ImagesetError),
//! }
//!
//! pub fn load_materials(
//!     conf_dir: &Path,
//!     img_dir: &Path,
//!     scales: &HashMap<String, f64>,
//! ) -> Result<Vec<ReloadMaterial>, MaterialError>
//! // - actions.xml / behaviors.xml の parse 失敗 → Err（ConfigError 伝播・素材は 1 つも
//! //   返さない）
//! // - validate_required_behaviors 失敗 → Err 伝播
//! // - enumerate_sets(img_dir) 不在 → Err 伝播。列挙は辞書順 = ReloadMaterial の順 =
//! //   「既定 set = 先頭」
//! // - 各 set: ImageSet::load(img_dir, set, scales.get(set).copied()) 成功分だけ
//! //   material 化。load 失敗 set は warn ログ + スキップ（自動テストで pin しない・
//! //   coder 実装+報告）。scales に無い set は None（等倍）
//! // - 各 set で available_refs + check_references を実行し report.warnings を
//! //   log::warn（欠落参照アニメの実無効化適用は #10・ここでは警告のみ）
//! // - table は BehaviorTable::new(&behaviors) の set 毎所有 copy
//! // - 空 img_dir（set 0 件）→ Ok(空 Vec)
//!
//! // ---- src/mascot/mod.rs ----
//! impl Mascot {
//!     // image_set_name と Arc<ImageSet> の 2 フィールド差し替えのみ。
//!     // behavior/anchor/look_right/time/paused/dragging/needs_repaint は一切不変
//!     pub fn rebind_image_set(
//!         &mut self,
//!         image_set_name: impl Into<String>,
//!         image_set: Arc<ImageSet>,
//!     )
//!     // Arc の deref 参照（外部観測点・現行は pub(crate) field のみで観測不能）
//!     pub fn image_set(&self) -> &ImageSet
//! }
//!
//! // ---- src/app/manager.rs ----
//! impl Manager {
//!     // 空 materials: 全 mascot dispose 扱い（remove_pending・次 tick の retain で
//!     //   除去）+ set_tables クリア + base table 変更なし
//!     // 非空 materials:
//!     //   1. base table を materials[0].table で置換し、set_tables を全消しの上で
//!     //      全 materials 分を再登録（既定 set の分は base と set_tables の両方・
//!     //      古い stale エントリは残らない）
//!     //   2. 各 mascot（index 順）: 自身の set が materials に残存 → 自 set 維持 /
//!     //      消滅 → materials[0] の set へ rebind_image_set（image_set_name も新 set 名）
//!     //   3. behavior 再構築: 現在の behavior_name() が「付け替え後 set の table」に
//!     //      存在 かつ enabled（is_behavior_enabled の同一式判定・pre-check により
//!     //      build_behavior の再配置分岐は不通 = 同名再構築経路で rng を消費しない）
//!     //      → 同名再構築。存在しない / 無効 → build_next_behavior(None) で再選択。
//!     //      どちらも新 runner で進行リセット（anchor/look_right/paused/dragging 維持）
//!     //   4. 構築 Err / set_behavior Err → log + その mascot のみ dispose（他は無傷）
//!     // - Environment の disabled map は変更しない
//!     pub fn reload(&mut self, materials: Vec<ReloadMaterial>)
//! }
//! ```
//!
//! 自己完結（tests/common 不使用）。app_manager_ext_test.rs の deterministic Rng fake /
//! FakeSource / ScriptedFactory パターンと、config_parse_test.rs / imageset_test.rs の
//! temp dir + 実 XML + PNG 生成パターンを踏襲する。

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use shimeji::app::environment::{Environment, OsSource};
use shimeji::app::manager::Manager;
use shimeji::app::reload::{load_materials, MaterialError, ReloadMaterial};
use shimeji::config::script::EvalError;
use shimeji::config::{
    ActionDef, ActionsConfig, BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap,
};
use shimeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use shimeji::mascot::{Mascot, Rect, Rng};
use shimeji::render::imageset::{Frame, ImageSet};
use shimeji::tint::{TintMode, TintStyle};

// =====================================================================
// 合成データヘルパ（自己完結）
// =====================================================================

/// 矩形ヘルパ。
fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

/// 単一フレームだけを持つ ImageSet（Arc 交換の観測用: フレーム寸法で新旧を判別する）。
fn image_set_with(name: &str, file: &str, width: u32, height: u32) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::from([(
            file.to_string(),
            Frame::from_rgba(width, height, vec![0; width as usize * height as usize * 4]),
        )]),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

/// フレーム無し ImageSet（mascot 初期生成用・app_manager_ext_test 踏襲）。
fn empty_image_set(name: &str) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
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

/// 単一モニタ構成の Environment（monitor (0,0,1920,1080) / work area (0,0,1920,1040)）。
fn single_monitor_env() -> Environment {
    let state = Rc::new(RefCell::new(FakeState {
        monitors: vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))],
        cursor: None,
        active_window: None,
        windows: Vec::new(),
        moved: Vec::new(),
        raised: Vec::new(),
    }));
    Environment::new(FakeSource {
        state: state.clone(),
    })
}

// =====================================================================
// Manager fixture（scripted action + 合成 config）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、枯渇 = 過剰消費として
/// panic する（app_manager_ext_test.rs 踏襲・rng 消費回数の pin に使う）。
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

fn eval_error() -> EvalError {
    EvalError {
        expr: "test-expr".to_string(),
        message: "テスト用の評価エラー".to_string(),
    }
}

/// action メソッドの init 戻り値計画。
#[derive(Clone)]
enum Plan {
    Ok,
    Eval,
}

/// ScriptedAction の振る舞い仕様。既定 = 常時継続・正常・rng を消費しない。
#[derive(Clone)]
struct ActionScript {
    has_next_true_times: usize,
    init: Plan,
}

impl Default for ActionScript {
    fn default() -> Self {
        ActionScript {
            has_next_true_times: usize::MAX,
            init: Plan::Ok,
        }
    }
}

struct ScriptedAction {
    script: ActionScript,
    has_next_calls: usize,
}

impl Action for ScriptedAction {
    fn init(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn shimeji::mascot::EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        match self.script.init {
            Plan::Ok => Ok(()),
            Plan::Eval => Err(ActionError::Eval(eval_error())),
        }
    }

    fn has_next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn shimeji::mascot::EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.has_next_calls += 1;
        Ok(self.has_next_calls <= self.script.has_next_true_times)
    }

    fn next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn shimeji::mascot::EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}

/// config 駆動ファクトリ。`fail_on` に含まれる action 参照名は構築 Err、
/// `init_err` に含まれる名前は init Err の action を返す（dispose 経路の pin 用）。
struct ScriptedFactory {
    scripts: HashMap<String, ActionScript>,
    fail_on: HashSet<String>,
}

impl ScriptedFactory {
    fn new() -> Self {
        ScriptedFactory {
            scripts: HashMap::new(),
            fail_on: HashSet::new(),
        }
    }

    /// 構築を Err にする action 参照名を登録する。
    fn with_fail(mut self, names: &[&str]) -> Self {
        self.fail_on.extend(names.iter().map(|n| (*n).to_string()));
        self
    }

    /// init を Err にする action 参照名を登録する。
    fn with_init_err(mut self, name: &str) -> Self {
        self.scripts.insert(
            name.to_string(),
            ActionScript {
                has_next_true_times: usize::MAX,
                init: Plan::Eval,
            },
        );
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
                let script = self.scripts.get(name).cloned().unwrap_or_default();
                Ok(Box::new(ScriptedAction {
                    script,
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
    BehaviorTable::new(&BehaviorsConfig {
        entries,
        ..Default::default()
    })
}

fn make_manager_with_rng(
    env: Environment,
    table: BehaviorTable,
    factory: ScriptedFactory,
    rng: Box<dyn Rng>,
) -> Manager {
    let mut manager = Manager::new(env, table, Box::new(factory), rng);
    manager.set_exit_on_last_removed(false);
    manager
}

/// 指定 set の新規 Mascot（fresh・behavior 無し）。
fn mascot_of_set(image_set: &str, anchor: (i32, i32)) -> Mascot {
    Mascot::new(image_set, empty_image_set(image_set), anchor)
}

/// ReloadMaterial を直接構築する（manager 側テストは loader と独立に検証する）。
fn material(name: &str, image_set: Arc<ImageSet>, entries: Vec<BehaviorEntry>) -> ReloadMaterial {
    ReloadMaterial {
        name: name.to_string(),
        image_set,
        table: table(entries),
        // manager 側テストは action 定義集合を使わない（空集合）
        actions: Arc::new(ActionsConfig::default()),
        disabled_animations: Vec::new(),
    }
}

/// tint 宣言付きの ReloadMaterial（スライス 3: set 別 tint の注入検証用）。
fn material_with_tint(
    name: &str,
    image_set: Arc<ImageSet>,
    entries: Vec<BehaviorEntry>,
    tint: TintStyle,
) -> ReloadMaterial {
    ReloadMaterial {
        name: name.to_string(),
        image_set,
        table: table(entries),
        actions: Arc::new(ActionsConfig {
            tint,
            ..Default::default()
        }),
        disabled_animations: Vec::new(),
    }
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
#[derive(Debug, Clone)]
struct MascotView {
    image_set: String,
    behavior: Option<String>,
    anchor: (i32, i32),
    look_right: bool,
    paused: bool,
    dragging: bool,
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
            dragging: m.is_dragging(),
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

fn find_set<'a>(snap: &'a [MascotView], set: &str) -> &'a MascotView {
    snap.iter()
        .find(|v| v.image_set == set)
        .unwrap_or_else(|| panic!("set {set} の mascot が見つからない"))
}

// =====================================================================
// 素材ローダ fixture（temp dir + 最小有効 XML + PNG・config_parse / imageset 踏襲）
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

/// 1 Action（`Walk`・Stay・pose 1 個）だけを持つ actions XML（#32 の set 別内容
/// 観察用・参照画像を差し替えて「同名 Action 別内容」を作る）。
fn actions_xml(image: &str) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n",
            "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
            "  <ActionList>\n",
            "    <Action Name=\"Walk\" Type=\"Stay\">\n",
            "      <Animation>\n",
            "        <Pose Image=\"{image}\" ImageAnchor=\"0,0\" Velocity=\"0,0\" Duration=\"1\"/>\n",
            "      </Animation>\n",
            "    </Action>\n",
            "  </ActionList>\n",
            "</Mascot>\n"
        ),
        image = image
    )
}

/// 必須 4 種 + `Walk` + `Sit` の behaviors XML（#32 の set 別 behaviors 観察用）。
const SET_BEHAVIORS_XML: &str = concat!(
    "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
    "  <BehaviorList>\n",
    "    <Behavior Name=\"ChaseMouse\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Fall\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Dragged\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Thrown\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Walk\" Frequency=\"1\"/>\n",
    "    <Behavior Name=\"Sit\" Frequency=\"1\"/>\n",
    "  </BehaviorList>\n",
    "</Mascot>\n"
);

/// conf/ + img/ を持つ使い捨てディレクトリ（Drop で再帰削除）。
struct TempAssets {
    root: PathBuf,
}

impl TempAssets {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("shimeji_t9d_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // 前回残留の掃除
        std::fs::create_dir_all(root.join("conf")).expect("conf ディレクトリを作れる");
        std::fs::create_dir_all(root.join("img")).expect("img ディレクトリを作れる");
        TempAssets { root }
    }

    fn conf_dir(&self) -> PathBuf {
        self.root.join("conf")
    }

    fn img_dir(&self) -> PathBuf {
        self.root.join("img")
    }

    fn write_conf(&self, name: &str, content: &str) {
        std::fs::write(self.conf_dir().join(name), content).expect("conf ファイルを書ける");
    }

    /// `conf/<set>/<name>` を書き込む（set 専用 conf・#32）。
    fn write_set_conf(&self, set: &str, name: &str, content: &str) {
        let dir = self.conf_dir().join(set);
        std::fs::create_dir_all(&dir).expect("set conf ディレクトリを作れる");
        std::fs::write(dir.join(name), content).expect("set conf ファイルを書ける");
    }

    /// `img/<set>/conf/<name>` を書き込む（探索順 1 番目・#32）。
    fn write_img_conf(&self, set: &str, name: &str, content: &str) {
        let dir = self.img_dir().join(set).join("conf");
        std::fs::create_dir_all(&dir).expect("img 側 conf ディレクトリを作れる");
        std::fs::write(dir.join(name), content).expect("img 側 conf ファイルを書ける");
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

impl Drop for TempAssets {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// =====================================================================
// 契約 A: load_materials（素材ローダ）
// =====================================================================

/// set 列挙は辞書順で、その順が ReloadMaterial の順かつ「既定 set = 先頭」を決める。
/// 各 material は set 名 / Arc<ImageSet> / set 毎所有の BehaviorTable
///（behaviors.xml 全行・XML 順）を持つ。
#[test]
fn load_materials_enumerates_sets_in_dictionary_order_with_owned_tables() {
    let assets = TempAssets::new("order");
    assets.write_valid_conf();
    assets.write_png("Shimeji", "walk1.png", 32, 24);
    assets.write_png("Alpha", "a1.png", 8, 8);

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
        .expect("有効な conf + 2 set でロードできる");

    let names: Vec<&str> = materials.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(
        names,
        ["Alpha", "Shimeji"],
        "set 列挙は辞書順 = ReloadMaterial の順"
    );
    assert_eq!(
        materials[0].name, "Alpha",
        "既定 set = 先頭（辞書順の最初）"
    );

    for m in &materials {
        assert_eq!(
            m.image_set.name, m.name,
            "material の image_set は当該 set のロード結果"
        );
        let rows: Vec<&str> = m.table.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            rows,
            ["ChaseMouse", "Fall", "Dragged", "Thrown", "Walk"],
            "table は behaviors.xml 全行（XML 頻・set 毎の所有 copy）"
        );
    }
    assert_eq!(
        materials[0]
            .image_set
            .frame("a1.png")
            .map(|f| (f.width, f.height)),
        Some((8, 8)),
        "Alpha のフレームは実 PNG 寸法"
    );
    assert_eq!(
        materials[1]
            .image_set
            .frame("walk1.png")
            .map(|f| (f.width, f.height)),
        Some((32, 24)),
        "Shimeji のフレームは実 PNG 寸法"
    );
}

/// scales.map の値が反映される: 指定 set はプリスケール（0.5 で寸法半減）、
/// 無指定 set は None（等倍）。
#[test]
fn load_materials_applies_scale_map_per_set() {
    let assets = TempAssets::new("scale");
    assets.write_valid_conf();
    assets.write_png("Scaled", "walk1.png", 32, 24);
    assets.write_png("Plain", "walk1.png", 8, 6);

    let mut scales = HashMap::new();
    scales.insert("Scaled".to_string(), 0.5);

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &scales)
        .expect("scale 指定ありでもロードできる");

    let plain = materials
        .iter()
        .find(|m| m.name == "Plain")
        .expect("Plain が material 化される");
    let scaled = materials
        .iter()
        .find(|m| m.name == "Scaled")
        .expect("Scaled が material 化される");

    assert_eq!(
        scaled
            .image_set
            .frame("walk1.png")
            .map(|f| (f.width, f.height)),
        Some((16, 12)),
        "scale 0.5 で画像フレーム寸法が半減する（既存 imageset テストの scale 検証踏襲）"
    );
    assert_eq!(
        plain
            .image_set
            .frame("walk1.png")
            .map(|f| (f.width, f.height)),
        Some((8, 6)),
        "scales に無い set は None（等倍）"
    );
}

/// 空 img_dir（set 0 件）→ Ok(空 Vec)。
#[test]
fn load_materials_with_empty_img_dir_returns_empty_vec() {
    let assets = TempAssets::new("empty");
    assets.write_valid_conf();

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
        .expect("空 img_dir はエラーではなく空素材");
    assert!(materials.is_empty(), "set 0 件 → 空 Vec");
}

/// conf 側の失敗は全て ConfigError として伝播する（素材は 1 つも返さない）:
/// actions.xml parse 失敗 / behaviors.xml parse 失敗 / 必須 4 種欠落。
#[test]
fn load_materials_propagates_config_parse_and_required_errors() {
    // actions.xml の parse 失敗（未知ルートタグ）
    {
        let assets = TempAssets::new("cfg_actions");
        assets.write_valid_conf();
        assets.write_conf("actions.xml", "<WrongRoot/>");
        assets.write_png("SetA", "walk1.png", 8, 8);
        let err = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
            .expect_err("actions.xml の parse 失敗は Err");
        assert!(
            matches!(err, MaterialError::Config(_)),
            "ConfigError が MaterialError::Config として伝播する"
        );
        assert!(!err.to_string().is_empty(), "Display は日本語メッセージ");
    }

    // behaviors.xml の parse 失敗
    {
        let assets = TempAssets::new("cfg_behaviors");
        assets.write_valid_conf();
        assets.write_conf("behaviors.xml", "<Other/>");
        assets.write_png("SetA", "walk1.png", 8, 8);
        let err = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
            .expect_err("behaviors.xml の parse 失敗は Err");
        assert!(matches!(err, MaterialError::Config(_)));
    }

    // validate_required_behaviors 失敗（必須 4 種欠落）
    {
        let assets = TempAssets::new("cfg_required");
        assets.write_valid_conf();
        assets.write_conf(
            "behaviors.xml",
            concat!(
                "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n",
                "  <BehaviorList>\n",
                "    <Behavior Name=\"Walk\" Frequency=\"1\"/>\n",
                "  </BehaviorList>\n",
                "</Mascot>\n"
            ),
        );
        assets.write_png("SetA", "walk1.png", 8, 8);
        let err = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
            .expect_err("必須 4 種欠落は Err");
        assert!(
            matches!(err, MaterialError::Config(_)),
            "validate_required_behaviors の失敗も Config として伝播する"
        );
        assert!(!err.to_string().is_empty(), "Display は日本語メッセージ");
    }
}

/// conf ファイルが見つからない set は warn + スキップ（素材全体は失敗しない・
/// Java failedConfigurations 相当）/ img_dir 不在 → Imageset（enumerate_sets の Err 伝播）。
#[test]
fn load_materials_skips_sets_without_config_and_propagates_missing_img_dir() {
    // conf ファイル不在（画像だけ置かれた set）
    {
        let assets = TempAssets::new("missing_conf");
        assets.write_png("Orphan", "walk1.png", 8, 8);
        let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
            .expect("conf 不在の set はスキップされ、素材全体は失敗しない");
        assert!(
            materials.is_empty(),
            "conf を持つ set が 1 つも無ければ空素材（従来の Err から #32 で変更）"
        );
    }

    // img_dir 不在（enumerate_sets の I/O エラー）
    {
        let assets = TempAssets::new("missing_img");
        assets.write_valid_conf();
        let missing = assets.root.join("no_such_img_dir");
        let err = load_materials(&assets.conf_dir(), &missing, &HashMap::new())
            .expect_err("img_dir 不在は Err");
        assert!(
            matches!(err, MaterialError::Imageset(_)),
            "ImagesetError が MaterialError::Imageset として伝播する"
        );
        assert!(!err.to_string().is_empty(), "Display は日本語メッセージ");
    }
}

// =====================================================================
// 契約 A2 (#32): per-set 設定ファイルの解決（Java Main の探索順）
// =====================================================================

/// ActionDef の先頭アニメ先頭 Pose の参照画像（set 別内容の観察点）。
fn first_pose_image(def: &ActionDef) -> &str {
    let animations = match def {
        ActionDef::Embedded { animations, .. }
        | ActionDef::Stay { animations, .. }
        | ActionDef::Move { animations, .. }
        | ActionDef::Animate { animations, .. }
        | ActionDef::Sequence { animations, .. }
        | ActionDef::Select { animations, .. } => animations,
    };
    animations[0].poses[0].image.as_str()
}

/// `conf/<set>/Actions.xml` は `conf/actions.xml` より優先され、**同名 Action でも
/// set ごとに別内容**になる（デレマスしめじ v1.9 の `Stand` が set ごとに別画像を
/// 参照する状況の最小再現）。専用ファイルを持たない set は共通 conf へフォールバック。
#[test]
fn load_materials_prefers_set_specific_config_over_shared() {
    let assets = TempAssets::new("per_set_conf");
    assets.write_conf("actions.xml", &actions_xml("/shared.png"));
    assets.write_conf("behaviors.xml", MINIMAL_BEHAVIORS_XML);
    assets.write_set_conf("SetA", "Actions.xml", &actions_xml("/a.png"));
    assets.write_set_conf("SetA", "Behavior.xml", MINIMAL_BEHAVIORS_XML);
    assets.write_png("SetA", "a.png", 8, 8);
    assets.write_png("SetB", "shared.png", 8, 8);

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
        .expect("set 専用 conf + 共通 conf でロードできる");
    assert_eq!(materials.len(), 2, "2 set とも material 化される");

    let set_a = materials
        .iter()
        .find(|m| m.name == "SetA")
        .expect("SetA が material 化される");
    let set_b = materials
        .iter()
        .find(|m| m.name == "SetB")
        .expect("SetB が material 化される");

    let walk_a = set_a.actions.actions.get("Walk").expect("SetA の Walk");
    let walk_b = set_b.actions.actions.get("Walk").expect("SetB の Walk");
    assert_eq!(
        first_pose_image(walk_a),
        "/a.png",
        "set 専用 conf が優先される"
    );
    assert_eq!(
        first_pose_image(walk_b),
        "/shared.png",
        "専用 conf を持たない set は共通 conf へフォールバックする"
    );
}

/// 探索順 1 番目 `img/<set>/conf/` が `conf/<set>/` より優先される
/// （Java `Main.getActionsFilePath` L392-396 逐語）。
#[test]
fn load_materials_prefers_img_side_conf_over_conf_dir() {
    let assets = TempAssets::new("img_conf");
    assets.write_conf("behaviors.xml", MINIMAL_BEHAVIORS_XML);
    assets.write_set_conf("SetA", "Actions.xml", &actions_xml("/conf_side.png"));
    assets.write_img_conf("SetA", "Actions.xml", &actions_xml("/img_side.png"));
    assets.write_png("SetA", "img_side.png", 8, 8);

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
        .expect("img 側 conf でロードできる");
    let set_a = materials
        .iter()
        .find(|m| m.name == "SetA")
        .expect("SetA が material 化される");
    let walk = set_a.actions.actions.get("Walk").expect("Walk");
    assert_eq!(
        first_pose_image(walk),
        "/img_side.png",
        "img/<set>/conf/ が最優先（探索順 1 番目）"
    );
}

/// behaviors も set ごとに解決され、当該 set の行動表になる（#32）。
#[test]
fn load_materials_uses_set_specific_behaviors() {
    let assets = TempAssets::new("per_set_behaviors");
    assets.write_conf("actions.xml", &actions_xml("/shared.png"));
    assets.write_conf("behaviors.xml", MINIMAL_BEHAVIORS_XML);
    assets.write_set_conf("SetA", "Behavior.xml", SET_BEHAVIORS_XML);
    assets.write_png("SetA", "shared.png", 8, 8);
    assets.write_png("SetB", "shared.png", 8, 8);

    let materials = load_materials(&assets.conf_dir(), &assets.img_dir(), &HashMap::new())
        .expect("set 専用 behaviors でロードできる");
    let rows_of = |name: &str| -> Vec<String> {
        materials
            .iter()
            .find(|m| m.name == name)
            .expect("material がある")
            .table
            .rows
            .iter()
            .map(|row| row.name.clone())
            .collect()
    };
    assert!(
        rows_of("SetA").iter().any(|row| row == "Sit"),
        "set 専用 behaviors の行が当該 set の table に入る: {:?}",
        rows_of("SetA")
    );
    assert!(
        !rows_of("SetB").iter().any(|row| row == "Sit"),
        "専用 behaviors を持たない set は共通 behaviors のまま: {:?}",
        rows_of("SetB")
    );
}

// =====================================================================
// 契約 B: Mascot::rebind_image_set / image_set
// =====================================================================

/// rebind は image_set_name と Arc<ImageSet> の 2 フィールド差し替えのみ。
/// behavior / anchor / look_right / time / paused / dragging / needs_repaint は
/// 一切不変。image_set() は差し替え後の Arc を deref して観測できる。
///
/// Java 不変式: Manager.tick は環境 tick を先に実行するため、実物 Environment を
/// mascot.tick 直叩きで使うテストは事前に 1 回 tick する（#8 確定の遅延更新・
/// screen_union は Environment::tick で初めて実値が流入する）。
#[test]
fn mascot_rebind_image_set_swaps_only_name_and_arc() {
    let env = single_monitor_env();
    // Java 不変式の代替（Manager.tick を経由しないため事前に 1 回だけ環境 tick）
    env.tick();
    let mut mascot = Mascot::new(
        "SetA",
        image_set_with("SetA", "pose.png", 32, 32),
        (100, 200),
    );
    mascot.set_look_right(true);

    // 実行中 behavior を付与（同名 table から構築・rng は不使用）
    let t = table(vec![row("Walk", 100)]);
    let mut factory = ScriptedFactory::new();
    let mut rng = unit_rng();
    let runner = t
        .build_behavior("Walk", &mut mascot, &env, &mut factory, rng.as_mut())
        .expect("Walk を構築できる");
    mascot
        .set_behavior(Some(runner), &env, &t, &mut factory, rng.as_mut())
        .expect("behavior を設定できる");
    mascot.tick(&env, &t, &mut factory, rng.as_mut());
    mascot.tick(&env, &t, &mut factory, rng.as_mut());

    // 差し替え前の状態を固める
    assert_eq!(mascot.behavior_name(), Some("Walk"));
    assert_eq!(mascot.time(), 2);
    assert_eq!(
        mascot
            .image_set()
            .frame("pose.png")
            .map(|f| (f.width, f.height)),
        Some((32, 32)),
        "差し替え前は旧 Arc（32x32）"
    );
    mascot.set_paused(true);
    mascot.set_dragging(true);
    mascot.clear_needs_repaint();

    // 契約: 2 フィールド差し替えのみ
    mascot.rebind_image_set("SetB", image_set_with("SetB", "pose.png", 16, 16));

    assert_eq!(mascot.image_set_name(), "SetB", "set 名が差し替わる");
    assert_eq!(
        mascot.image_set().name,
        "SetB",
        "image_set() は新 Arc を返す"
    );
    assert_eq!(
        mascot
            .image_set()
            .frame("pose.png")
            .map(|f| (f.width, f.height)),
        Some((16, 16)),
        "Arc 交換の観測: 新画像セットのフレーム寸法"
    );
    assert_eq!(
        mascot.behavior_name(),
        Some("Walk"),
        "behavior（runner）は一切不変"
    );
    assert_eq!(mascot.anchor(), (100, 200), "anchor 不変");
    assert!(mascot.look_right(), "look_right 不変");
    assert_eq!(mascot.time(), 2, "time 不変");
    assert!(mascot.is_paused(), "paused 不変");
    assert!(mascot.is_dragging(), "dragging 不変");
    assert!(
        !mascot.needs_repaint(),
        "needs_repaint 不変（再描画要求を出さない）"
    );
}

// =====================================================================
// 契約 C-1: Manager::reload（空 materials）
// =====================================================================

/// 空 materials: 全 mascot が dispose 扱い（remove_pending・次 tick の retain で除去）+
/// set_tables はクリア（stale エントリは base フォールバックに戻る）+
/// base table は変更しない。
#[test]
fn manager_reload_with_empty_materials_disposes_all_and_clears_set_tables() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("BaseWalk", 100)]),
        ScriptedFactory::new(),
        unit_rng(),
    );
    manager.set_behavior_table("GhostSet", table(vec![row("GhostWalk", 100)]));
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.add(mascot_of_set("SetA", (11, 500)));
    manager.add(mascot_of_set("SetB", (12, 500)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 3);

    // 事前状態: GhostSet は注入 table・SetA/SetB は base フォールバック
    assert_eq!(
        manager.behavior_menu_items("GhostSet").selectable,
        ["GhostWalk"],
        "注入 table が生きている"
    );
    assert_eq!(
        manager.behavior_menu_items("SetA").selectable,
        ["BaseWalk"],
        "未登録 set は base フォールバック"
    );

    manager.reload(vec![]);

    // dispose 扱いは即時（remove_pending）・削除反映は次 tick
    let mut pending = 0;
    manager.apply_all(|m| {
        if m.remove_pending() {
            pending += 1;
        }
    });
    assert_eq!(pending, 3, "全 mascot が dispose 扱いになる");
    assert_eq!(manager.count(), 3, "削除は次 tick の retain で反映される");

    // set_tables はクリア・base table は変更なし
    assert_eq!(
        manager.behavior_menu_items("GhostSet").selectable,
        ["BaseWalk"],
        "stale エントリは消滅し base フォールバックに戻る"
    );
    assert_eq!(
        manager.behavior_menu_items("SetA").selectable,
        ["BaseWalk"],
        "base table は変更しない"
    );
    assert_eq!(manager.behavior_menu_items("SetB").selectable, ["BaseWalk"]);

    manager.tick(Instant::now());
    assert!(manager.is_empty(), "次 tick で全 mascot が除去される");
}

// =====================================================================
// 契約 C-2: Manager::reload（非空・table 置換 + 同名再構築/再選択の rng 分岐）
// =====================================================================

/// 非空 materials: base table を materials[0].table で置換 + set_tables 全消し再登録
///（stale エントリ残留なし）/ 自 set 残存 mascot は自 set 維持 / 現行 behavior 名が
/// 新 table に存在 → 同名再構築（rng を消費しない）・存在しない → 再選択（rng 消費）/
/// anchor・look_right・paused・dragging 維持 / Arc は新画像セットへ差し替わる。
#[test]
fn manager_reload_replaces_tables_and_rebuilds_without_extra_rng() {
    let env = single_monitor_env();
    // rng pin: 値は 1 個だけ（0.9）。同名再構築が rng を消費すれば A が Stare になり、
    // B の再選択で rng 枯渇 panic → どちらも失敗で検出される。
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("OldBase", 100)]),
        ScriptedFactory::new(),
        fixed_rng(vec![0.9]),
    );
    manager.set_behavior_table("SetA", table(vec![row("Walk", 100)]));
    manager.set_behavior_table("SetB", table(vec![row("SitDown", 100)]));
    manager.set_behavior_table("StaleSet", table(vec![row("StaleWalk", 100)]));
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.add(mascot_of_set("SetB", (20, 600)));
    manager.tick(Instant::now());
    manager.apply_all(|m| {
        m.set_look_right(m.image_set_name() == "SetA");
    });
    manager.set_behavior_all_of_set("SetA", "Walk");
    manager.set_behavior_all_of_set("SetB", "SitDown");
    manager.apply_all(|m| {
        if m.image_set_name() == "SetA" {
            m.set_paused(true);
            m.set_dragging(true);
        }
    });

    // 事前状態の確認（table 置換の比較対象）
    assert_eq!(manager.behavior_menu_items("SetA").selectable, ["Walk"]);
    assert_eq!(manager.behavior_menu_items("SetB").selectable, ["SitDown"]);
    assert_eq!(
        manager.behavior_menu_items("StaleSet").selectable,
        ["StaleWalk"]
    );

    let mats = vec![
        material(
            "SetA",
            image_set_with("SetA", "pose.png", 16, 16),
            vec![row("Walk", 1), row("Stare", 1)],
        ),
        material(
            "SetB",
            image_set_with("SetB", "b.png", 16, 16),
            vec![row("AltWalk", 1)],
        ),
    ];
    manager.reload(mats);

    let snap = snapshot(&mut manager);
    let a = find_set(&snap, "SetA");
    assert_eq!(
        a.behavior.as_deref(),
        Some("Walk"),
        "現行名 Walk は新 table に存在 → 同名再構築（rng を消費しない）"
    );
    assert_eq!(a.anchor, (10, 500), "anchor 維持");
    assert!(a.look_right, "look_right 維持");
    assert!(a.paused, "paused 維持");
    assert!(a.dragging, "dragging 維持");
    assert!(!a.remove_pending, "自 set 残存 mascot は dispose されない");
    assert_eq!(
        a.frame_dims,
        Some((16, 16)),
        "Arc は新画像セット（16x16）へ差し替わる"
    );

    let b = find_set(&snap, "SetB");
    assert_eq!(
        b.behavior.as_deref(),
        Some("AltWalk"),
        "現行名 SitDown は新 table に無い → build_next_behavior(None) 再選択（0.9 → AltWalk）"
    );
    assert_eq!(b.anchor, (20, 600), "再選択でも anchor 維持");
    assert!(!b.look_right, "再選択でも look_right 維持");
    assert!(!b.remove_pending);
    assert_eq!(
        b.frame_dims,
        Some((16, 16)),
        "自 set 維持・Arc は新オブジェクト"
    );

    // table 置換と stale 掃除の観測（menu は未知 set = base 参照で観測可）
    assert_eq!(
        manager.behavior_menu_items("SetA").selectable,
        ["Walk", "Stare"],
        "materials[0] の table が base に入る（旧 base OldBase は消滅）"
    );
    assert_eq!(
        manager.behavior_menu_items("SetB").selectable,
        ["AltWalk"],
        "全 materials 分が set_tables に再登録される"
    );
    assert_eq!(
        manager.behavior_menu_items("StaleSet").selectable,
        ["Walk", "Stare"],
        "古い stale エントリは残らず base フォールバックに戻る"
    );
    assert_eq!(
        manager.behavior_menu_items("UnknownSet").selectable,
        ["Walk", "Stare"],
        "未知 set も新 base を参照する"
    );
}

// =====================================================================
// 契約 C-3: set 消滅時の既定 set フォールバック（rebind + behavior 復活）
// =====================================================================

/// 消滅 set の mascot は materials[0] の set 名 + image_set へ付け替わる。
/// behavior は「付け替え後 set の table」で判定される:
/// - 新 table に存在する名前（Walk）は同名再構築（rng 不消費・旧 set の table が
///   どうであれ新 table 基準）
/// - 新 table に存在しない名前（SitDown）は再選択（rng 消費 → 0.9 → Stare）。
///   もし旧 set の table（SitDown を含む）で判定していたら同名再構築 SitDown になるため、
///   判定表が「付け替え後」であることも pin できる
#[test]
fn manager_reload_falls_back_to_first_material_when_set_vanished() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("OldBase", 100)]),
        ScriptedFactory::new(),
        fixed_rng(vec![0.9]),
    );
    manager.set_behavior_table("SetA", table(vec![row("Walk", 100)]));
    manager.set_behavior_table("SetB", table(vec![row("Walk", 100)]));
    manager.set_behavior_table("SetC", table(vec![row("SitDown", 100)]));
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.add(mascot_of_set("SetB", (20, 600)));
    manager.add(mascot_of_set("SetC", (30, 700)));
    manager.tick(Instant::now());
    manager.set_behavior_all_of_set("SetA", "Walk");
    manager.set_behavior_all_of_set("SetB", "Walk");
    manager.set_behavior_all_of_set("SetC", "SitDown");

    // SetB / SetC を消滅させる（materials は SetA のみ）
    let mats = vec![material(
        "SetA",
        image_set_with("SetA", "pose.png", 16, 16),
        vec![row("Walk", 1), row("Stare", 1)],
    )];
    manager.reload(mats);

    assert_eq!(manager.count(), 3, "全 mascot が存続する");
    assert_eq!(
        manager.count_of("SetA"),
        3,
        "消滅 set の mascot は materials[0] の set へ付け替わる"
    );

    let snap = snapshot(&mut manager);
    for view in &snap {
        assert_eq!(
            view.image_set, "SetA",
            "全 mascot が materials[0] の set 名を持つ"
        );
        assert_eq!(
            view.frame_dims,
            Some((16, 16)),
            "image_set も materials[0] の Arc へ付け替わる"
        );
        assert!(!view.remove_pending, "存続 mascot は dispose されない");
    }
    assert_eq!(
        find_set(&snap, "SetA").behavior.as_deref(),
        Some("Walk"),
        "自 set 残存・同名再構築（rng 不消費）"
    );
    assert_eq!(find_set(&snap, "SetA").anchor, (10, 500), "anchor 維持");
    // SetB 由来: 現行名 Walk は新 SetA table に存在 → 同名再構築（rng 不消費）
    //（同一 index 順で 2 番目・SetC の再選択より先に処理される前提の rng pin）
    let reb = snap
        .iter()
        .find(|v| v.anchor == (20, 600))
        .expect("旧 SetB の mascot が残る");
    assert_eq!(
        reb.behavior.as_deref(),
        Some("Walk"),
        "付け替え後 set の table に同名があれば同名再構築で復活する"
    );
    assert_eq!(reb.anchor, (20, 600), "anchor 維持");
    // SetC 由来: 現行名 SitDown は新 SetA table に無い → 再選択（0.9 → Stare）
    let rec = snap
        .iter()
        .find(|v| v.anchor == (30, 700))
        .expect("旧 SetC の mascot が残る");
    assert_eq!(
        rec.behavior.as_deref(),
        Some("Stare"),
        "付け替え後 set の table に無ければ再選択（旧 table 基準なら SitDown のまま）"
    );
    assert_eq!(rec.anchor, (30, 700), "再選択でも anchor 維持");
}

// =====================================================================
// 契約 C-4: 構築 Err / set_behavior Err → 該当 mascot のみ dispose
// =====================================================================

/// 同名再構築の構築 Err（factory Err）と set_behavior Err（init Err）は
/// それぞれその mascot のみ dispose（log + 次 tick で除去）・他は無傷。
#[test]
fn manager_reload_disposes_only_mascots_whose_rebuild_fails() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new()
            .with_fail(&["BrokenWalk"])
            .with_init_err("ErrInitWalk"),
        unit_rng(),
    );
    manager.set_behavior_table("SetA", table(vec![row("Walk", 100)]));
    manager.set_behavior_table("SetB", table(vec![row("Walk", 100)]));
    manager.set_behavior_table("SetC", table(vec![row("Walk", 100)]));
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.add(mascot_of_set("SetB", (20, 600)));
    manager.add(mascot_of_set("SetC", (30, 700)));
    manager.tick(Instant::now());
    manager.set_behavior_all("Walk");
    assert_eq!(manager.count(), 3);

    let mats = vec![
        material(
            "SetA",
            image_set_with("SetA", "pose.png", 16, 16),
            vec![row("Walk", 1)],
        ),
        // 構築 Err: action 参照名 BrokenWalk が factory で失敗する
        material(
            "SetB",
            image_set_with("SetB", "b.png", 16, 16),
            vec![row_with_action("Walk", "BrokenWalk", 1)],
        ),
        // set_behavior Err: action の init が失敗する
        material(
            "SetC",
            image_set_with("SetC", "c.png", 16, 16),
            vec![row_with_action("Walk", "ErrInitWalk", 1)],
        ),
    ];
    manager.reload(mats);

    let snap = snapshot(&mut manager);
    let a = snap.iter().find(|v| v.image_set == "SetA").unwrap();
    assert_eq!(
        a.behavior.as_deref(),
        Some("Walk"),
        "成功した mascot は新 runner で再構築される"
    );
    assert!(!a.remove_pending, "他は無傷");

    let b = snap.iter().find(|v| v.image_set == "SetB").unwrap();
    assert!(b.remove_pending, "構築 Err → その mascot のみ dispose");
    let c = snap.iter().find(|v| v.image_set == "SetC").unwrap();
    assert!(
        c.remove_pending,
        "set_behavior Err → その mascot のみ dispose"
    );

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 1, "失敗 2 体は次 tick で除去される");
    assert_eq!(manager.count_of("SetA"), 1);
}

// =====================================================================
// 契約 C-5: enabled pre-check（同一式）+ Environment の disabled map 不変
// =====================================================================

/// 現行 behavior が toggleable で無効化されている場合、同名再構築は行われず
/// build_next_behavior(None) で再選択される（build_behavior の再配置分岐は不通 =
/// anchor 不変・rng は再選択の 1 回のみ）。reload 自体は Environment の
/// disabled map を変えない（menu の checked 状態で観測）。
#[test]
fn manager_reload_enabled_precheck_and_disabled_map_untouched() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new(),
        fixed_rng(vec![0.5]),
    );
    manager.set_behavior_table(
        "SetA",
        table(vec![row_entry("Pose", 100, false, true), row("Walk", 100)]),
    );
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.tick(Instant::now());
    manager.set_behavior_all_of_set("SetA", "Pose");
    // 有効なうちに Pose を設定した後、トグル OFF
    manager.set_behavior_enabled("SetA", "Pose", false);
    assert_eq!(
        manager.behavior_menu_items("SetA").toggleable,
        [("Pose".to_string(), false)],
        "reload 前: Pose は無効（checked = false）"
    );

    let mats = vec![material(
        "SetA",
        image_set_with("SetA", "pose.png", 16, 16),
        vec![row_entry("Pose", 1, false, true), row("Walk", 1)],
    )];
    manager.reload(mats);

    let snap = snapshot(&mut manager);
    let a = find_set(&snap, "SetA");
    assert_eq!(
        a.behavior.as_deref(),
        Some("Walk"),
        "無効な Pose は同名再構築されず再選択される（有効候補は Walk のみ → 0.5 → Walk）"
    );
    assert_eq!(
        a.anchor,
        (10, 500),
        "pre-check により再配置分岐は不通（anchor 不変）"
    );
    assert!(!a.remove_pending);

    assert_eq!(
        manager.behavior_menu_items("SetA").toggleable,
        [("Pose".to_string(), false)],
        "reload は Environment の disabled map を変えない"
    );
    assert_eq!(
        manager.behavior_menu_items("SetA").selectable,
        ["Walk"],
        "menu は新 table を参照する"
    );
}

// =====================================================================
// 契約 C-6: behavior 無し mascot（再選択経路・進行リセット）
// =====================================================================

/// behavior を持たない mascot も reload 対象: 現行名は存在しないため
/// build_next_behavior(None)（createMascot と同一経路）で再選択され、
/// 新 runner が与えられる（dispose されない）。
#[test]
fn manager_reload_gives_behavior_to_behaviorless_mascot() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new(),
        fixed_rng(vec![0.5]),
    );
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 1);

    let mats = vec![material(
        "SetA",
        image_set_with("SetA", "pose.png", 16, 16),
        vec![row("Walk", 1)],
    )];
    manager.reload(mats);

    let snap = snapshot(&mut manager);
    let a = find_set(&snap, "SetA");
    assert_eq!(
        a.behavior.as_deref(),
        Some("Walk"),
        "behavior 無し mascot も再選択（build_next_behavior(None)）で runner を得る"
    );
    assert_eq!(a.anchor, (10, 500));
    assert_eq!(a.frame_dims, Some((16, 16)));
    assert!(!a.remove_pending, "存続する（dispose されない）");
    assert_eq!(manager.count(), 1);
}

// =====================================================================
// スライス 3: set 別 tint（reload が登録 → spawn / 存続個体へ注入）
// =====================================================================

/// reload は set 宣言の tint を Manager に登録し、spawn する個体へその set の
/// tint を注入する。Off の set（宣言なし）は色なしのまま = 既存 set は影響を受けない。
#[test]
fn manager_reload_injects_per_set_tint_into_spawned_mascots() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new(),
        unit_rng(),
    );
    manager.set_image_set_resolver(|name| match name {
        "SetA" => Some(image_set_with("SetA", "a.png", 8, 8)),
        "SetB" => Some(image_set_with("SetB", "b.png", 8, 8)),
        _ => None,
    });

    let cycle = TintStyle {
        mode: TintMode::Cycle,
        rotate: 150.0,
        ..Default::default()
    };
    manager.reload(vec![
        material_with_tint(
            "SetA",
            image_set_with("SetA", "a.png", 8, 8),
            vec![row("Walk", 100)],
            cycle,
        ),
        material_with_tint(
            "SetB",
            image_set_with("SetB", "b.png", 8, 8),
            vec![row("Walk", 100)],
            TintStyle::default(),
        ),
    ]);

    manager.request_spawn("SetA");
    manager.request_spawn("SetB");
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2);

    // apply_all 経由（公開 API）で set ごとの色を観測する。位相の線形前進・wrap は
    // mascot_test が直接検証するため、ここでは「set 宣言が個体へ届くこと」を見る。
    let mut tints = Vec::new();
    manager.apply_all(|m| {
        tints.push((m.image_set_name().to_string(), m.tint_rgb(), m.tint_hue()));
    });
    let tint_of = |tints: &[(String, Option<[u8; 3]>, f32)], set: &str| -> (Option<[u8; 3]>, f32) {
        tints
            .iter()
            .find(|(name, _, _)| name.as_str() == set)
            .map(|(_, rgb, hue)| (*rgb, *hue))
            .unwrap_or_else(|| panic!("set {set} の mascot が居ない"))
    };

    let (a_rgb, a_hue) = tint_of(&tints, "SetA");
    assert!(a_rgb.is_some(), "宣言のある set は色づけされる");
    assert!(
        (a_hue - 6.0).abs() < 1e-3,
        "spawn と同一 tick で 150°/s × 0.04s = 6° 進む: {a_hue}"
    );

    let (b_rgb, _) = tint_of(&tints, "SetB");
    assert_eq!(b_rgb, None, "宣言の無い set は色なし = 従来とバイト同一");
}

/// reload は存続する個体の tint も「付け替え後 set」の宣言へ更新する
/// （色は ImageSet ではなく set 宣言に由来するため、旧宣言を残さない）。
#[test]
fn manager_reload_updates_persisting_mascot_tint() {
    let env = single_monitor_env();
    let mut manager = make_manager_with_rng(
        env,
        table(vec![row("Walk", 100)]),
        ScriptedFactory::new(),
        unit_rng(),
    );
    manager.set_behavior_table("SetA", table(vec![row("Walk", 100)]));
    manager.add(mascot_of_set("SetA", (10, 500)));
    manager.tick(Instant::now());

    let mut before = None;
    manager.apply_all(|m| before = Some(m.tint_rgb()));
    assert_eq!(before.flatten(), None, "既定は色なし");

    manager.reload(vec![material_with_tint(
        "SetA",
        image_set_with("SetA", "a.png", 8, 8),
        vec![row("Walk", 100)],
        TintStyle {
            mode: TintMode::Cycle,
            rotate: 150.0,
            ..Default::default()
        },
    )]);

    let mut after = None;
    manager.apply_all(|m| after = Some(m.tint_rgb()));
    assert!(
        after.flatten().is_some(),
        "reload 後の set 宣言の色が入る（旧 Off を残さない）"
    );
}
