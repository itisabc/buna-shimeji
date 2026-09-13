//! タスク #8 Step1: app モジュール（Environment / Manager / tick スケジューラ）の
//! 契約テスト（RED）。
//!
//! Java 正本（.tmp/java-ref/）を仕様として、公開契約の振る舞いのみを検証する
//! （tests/action_test.rs 書式踏襲・自己完結・tests/common 不使用）。
//! 期待値には Java 行番号をコメント焼き込み。実装（src/app/・lib.rs `pub mod app`）
//! 未存在のため cargo test は compile error = RED が正常。
//!
//! pin する API 契約（coder への指示・シグネチャは tests が固定する）:
//!
//! ```text
//! // ---- src/app/environment.rs ----
//! pub struct SpawnRequest {                        // spawn キューの 1 件
//!     pub image_set_name: String,
//!     pub anchor: (i32, i32),
//!     pub look_right: bool,
//!     pub behavior_name: Option<String>,           // #9b: None = buildNextBehavior(null) 経路 /
//!                                                  //       Some(name) = buildBehavior(name) 経路
//!                                                  //      （queue_spawn の第 4 引数は Some 化して格納）
//! }
//!
//! pub trait OsSource {                             // OS 供給の抽象（実 Win32 供給は #10）
//!     fn monitors(&self) -> Vec<(Rect, Rect)>;     // (モニタ矩形, WorkArea)・monitor index 順
//!     fn cursor_position(&self) -> Option<(i32, i32)>; // None = 取得失敗（MouseInfo null 相当）
//!     fn active_window(&self) -> Option<(i64, Rect)>;  // (id, 矩形)。None = アクティブ窓無し
//!     fn move_window(&self, id: i64, x: i32, y: i32);  // SetWindowPos 相当
//!     fn windows(&self) -> Vec<(i64, Rect)>;       // #9b: interactive 窓列挙（OUT_OF_BOUNDS 判定前）
//!     fn raise_window(&self, id: i64);             // #9b: BringWindowToTop 相当
//! }
//! // Rect 型 = `shimeji::mascot::Rect`。
//!
//! // Environment は EnvironmentView を全実装する（default todo!() を全置換）。
//! pub fn Environment::new(source: impl OsSource + 'static) -> Environment
//! pub fn Environment::tick(&self)
//!     // AbstractEnvironment.tick L167-183（screen/complexScreen/complexWorkArea 更新 + cursor）
//!     // + WindowsEnvironment.tick L66-85（activeWindow 更新）
//! pub fn Environment::drain_spawns(&mut self) -> Vec<SpawnRequest> // FIFO 全取出し+クリア
//! pub fn Environment::set_breeding_allowed(&mut self, bool)        // settings トグル setter 群
//! pub fn Environment::set_transients_enabled(&mut self, bool)
//! pub fn Environment::set_transformation_allowed(&mut self, bool)
//! pub fn Environment::set_throwing_allowed(&mut self, bool)
//! pub fn Environment::set_multiscreen(&mut self, bool)
//! pub fn Environment::set_scaling(&mut self, f64)
//! // 既定値 = Settings.java 既定（settings.properties 無しのため既定適用・L32-45）:
//! //   breeding/transients/transformation/throwing/multiscreen = true・scaling = 1.0
//!
//! // EnvironmentView 実装の合同契約:
//! // - work_area()（base-4・anchor 無し）= プライマリモニタ（仮想座標 (0,0) を含む
//! //   monitor の work area・該当なしなら先頭）＝ Java per-mascot キャッシュの意図的
//! //   差異（design §1.7(e) 延長）
//! // - screen()（base-4）= screen_area() の union の mascot::Rect 表示
//! // - work_area_at(x, y) は決定的: contains（境界含み）する最初の monitor の slot・
//! //   該当無し = AreaSlot::Invisible
//! // - floor/wall/ceiling は env.rs 純関数へ委譲（重複実装禁止）
//! // - active window: id 変化 → delta リセット（0）/ 同一 id → delta = 新旧差分 /
//! //   rect 無し → setRect(-1,-1,0,0) 相当・visible = screen と intersects
//! // - cursor: source から取得し delta(dx,dy) 更新（平均化式・Location.java L159-160）
//!
//! // ---- src/app/manager.rs ----
//! pub const Manager::TICK_INTERVAL_MS: u64 = 40;                   // Manager.java L37
//! pub fn Manager::tick_due(elapsed: Duration) -> bool              // elapsed >= 40ms
//! pub fn Manager::next_delay(elapsed: Duration) -> Duration
//!     // elapsed < 40ms → 残り時間。elapsed >= 40ms → 常に 40ms
//!     //（間隔 2 回分以上の elapsed でも実行は 1 tick 分のみ進め次回 40ms 後 =
//!     //  スリープ復帰バースト防止。Java Ticker L158-165 の「最大 2tick 補填」とは
//!     //  AGENTS §5-7 承認済みの意図的差異）
//! pub fn Manager::new(env: Environment, table: BehaviorTable,
//!     factory: Box<dyn BehaviorFactory>, rng: Box<dyn Rng>) -> Manager
//! pub fn Manager::set_image_set_resolver(&mut self,
//!     resolver: impl FnMut(&str) -> Option<Arc<ImageSet>> + 'static)
//! pub fn Manager::add(&mut self, mascot: Mascot)                   // added キュー・次 tick 反映
//! pub fn Manager::tick(&mut self, now: Instant)                    // Java L201-244:
//!     // ① env.tick() → ② spawn キュー drain（新規 Mascot 生成・追加）
//!     // → ③ remove_pending 適用（削除）→ ④ 全員 mascot.tick()
//! pub fn Manager::apply_all(&mut self, apply: impl FnMut(&mut Mascot))
//!     // ⑤（Java Mascot.apply L662-708 相当の glue）。#10 が draw+needs_repaint クリアを
//!     // 実施できる形。Manager 自体は描画しない
//! pub fn Manager::environment_view(&self) -> &dyn EnvironmentView  // spawn キュー push 等の観測点
//! pub fn Manager::count(&self) -> usize                            // L522-549
//! pub fn Manager::count_of(&self, image_set_name: &str) -> usize
//! pub fn Manager::is_empty(&self) -> bool
//! pub fn Manager::remain_one(&mut self)                            // L346-356: 先頭 1 体残し・
//!     // 末尾から逆順 dispose
//! pub fn Manager::remain_one_of_set(&mut self, image_set_name: &str) // L385-401: 同 set のうち
//!     // リスト末尾側（最後に追加された）1 体を残し、それより前の同 set mascot を dispose
//! pub fn Manager::remain_none_of_set(&mut self, image_set_name: &str) // L429-442
//! pub fn Manager::dispose_all(&mut self)                           // L447-456: 末尾から逆順 dispose
//! pub fn Manager::set_behavior_all(&mut self, name: &str)          // L291-340:
//!     // 構築失敗 → log + dispose（L301-306 逐語）
//! pub fn Manager::set_exit_on_last_removed(&mut self, bool)        // L133-135・既定 true
//! pub fn Manager::should_exit(&self) -> bool                       // 全員消滅 tick 後に true。
//!     // process::exit はしない（#10 が消費）
//! pub fn Manager::is_paused(&self) -> bool                         // L465-475: 空=false・allMatch
//! pub fn Manager::toggle_pause_all(&mut self)                      // L480-495: 空=何もしない
//! pub fn Manager::set_enabled(&mut self, bool) / is_enabled(&self) -> bool // L503-515
//! pub fn Manager::get_mascot_with_affordance(&mut self, affordance: &str)
//!     -> Option<&mut Mascot>                                       // L558-573（WeakRef 相当）
//! pub fn Manager::has_overlapping_mascots_at(&self, anchor: (i32, i32)) -> bool // L581-601
//!
//! // ---- Mascot 追加（mod.rs 機械的波及・(T) 合流契約）----
//! pub fn Mascot::clear_needs_repaint(&mut self)  // Java needsRepaint=false 相当（apply L693-696）
//! ```

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use shimeji::app::environment::{Environment, OsSource};
use shimeji::app::manager::Manager;
use shimeji::config::script::Variable;
use shimeji::config::{
    ActionDef, ActionsConfig, Animation, BehaviorDef, BehaviorEntry, BehaviorsConfig, Pose,
    SequenceChild, VarMap,
};
use shimeji::mascot::action::build_action;
use shimeji::mascot::behavior::{Action, BehaviorError, BehaviorFactory, BehaviorTable};
use shimeji::mascot::env::{resolve_border, AreaSlot, AreaState, BorderKind, BorderRef, Edge};
use shimeji::mascot::{EnvironmentView, ImageState, Mascot, Rng};
use shimeji::render::imageset::ImageSet;

// =====================================================================
// 合成データヘルパ（自己完結）
// =====================================================================

/// 矩形ヘルパ。
fn rect(left: i32, top: i32, right: i32, bottom: i32) -> shimeji::mascot::Rect {
    shimeji::mascot::Rect {
        left,
        top,
        right,
        bottom,
    }
}

/// 矩形だけ一致させるヘルパ（deltas・visible は個別 assert）。
fn assert_area_rect(got: &AreaState, left: i32, top: i32, right: i32, bottom: i32) {
    assert_eq!(got.left, left);
    assert_eq!(got.top, top);
    assert_eq!(got.right, right);
    assert_eq!(got.bottom, bottom);
}

fn empty_image_set(name: &str) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

// =====================================================================
// Environment — fake source
// =====================================================================

/// OS 供給の状態（テストから RefCell 経由で差し替える）。
#[derive(Default)]
struct FakeState {
    monitors: Vec<(shimeji::mascot::Rect, shimeji::mascot::Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, shimeji::mascot::Rect)>,
    moved: Vec<(i64, i32, i32)>,
}

struct FakeSource {
    state: Rc<RefCell<FakeState>>,
}

impl OsSource for FakeSource {
    fn monitors(&self) -> Vec<(shimeji::mascot::Rect, shimeji::mascot::Rect)> {
        self.state.borrow().monitors.clone()
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        self.state.borrow().cursor
    }

    fn active_window(&self) -> Option<(i64, shimeji::mascot::Rect)> {
        self.state.borrow().active_window
    }

    fn move_window(&self, id: i64, x: i32, y: i32) {
        self.state.borrow_mut().moved.push((id, x, y));
    }

    fn windows(&self) -> Vec<(i64, shimeji::mascot::Rect)> {
        Vec::new()
    }

    fn raise_window(&self, _id: i64) {}
}

type EnvHandle = Rc<RefCell<FakeState>>;

/// モニタ構成つき Environment（handle も返す・tick 間に状態差し替え可能）。
fn env_with_monitors(
    monitors: Vec<(shimeji::mascot::Rect, shimeji::mascot::Rect)>,
) -> (Environment, EnvHandle) {
    let state = Rc::new(RefCell::new(FakeState {
        monitors,
        cursor: None,
        active_window: None,
        moved: Vec::new(),
    }));
    (
        Environment::new(FakeSource {
            state: state.clone(),
        }),
        state,
    )
}

/// 単一モニタ構成: monitor (0,0,1920,1080) / work area (0,0,1920,1040)。
fn single_monitor_env() -> (Environment, EnvHandle) {
    env_with_monitors(vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))])
}

/// 2 モニタ構成（2 枚目は右側・同 taskbar 揃いの work area）。
fn two_monitor_env() -> (Environment, EnvHandle) {
    env_with_monitors(vec![
        (rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040)),
        (rect(1920, 0, 2560, 1080), rect(1920, 0, 2560, 1040)),
    ])
}

/// 名前から Arc<ImageSet> を引く resolver（固定 map・テスト用）。
fn single_set_resolver(
    name: &str,
    set: Arc<ImageSet>,
) -> impl FnMut(&str) -> Option<Arc<ImageSet>> {
    let mut map = HashMap::new();
    map.insert(name.to_string(), set);
    move |requested: &str| map.get(requested).cloned()
}

/// identity タグ付きの新規 Mascot（fresh・behavior 無し・タグは apply で上書きされるまで保持）。
fn tagged_mascot(tag: &str, anchor: (i32, i32)) -> Mascot {
    let mut m = Mascot::new("TestSet", empty_image_set("TestSet"), anchor);
    m.set_affordances(vec![tag.to_string()]);
    m
}

/// identity タグ付き + 指定 image set 名の新規 Mascot（remain 系の set 識別観測用）。
fn tagged_mascot_set(tag: &str, image_set_name: &str, anchor: (i32, i32)) -> Mascot {
    let mut m = Mascot::new(image_set_name, empty_image_set(image_set_name), anchor);
    m.set_affordances(vec![tag.to_string()]);
    m
}

// =====================================================================
// Manager fixture（実 Action 実装 + 合成 config）
// =====================================================================

/// box 化した [0,1) 乱数（過剰消費 panic）。
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

/// config 駆動ファクトリ（action::build_action をそのまま差し込む）。
struct ConfigFactory {
    actions: ActionsConfig,
}

impl BehaviorFactory for ConfigFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { name, .. } => {
                build_action(&self.actions, name, &VarMap::new(), 1.0)
            }
            SequenceChild::Inline(_) => Err(BehaviorError::UnknownBehavior(
                "(inline は本テストで未使用)".to_string(),
            )),
        }
    }
}

fn pose(image: &str, anchor: (i32, i32), velocity: (i32, i32), duration: i32) -> Pose {
    Pose {
        image: image.to_string(),
        anchor,
        velocity,
        duration,
    }
}

fn anim(condition: Option<Variable>, is_turn: bool, poses: Vec<Pose>) -> Animation {
    Animation {
        condition,
        poses,
        is_turn,
    }
}

fn attrs(pairs: &[(&str, &str)]) -> VarMap {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), Variable::parse(v)))
        .collect()
}

/// 合成 actions.xml 相当:
/// - "Walk"  : velocity (1,0)・Affordance "mark"（同 tick 適用観測用タグ）
/// - "Stare" : velocity (3,0)・Affordance "stare"
///
/// （空 image set のため apply_pose は anchor 移動のみ観測可能）
fn fixture_actions() -> ActionsConfig {
    let mut map = BTreeMap::new();
    map.insert(
        "Walk".to_string(),
        ActionDef::Animate {
            border: None,
            attrs: attrs(&[("Affordance", "mark")]),
            animations: vec![anim(None, false, vec![pose("p.png", (64, 64), (1, 0), 30)])],
        },
    );
    map.insert(
        "Stare".to_string(),
        ActionDef::Animate {
            border: None,
            attrs: attrs(&[("Affordance", "stare")]),
            animations: vec![anim(None, false, vec![pose("s.png", (64, 64), (3, 0), 30)])],
        },
    );
    ActionsConfig { actions: map }
}

/// "Walk" / "Stare" の BehaviorTable（frequency 100・action は同名 Ref）。
fn fixture_table() -> BehaviorTable {
    let def = |name: &str| BehaviorDef {
        name: name.to_string(),
        frequency: 100,
        hidden: false,
        toggleable: false,
        action: SequenceChild::Ref {
            name: name.to_string(),
            attrs: VarMap::new(),
        },
        next: None,
    };
    BehaviorTable::new(&BehaviorsConfig {
        entries: vec![
            BehaviorEntry::Single(def("Walk")),
            BehaviorEntry::Single(def("Stare")),
        ],
    })
}

/// exit-on-last-removed を無効化した基本 Manager
///（exit 契約自体を試すテストのみ set_exit_on_last_removedでも true に戻せる）。
fn make_manager(env: Environment) -> Manager {
    let mut manager = Manager::new(
        env,
        fixture_table(),
        Box::new(ConfigFactory {
            actions: fixture_actions(),
        }),
        Box::new(BoxedRng {
            values: vec![0.5; 128],
            consumed: 0,
        }),
    );
    manager.set_exit_on_last_removed(false);
    manager.set_image_set_resolver(single_set_resolver("TestSet", empty_image_set("TestSet")));
    manager
}

/// env 側 spawn キューへ積むテストヘルパ（`environment_view` 観測点経由）。
fn spawn_into(
    m: &mut Manager,
    image_set_name: &str,
    anchor: (i32, i32),
    look_right: bool,
    behavior_name: &str,
) {
    m.environment_view()
        .queue_spawn(image_set_name, anchor, look_right, behavior_name);
}

// =====================================================================
// Environment — screen union + slot 契約
// =====================================================================

/// 2 モニタの screen union・monitor index 順 slot 1:1（(AA) 契約）+
/// work area 変化（タスクバー変化相当）の Area.set デルタ（Area.java L430-442 逐語）。
#[test]
fn env_screen_union_slot_order_and_work_area_deltas() {
    let (env, state) = two_monitor_env();

    // tick1: ゼロ基準からの初期化（assert は座標のみ）
    env.tick();
    let screen = env.screen_area();
    assert_area_rect(&screen, 0, 0, 2560, 1080);
    let screens = env.screens();
    assert_eq!(screens.len(), 2, "screens は monitor index 順の slot 1:1");
    assert_area_rect(&screens[0], 0, 0, 1920, 1080);
    assert_area_rect(&screens[1], 1920, 0, 2560, 1080);
    assert_area_rect(
        &env.work_area_state(AreaSlot::Screen(1)),
        1920,
        0,
        2560,
        1080,
    );
    assert_area_rect(
        &env.work_area_state(AreaSlot::WorkArea(0)),
        0,
        0,
        1920,
        1040,
    );

    // tick2: プライマリ work area 底辺 1040→1042（タスクバー変化相当）→ dbottom +2
    state.borrow_mut().monitors = vec![
        (rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1042)),
        (rect(1920, 0, 2560, 1080), rect(1920, 0, 2560, 1040)),
    ];
    env.tick();
    assert_area_rect(
        &env.work_area_state(AreaSlot::WorkArea(0)),
        0,
        0,
        1920,
        1042,
    );
    let work_area = env.work_area_state(AreaSlot::WorkArea(0));
    assert_eq!(work_area.dbottom, 2, "Area.set 差分（Java L430-442 逐語）");
    assert_eq!(work_area.dright, 0);
    assert!(work_area.visible, "work area は可視");
}

/// work_area_at（AbstractEnvironment.java L186-193 相当）:
/// 点を含む**最初の** monitor の work area slot・該当無し = Invisible。
#[test]
fn env_work_area_at_is_deterministic_invisible_fallback() {
    let (env, _) = two_monitor_env();
    env.tick();

    assert_eq!(env.work_area_at(100, 100), AreaSlot::WorkArea(0));
    assert_eq!(env.work_area_at(2000, 500), AreaSlot::WorkArea(1));
    // 境界点 x=1920 は 2 つの work area どちらにも含まれる（Area.contains 境界含み）が
    // monitor index 順で決定的に slot 0（settled: contains する最初の monitor）
    assert_eq!(env.work_area_at(1920, 500), AreaSlot::WorkArea(0));
    assert_eq!(env.work_area_at(5000, 0), AreaSlot::Invisible);

    // invisibleScreen 相当（全 0・visible=false・AbstractEnvironment L93-98）
    let invisible = env.work_area_state(AreaSlot::Invisible);
    assert_area_rect(&invisible, 0, 0, 0, 0);
    assert!(!invisible.visible);
}

/// work_area()（base-4・anchor 無し）= プライマリモニタ（仮想座標 (0,0) を含む monitor）の
/// work area＝Java per-mascot キャッシュの意図的差異（design §1.7(e) 延長）+
/// screen()（base-4）= 全モニタ union。
#[test]
fn env_work_area_base_method_targets_primary_monitor() {
    let (env, _) = two_monitor_env();
    env.tick();

    assert_eq!(
        env.work_area(),
        rect(0, 0, 1920, 1040),
        "プライマリ（index 0）の work area"
    );
    assert_eq!(
        env.screen(),
        rect(0, 0, 2560, 1080),
        "base-4 screen = 全モニタ union"
    );
}

// =====================================================================
// Environment — active window / cursor / settings / spawn
// =====================================================================

/// ActiveWindow 同一 id → delta = 新旧差分・visible = screen と intersects
///（WindowsEnvironment.java L66-85 逐語）。
#[test]
fn env_active_window_same_id_tracks_delta_and_visibility() {
    let (env, state) = single_monitor_env();
    state.borrow_mut().active_window = Some((7, rect(300, 200, 900, 800)));

    // tick1: 画面内 → visible = true
    env.tick();
    assert_area_rect(&env.active_window(), 300, 200, 900, 800);
    assert!(env.active_window().visible);

    // tick2: 同一 id・矩形移動（320,200,900,800）→ dleft = 20
    state.borrow_mut().active_window = Some((7, rect(320, 200, 900, 800)));
    env.tick();
    let win = env.active_window();
    assert_area_rect(&win, 320, 200, 900, 800);
    assert_eq!(win.dleft, 20, "同一 id → delta = 新旧差分（L75）");
    assert_eq!(win.dtop, 0);
    assert_eq!(win.dright, 0);
    assert_eq!(win.dbottom, 0);

    // tick3: 同一 id・画面外 → visible false（L77 intersects gate）
    state.borrow_mut().active_window = Some((7, rect(5000, 200, 5600, 800)));
    env.tick();
    assert!(!env.active_window().visible);
}

/// ActiveWindow id 変化 → delta リセット（WindowsEnvironment.java L79-82 逐語）。
#[test]
fn env_active_window_deltas_reset_on_id_change() {
    let (env, state) = single_monitor_env();
    state.borrow_mut().active_window = Some((7, rect(300, 200, 900, 800)));
    env.tick();
    assert_area_rect(&env.active_window(), 300, 200, 900, 800);

    state.borrow_mut().active_window = Some((9, rect(400, 200, 900, 800)));
    env.tick();
    let win = env.active_window();
    assert_area_rect(&win, 400, 200, 900, 800);
    assert_eq!(win.dleft, 0, "id 変化 → resetDeltas（delta 0）");
    assert_eq!(win.dtop, 0);
    assert_eq!(win.dright, 0);
    assert_eq!(win.dbottom, 0);
}

/// ActiveWindow rect 無し → setRect(-1,-1,0,0) 相当（Area は全辺 -1・幅 0）+
/// 画面と交差しない → visible false（WindowsEnvironment.java L72-76 逐語）。
#[test]
fn env_active_window_none_uses_negative_rect_and_invisible() {
    let (env, _) = single_monitor_env();
    env.tick();
    let win = env.active_window();
    assert_eq!(win.left, -1);
    assert_eq!(win.top, -1);
    assert_eq!(win.right, -1);
    assert_eq!(win.bottom, -1);
    assert!(!win.visible, "幅 0 → intersects false");
}

/// カーソル delta の平均化式（Location.java L159-160 逐語・env.rs doc 平均化は app 側）:
/// dx = (dx + 新旧差) / 2（Java int 除算 0 向け切り捨て）。
#[test]
fn env_cursor_delta_uses_averaging_formula() {
    let (env, state) = single_monitor_env();
    state.borrow_mut().cursor = Some((100, 100));

    // tick1: ゼロ基準からの移動 → dx = (0 + 100) / 2 = 50
    env.tick();
    let c = env.cursor();
    assert_eq!((c.x, c.y), (100, 100));
    assert_eq!(c.dx, 50);
    assert_eq!(c.dy, 50);

    // tick2: 移動 (110,105) → dx = (50 + 10)/2 = 30・dy = (50 + 5)/2 = 27
    state.borrow_mut().cursor = Some((110, 105));
    env.tick();
    let c = env.cursor();
    assert_eq!(c.dx, 30);
    assert_eq!(c.dy, 27);
}

/// カーソル取得失敗（None・MouseInfo null 相当）→ (0,0) 設置
///（AbstractEnvironment.java L180-181 逐語）。
#[test]
fn env_cursor_none_falls_back_to_origin() {
    let (env, _) = single_monitor_env();
    env.tick();
    let c = env.cursor();
    assert_eq!(c.x, 0);
    assert_eq!(c.y, 0);
}

/// settings 透過: 既定値（Settings.java L32-45・settings.properties 無しのため既定適用）+
/// トグル setter 群が EnvironmentView 経由で反映される。
#[test]
fn env_settings_defaults_and_setters() {
    let (mut env, _) = single_monitor_env();

    assert!(env.breeding_allowed(), "既定 true");
    assert!(env.transients_enabled());
    assert!(env.transformation_allowed());
    assert!(env.throwing_allowed());
    assert!(env.multiscreen());
    assert_eq!(env.scaling(), 1.0);

    env.set_breeding_allowed(false);
    env.set_transients_enabled(false);
    env.set_transformation_allowed(false);
    env.set_throwing_allowed(false);
    env.set_multiscreen(false);
    env.set_scaling(2.0);
    assert!(!env.breeding_allowed());
    assert!(!env.transients_enabled());
    assert!(!env.transformation_allowed());
    assert!(!env.throwing_allowed());
    assert!(!env.multiscreen());
    assert_eq!(env.scaling(), 2.0);
}

/// spawn キュー: FIFO・&self から push 可（単一スレッド前提・Mutex 増加禁止）+
/// drain で FIFO 全取出し+クリア。
#[test]
fn env_spawn_queue_is_fifo_and_drain_clears() {
    let (mut env, _) = single_monitor_env();

    env.queue_spawn("TestSet", (100, 200), false, "Walk");
    env.queue_spawn("TestSet", (300, 400), true, "Stare");

    let drained = env.drain_spawns();
    assert_eq!(drained.len(), 2, "FIFO 順保持");
    assert_eq!(drained[0].image_set_name, "TestSet");
    assert_eq!(drained[0].anchor, (100, 200));
    assert!(!drained[0].look_right);
    assert_eq!(
        drained[0].behavior_name,
        Some("Walk".to_string()),
        "#9b: queue_spawn の第 4 引数は Some 化して格納される"
    );
    assert_eq!(drained[1].anchor, (300, 400));
    assert!(drained[1].look_right);
    assert_eq!(drained[1].behavior_name, Some("Stare".to_string()));

    // drain 後は空（クリアされる）
    assert!(env.drain_spawns().is_empty());
}

/// getFloor / getWall / getCeiling は env.rs 純関数へ委譲
///（重複実装禁止・代表 1 件ずつ）。
#[test]
fn env_borders_delegate_to_env_pure_functions() {
    let (env, _) = single_monitor_env();

    env.tick();

    // 床: work area 底辺上 → WorkArea(0) の Bottom 辺（getFloor L212-230）
    let floor_on = resolve_border(&env, BorderKind::Floor, (500, 1040), false, false);
    assert_eq!(
        floor_on,
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Bottom,
        })
    );
    let floor_off = resolve_border(&env, BorderKind::Floor, (500, 500), false, false);
    assert_eq!(floor_off, None, "床外 → NotOnBorder");

    // 壁: lookRight=false → work area 左辺（getWall L253-272 の逆向き契約）
    let wall = resolve_border(&env, BorderKind::Wall, (0, 500), false, false);
    assert_eq!(
        wall,
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Left,
        })
    );

    // 天井: work area 上辺（getCeiling L169-187）
    let ceiling = resolve_border(&env, BorderKind::Ceiling, (500, 0), false, false);
    assert_eq!(
        ceiling,
        Some(BorderRef {
            area: AreaSlot::WorkArea(0),
            edge: Edge::Top,
        })
    );
}

/// moveActiveWindow: tick で snapshot した id で source を呼び・窓無しでは呼ばない
///（WindowsEnvironment.java L274-289 相当）。
#[test]
fn env_move_active_window_uses_tracked_id_and_none_is_noop() {
    let (env, state) = single_monitor_env();
    state.borrow_mut().active_window = Some((7, rect(300, 200, 900, 800)));
    env.tick();
    env.move_active_window(10, 20);
    assert_eq!(*state.borrow().moved, [(7, 10, 20)]);

    let (env, state) = single_monitor_env();
    state.borrow_mut().active_window = None;
    env.tick();
    env.move_active_window(1, 1);
    assert!(state.borrow().moved.is_empty(), "窓無し → 移動呼び出しなし");
}

// =====================================================================
// Manager — tick 順序 / spawn / 削除 / 集計 API
// =====================================================================

/// tick 順序（Manager.java L201-244）:
/// ① env.tick() → ② spawn drain → ③ remove_pending 適用 → ④ 全員 mascot.tick()。
/// spawn が remove より先に反映され、新規 mascot は**同 tick で動き出す**。
#[test]
fn manager_tick_spawns_before_removes_and_spawn_moves_same_tick() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);

    // 既存 1 体を dispose（remove_pending）して同 tick に spawn 要求を積む
    manager.add(tagged_mascot("old", (0, 500)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 1);

    manager.get_mascot_with_affordance("old").unwrap().dispose();
    spawn_into(&mut manager, "TestSet", (500, 500), false, "Walk");
    manager.tick(Instant::now());

    assert_eq!(manager.count(), 1, "削除 1 体 + 追加 1 体 → 差し引き同数");
    assert!(
        manager.get_mascot_with_affordance("old").is_none(),
        "spawn drain の後、削除も同 tick に反映される"
    );
    let child = manager
        .get_mascot_with_affordance("mark")
        .expect("spawn された新規 mascot が同 tick で動き出す（mark タグ付与済み）");
    assert_eq!(child.behavior_name(), Some("Walk"));
    assert_eq!(
        child.time(),
        1,
        "新規 mascot が生まれた tick 内で 1 tick 進む"
    );
    assert_eq!(
        child.anchor(),
        (501, 500),
        "velocity (1,0) が同 tick で適用"
    );
}

/// spawn の第 4 引数 = behavior 名: queue_spawn(..., "Stare") で生まれる Mascot は
/// Stare を着た状態で始まる。
#[test]
fn manager_spawn_starts_with_queued_behavior() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    spawn_into(&mut manager, "TestSet", (700, 500), false, "Stare");
    manager.tick(Instant::now());
    let child = manager
        .get_mascot_with_affordance("stare")
        .expect("spawn された mascot");
    assert_eq!(child.behavior_name(), Some("Stare"));
}

/// spawn スキップ ①: image set 不在 → log+スキップ（該当 spawn は追加されない）。
#[test]
fn manager_spawn_skips_unknown_image_set() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    spawn_into(&mut manager, "MissingSet", (500, 500), false, "Walk");
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "set 不在 → 該当 spawn は捨てられる");
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "キューは drain で空・再処理されない");
}

/// spawn スキップ ②: behavior 構築失敗（未知名）→ log+スキップ（子は追加されない・
/// Breed.java L73-101 + Manager 側 dispose 相当経路）。
#[test]
fn manager_spawn_skips_unknown_behavior() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    spawn_into(&mut manager, "TestSet", (500, 500), false, "Nope");
    manager.tick(Instant::now());
    assert!(manager.is_empty(), "未知名 behavior → 子は追加されない");
}

/// add はキューイングされ次 tick で反映（§1.5: Java added LinkedHashSet 踏襲）+
/// count / count_of / is_empty。
#[test]
fn manager_add_queues_until_tick_and_count_apis_work() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);

    manager.add(tagged_mascot("a1", (100, 500)));
    manager.add(tagged_mascot("a2", (200, 500)));
    manager.add(tagged_mascot("b1", (300, 500)));

    assert_eq!(manager.count(), 0, "キュー内は未反映・次 tick で一括追加");
    assert!(manager.is_empty());

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 3);
    assert!(!manager.is_empty());
    assert_eq!(manager.count_of("TestSet"), 3, "全員同 set");
    assert_eq!(manager.count_of("Missing"), 0, "不在 set は 0");
}

/// remain_one: リスト先頭 1 体を残し、**末尾から逆順 dispose**（Java L346-356）。
///（逆順 dispose そのものは公開 API から観測不能・最終状態の先頭 pin +
///  コメント「i を size-1 から 1 まで逆順 dispose」が coder 実装指示）
#[test]
fn manager_remain_one_keeps_first_mascot() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.add(tagged_mascot("a2", (200, 500)));
    manager.add(tagged_mascot("a3", (300, 500)));
    manager.tick(Instant::now());

    manager.remain_one();
    manager.tick(Instant::now());

    assert_eq!(manager.count(), 1);
    assert!(
        manager.get_mascot_with_affordance("a1").is_some(),
        "先頭（a1）が残る"
    );
    assert!(
        manager.get_mascot_with_affordance("a2").is_none(),
        "末尾側から dispose"
    );
    assert!(manager.get_mascot_with_affordance("a3").is_none());
}

/// remain_one_of_set: 同 set のうち**リスト末尾側（最後に追加された）1 体**を残す。
/// Java 正本 = Manager.java 本体 L385-401: 逆順走査（`i = totalMascots-1 → 0`）+
/// `isFirst` フラグで、末尾から最初に見つけた同 set mascot を残し、それより前
/// （リスト先頭側）の同 set mascot を dispose する。
/// Javadoc L379-381 の「the first mascot」は本体と食い違う上流 quirk のため、
/// AGENTS §5-1（Java ソース逐語）により**本体準拠**。
#[test]
fn manager_remain_one_of_set_keeps_last_of_set() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    // fixture の追加順 = Manager 内部リスト順（Java added LinkedHashSet 踏襲）:
    //   index 0: a1(TestA) / index 1: b1(TestB) / index 2: b2(TestB) / index 3: a2(TestA)
    //   → remain_one_of_set("TestB") の生存者は b2（末尾側）・dispose は b1（先頭側）
    manager.add(tagged_mascot_set("a1", "TestA", (100, 500)));
    manager.add(tagged_mascot_set("b1", "TestB", (200, 500)));
    manager.add(tagged_mascot_set("b2", "TestB", (300, 500)));
    manager.add(tagged_mascot_set("a2", "TestA", (400, 500)));
    manager.tick(Instant::now());

    manager.remain_one_of_set("TestB");
    manager.tick(Instant::now());

    assert_eq!(manager.count(), 3, "TestB は 2→1 体・A 系は無関係");
    assert!(
        manager.get_mascot_with_affordance("b2").is_some(),
        "同 set のうち末尾側（b2・リスト後方 = 最後に追加）が残る（L390-397 逆順走査 + isFirst）"
    );
    assert!(
        manager.get_mascot_with_affordance("b1").is_none(),
        "リスト先頭側の同 set（b1）は dispose（L394-396）"
    );
    assert!(manager.get_mascot_with_affordance("a1").is_some());
    assert!(manager.get_mascot_with_affordance("a2").is_some());
}

/// remain_none_of_set: 該当 set を全 dispose（Java L429-442）。
#[test]
fn manager_remain_none_of_set_disposes_all_of_set() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot_set("a1", "TestA", (100, 500)));
    manager.add(tagged_mascot_set("b1", "TestB", (200, 500)));
    manager.add(tagged_mascot_set("a2", "TestA", (300, 500)));
    manager.tick(Instant::now());

    manager.remain_none_of_set("TestA");
    manager.tick(Instant::now());

    assert_eq!(manager.count(), 1, "TestB の 1 体のみ残る");
    assert!(manager.get_mascot_with_affordance("b1").is_some());
    assert!(manager.get_mascot_with_affordance("a1").is_none());
    assert!(manager.get_mascot_with_affordance("a2").is_none());
}

/// dispose_all: 全員 dispose・末尾から逆順（Java L447-456）→ 削除反映は次 tick。
#[test]
fn manager_dispose_all_and_removal_applies_on_next_tick() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.add(tagged_mascot("a2", (200, 500)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2);

    manager.dispose_all();
    assert_eq!(
        manager.count(),
        2,
        "dispose 直後は削除未反映（次 tick で一括）"
    );
    manager.tick(Instant::now());
    assert!(manager.is_empty());
}

/// setBehaviorAll（Java L291-340）: 全員へ name を適用（set_behavior = init まで実行）。
#[test]
fn manager_set_behavior_all_applies_to_every_mascot() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (1000, 500)));
    manager.add(tagged_mascot("a2", (1500, 500)));
    manager.tick(Instant::now());

    manager.set_behavior_all("Stare");
    manager.tick(Instant::now());

    let m = manager
        .get_mascot_with_affordance("stare")
        .expect("全員が stare アフォーダンスを出す");
    assert_eq!(
        m.behavior_name(),
        Some("Stare"),
        "setBehaviorAll で Stare に着替え"
    );
    assert_eq!(m.anchor(), (1003, 500), "Stare の velocity (3,0) が効く");
}

/// setBehaviorAll の失敗（Java L301-306）: 未知名 → log + dispose（削除は次 tick）。
#[test]
fn manager_set_behavior_all_unknown_behavior_disposes_all() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.add(tagged_mascot("a2", (200, 500)));
    manager.tick(Instant::now());

    manager.set_behavior_all("Nope");
    manager.tick(Instant::now());
    assert!(
        manager.is_empty(),
        "全員が構築失敗 → dispose → 次 tick で削除"
    );
}

/// isPaused（Java L465-475）+ togglePauseAll（L480-495）:
/// 空 = false・allMatch 契約・空に対する toggle は no-op。
#[test]
fn manager_is_paused_uses_allmatch_and_toggle_flips_everyone() {
    let (env, _) = single_monitor_env();
    let mut empty_manager = make_manager(env);
    assert!(!empty_manager.is_paused(), "空 = false");
    empty_manager.toggle_pause_all(); // 空 → no-op（panic しないことを pin）

    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.add(tagged_mascot("a2", (200, 500)));
    manager.tick(Instant::now());

    manager
        .get_mascot_with_affordance("a1")
        .unwrap()
        .set_paused(true);
    assert!(!manager.is_paused(), "allMatch 未満 → false");

    manager.toggle_pause_all(); // allMatch=false → 全員 pause
    assert!(manager.is_paused());
    assert!(manager
        .get_mascot_with_affordance("a2")
        .unwrap()
        .is_paused());

    manager.toggle_pause_all(); // allMatch=true → 全員解除
    assert!(!manager.is_paused());
}

/// enabled=false の tick は何もしない（Java L166 / L503-515: tick 本呼び出しがなされない）:
/// 追加キュー・spawn キュー・env 更新も触らず・mascot も進まない。再 enabled 後に遅延分が
/// 反映される。時間前進の観測は behavior 付き mascot（spawn 経由）で行う:
/// behavior 無し mascot は time 不増のためである（Mascot.java L613-625: `time++` は
/// `behavior != null` の内側 = tests/mascot_test.rs `tick_without_behavior_does_not_advance_time`
/// と同一契約。Java 実行系でも behavior 無し mascot は Manager に登録されない
/// ・Breed.java L92-99 / Main.java L494: setBehavior 成功後のみ add）。
#[test]
fn manager_disabled_tick_does_nothing_and_delayed_work_applies_later() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.set_enabled(false);
    manager.add(tagged_mascot("a1", (100, 500)));
    spawn_into(&mut manager, "TestSet", (500, 500), false, "Walk");

    manager.tick(Instant::now());
    assert!(
        manager.is_empty(),
        "disabled 中は追加キューも spawn も反映しない"
    );

    manager.set_enabled(true);
    manager.tick(Instant::now());
    assert_eq!(
        manager.count(),
        2,
        "再 enabled 後に遅延分を反映（追加 1 体 + spawn 1 体）"
    );

    // behavior 付き mascot（spawn 経由）: 再 enabled 後の初 tick で 1 tick 進む（過剰補填しない）
    let child = manager
        .get_mascot_with_affordance("mark")
        .expect("spawn された mascot が同 tick で動き出す");
    assert_eq!(
        child.time(),
        1,
        "再 enabled 後の初 tick で 1 tick 進む（過剰補填しない）"
    );
    assert_eq!(
        child.anchor(),
        (501, 500),
        "velocity (1,0) が同 tick で適用"
    );

    // behavior 無し mascot は time 不増（Mascot.java L613-625 逐語）
    assert_eq!(
        manager.get_mascot_with_affordance("a1").unwrap().time(),
        0,
        "behavior 無し mascot の time は不増（Java L624: time++ は behavior != null の内側）"
    );
}

/// exitOnLastRemoved（Java L240-243・既定 true）: 全員消滅 tick 後に exit フラグのみ立つ
///（process::exit はしない・#10 が消費）。false なら立たない。
#[test]
fn manager_exit_flag_turns_on_when_last_removed() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.set_exit_on_last_removed(true);
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.tick(Instant::now());
    assert!(!manager.should_exit(), "1 体以上残っている間は false");

    manager.dispose_all();
    manager.tick(Instant::now());
    assert!(manager.should_exit(), "全員消滅 → exit flag");

    // flag false 経路
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env); // helper 内で false 設定済み
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.tick(Instant::now());
    manager.dispose_all();
    manager.tick(Instant::now());
    assert!(!manager.should_exit(), "flag false → exit は起きない");
}

/// overlap 検出（Java L581-601・同一 anchor 2 体以上）+ affordance 取得（L558-573）。
#[test]
fn manager_overlap_check_and_affordance_lookup() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (500, 500)));
    manager.add(tagged_mascot("cushion", (500, 500)));
    manager.add(tagged_mascot("a3", (600, 500)));
    manager.tick(Instant::now());

    assert!(
        manager.has_overlapping_mascots_at((500, 500)),
        "同 anchor 2 体 → true"
    );
    assert!(
        !manager.has_overlapping_mascots_at((600, 500)),
        "1 体のみ → false"
    );

    assert!(manager.get_mascot_with_affordance("cushion").is_some());
    assert!(manager.get_mascot_with_affordance("nope").is_none());
}

/// apply_all（⑤・#10 glue 注入）: 全員がリスト順に 1 回ずつ渡される +
/// sink から Mascot::clear_needs_repaint（Java apply L662-708 の needsRepaint=false 相当）を
/// 実施できる契約。
#[test]
fn manager_apply_all_passes_each_mascot_in_order() {
    let (env, _) = single_monitor_env();
    let mut manager = make_manager(env);
    manager.add(tagged_mascot("a1", (100, 500)));
    manager.add(tagged_mascot("a2", (200, 500)));
    manager.tick(Instant::now());

    let mut visited: Vec<i32> = Vec::new();
    manager.apply_all(|m| {
        visited.push(m.anchor().0);
        m.clear_needs_repaint();
    });
    assert_eq!(visited, [100, 200], "リスト順に全員が 1 回ずつ渡される");
    assert!(!manager
        .get_mascot_with_affordance("a1")
        .unwrap()
        .needs_repaint());
}

/// Mascot::clear_needs_repaint（(T) 合流契約・Java needsRepaint=false 相当）。
/// 初期値 true → クリアで false → 次回 set_image 異値で再度立ち。
#[test]
fn mascot_clear_needs_repaint_resets_flag() {
    let mut m = Mascot::new("TestSet", empty_image_set("TestSet"), (100, 100));
    assert!(m.needs_repaint(), "構築直後は needs_repaint = true");
    m.clear_needs_repaint();
    assert!(!m.needs_repaint());
    m.set_image(Some(ImageState {
        image_ref: "p.png".to_string(),
        center: (64, 64),
        width: 128,
        height: 128,
    }));
    assert!(m.needs_repaint(), "画像更新 → 再度立ち");
}

/// スケジューラ純関数（AGENTS §5-7・Java Manager.java L158-165 との意図的差異）:
/// - tick_due: elapsed >= 40ms で true
/// - next_delay: バースト（elapsed >= 2×interval）でも次回は 40ms 後（1 tick 分のみ進める）
#[test]
fn scheduler_tick_due_and_next_delay_clamps_bursts() {
    assert!(!Manager::tick_due(Duration::from_millis(39)));
    assert!(Manager::tick_due(Duration::from_millis(40)));
    assert!(Manager::tick_due(Duration::from_millis(100)));

    assert_eq!(
        Manager::next_delay(Duration::from_millis(10)),
        Duration::from_millis(30),
        "通常: 残り 30ms"
    );
    assert_eq!(
        Manager::next_delay(Duration::from_millis(40)),
        Duration::from_millis(40),
        "elapsed == 間隔: 次 tick はまた 40ms 後"
    );
    assert_eq!(
        Manager::next_delay(Duration::from_millis(80)),
        Duration::from_millis(40),
        "間隔 2 回分: 実行は 1 tick 分のみ・次回も 40ms 後（バーストクランプ）"
    );
    assert_eq!(
        Manager::next_delay(Duration::from_millis(100)),
        Duration::from_millis(40),
        "大遅延でも補填なし・次回 40ms 後"
    );
}
