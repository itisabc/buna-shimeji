//! タスク #6: src/mascot/{mod.rs,behavior.rs,animation.rs}（Mascot コア）の契約テスト。
//!
//! Java 正本（.tmp/java-ref/）を仕様として、公開契約の振る舞いのみを検証する:
//! - Mascot::tick（Mascot.java L613-656 逐語）: is_animating() = animating && !paused。
//!   behavior が Some のとき runner.next() を 1 回呼んで time++。Eval エラーでも
//!   time++ は try/catch の外側なので実行され、dispose 相当
//!   （remove_pending = true・animating = false）になる。behavior None なら time 不増
//! - BehaviorRunner::init / next（UserBehavior.java 逐語）: init で action.has_next ==
//!   false なら直ちに次行動へ遷移。next は hotspot scan → off-screen 再配置+Fall /
//!   完了遷移 / LostGround 処理 / Eval 伝播
//! - 再配置式（Java (int) キャスト = 切り捨て）: ((rng * (area_width - 2)) as i32)
//!   + area_left + 1, area_top - 256。area は multiscreen ? screen（全画面 union）:
//!
//!   Java `MascotEnvironment.getWorkArea()` = アンカーが属する作業領域
//!   （MascotEnvironment.java L66-114。全モニタ外は invisibleScreen 0×0 の quirk）
//! - 頻度選択（Configuration.java L459-524 逐語）: random = rng * total_frequency、
//!   候補は XML 順で random -= frequency; random < 0 で選択。total == 0 は
//!   再配置 + Fall フォールバック。条件の評価エラーは Err にせず候補をスキップ
//! - build_next_behavior の分岐: previous が None または next.add == true なら全
//!   top-level が候補、next.add == false なら next リストの参照のみ。条件評価の
//!   コンテキストは mascot.eval_snapshot() + env.eval_context() の合成（MascotContext）。
//!   #7a（design §1.7(f)）から mascot.environment.* は MascotContext が自前解決し、
//!   eval_context 委譲は非 env パスのみに残る（env パスの値検証は tests/env_test.rs）
//! - build_behavior / build_behavior_direct: 不存在 → Err(UnknownBehavior)
//! - mouse_pressed（L242-288）/ mouse_released（L298-319）: is_draggable と
//!   Dragged/Thrown 遷移、hotspot クリック中の cursor クリア
//! - Animation/Pose（Animation.java / Pose.java 逐語）: duration は合計、pose_at は
//!   time % duration 後の減算ウォーク、apply は anchor += (look_right ? -dx : dx, dy)、
//!   image はフレーム存在時のみ Some（center は反転時 width - anchor.x）。
//!   欠落フレームは prev_image 保持で get_bounds 復元
//! - Mascot 状態: set_image の同値 no-op / prev 更新 / get_bounds の復元、
//!   dispose → remove_pending + 停止
//!
//! 実装 (src/mascot/) は未存在のため cargo test は compile error = RED が正常。
//! すべて合成データで動作し、実資産ファイル・tests/common には依存しない。

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;
use std::sync::Arc;

use shimeji::config::script::{EvalContext, EvalError, Variable, Variables};
use shimeji::config::{
    Animation, BehaviorDef, BehaviorEntry, BehaviorRef, BehaviorsConfig, NextBehaviorList, Pose,
    SequenceChild, VarMap,
};
use shimeji::mascot::animation::{
    animation_duration, animation_init_condition, animation_is_effective, animation_pose_at,
    animation_reset_condition, apply_pose,
};
use shimeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use shimeji::mascot::env::{AreaSlot, AreaState};
use shimeji::mascot::{
    EnvironmentView, EvalSnapshot, ImageState, Mascot, MascotContext, Rect, Rng,
};
use shimeji::render::imageset::{Frame, ImageSet};

// =====================================================================
// 共通モック（自己完結。tests/common は使わない・変更しない）
// =====================================================================

/// Action / Factory の呼び出し記録。
enum Call {
    Build(String),
    Init,
    HasNext,
    Next,
    IsDraggable,
}

type Log = Rc<RefCell<Vec<Call>>>;

fn new_log() -> Log {
    Rc::new(RefCell::new(Vec::new()))
}

fn count_calls(log: &Log, pred: impl Fn(&Call) -> bool) -> usize {
    log.borrow().iter().filter(|c| pred(c)).count()
}

fn count_init(log: &Log) -> usize {
    count_calls(log, |c| matches!(c, Call::Init))
}

fn count_next(log: &Log) -> usize {
    count_calls(log, |c| matches!(c, Call::Next))
}

fn count_is_draggable(log: &Log) -> usize {
    count_calls(log, |c| matches!(c, Call::IsDraggable))
}

fn build_names(log: &Log) -> Vec<String> {
    log.borrow()
        .iter()
        .filter_map(|c| match c {
            Call::Build(n) => Some(n.clone()),
            _ => None,
        })
        .collect()
}

fn eval_error() -> EvalError {
    EvalError {
        expr: "test-expr".to_string(),
        message: "テスト用の評価エラー".to_string(),
    }
}

/// action メソッドの戻り値計画。
#[derive(Clone)]
enum Plan {
    Ok,
    Eval,
    LostGround,
}

impl Plan {
    fn result(&self) -> Result<(), ActionError> {
        match self {
            Plan::Ok => Ok(()),
            Plan::Eval => Err(ActionError::Eval(eval_error())),
            Plan::LostGround => Err(ActionError::LostGround),
        }
    }
}

/// MockAction の振る舞い仕様。`has_next_true_times` は has_next が true を返す回数
/// （それ以降は false。usize::MAX で常に true）。既定は「常時継続・正常・ドラッグ可」。
#[derive(Clone)]
struct ActionScript {
    has_next_true_times: usize,
    init: Plan,
    next: Plan,
    draggable_ok: bool,
    draggable_err: bool,
}

impl Default for ActionScript {
    fn default() -> Self {
        ActionScript {
            has_next_true_times: usize::MAX,
            init: Plan::Ok,
            next: Plan::Ok,
            draggable_ok: true,
            draggable_err: false,
        }
    }
}

struct MockAction {
    log: Log,
    script: ActionScript,
    has_next_calls: usize,
}

impl Action for MockAction {
    fn init(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.log.borrow_mut().push(Call::Init);
        self.script.init.result()
    }

    fn has_next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.has_next_calls += 1;
        self.log.borrow_mut().push(Call::HasNext);
        Ok(self.has_next_calls <= self.script.has_next_true_times)
    }

    fn next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.log.borrow_mut().push(Call::Next);
        self.script.next.result()
    }

    fn is_draggable(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.log.borrow_mut().push(Call::IsDraggable);
        if self.script.draggable_err {
            Err(ActionError::Eval(eval_error()))
        } else {
            Ok(self.script.draggable_ok)
        }
    }
}

/// build_action を記録し、名前に応じた ActionScript で MockAction を返す。
/// 未登録名（と Inline 子）は既定の ActionScript。
struct MockFactory {
    log: Log,
    scripts: HashMap<String, ActionScript>,
}

impl MockFactory {
    fn new(log: &Log) -> Self {
        MockFactory {
            log: log.clone(),
            scripts: HashMap::new(),
        }
    }

    fn with_script(mut self, name: &str, script: ActionScript) -> Self {
        self.scripts.insert(name.to_string(), script);
        self
    }
}

impl BehaviorFactory for MockFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        let name = match child {
            SequenceChild::Ref { name, .. } => Some(name.clone()),
            SequenceChild::Inline(_) => None,
        };
        if let Some(n) = &name {
            self.log.borrow_mut().push(Call::Build(n.clone()));
        }
        let script = name
            .as_ref()
            .and_then(|n| self.scripts.get(n).cloned())
            .unwrap_or_default();
        Ok(Box::new(MockAction {
            log: self.log.clone(),
            script,
            has_next_calls: 0,
        }))
    }
}

/// env.eval_context() が返す評価コンテキスト。既知パスのみ値を返し、
/// 不明パスは None（= 評価器が未知パスを要求したら Err になる検出器）。
///
/// [契約変更 #7a・design §1.7(d)] 旧契約ではこのモックが mascot.environment.* の
/// 委譲先として env パス（workArea.left=42.0 / activeIE.visible / floor isOn）に
/// 応答していたが、#7a から mascot.environment.* は MascotContext が自前解決するため
/// eval_context は env パスに応答しなくてよい（platform primitives のみでよい）。
/// これに従いモックは非 env パス（mascot.custom.*）のみに応答する。
/// env パスの自前解決値の検証は tests/env_test.rs（拡張 10 メソッドを実装する
/// SynthEnv + Java 期待値の焼き込み）が担う。
struct MockEvalCtx {
    is_on_calls: RefCell<Vec<(String, f64, f64)>>,
}

impl EvalContext for MockEvalCtx {
    fn number(&self, path: &str) -> Option<f64> {
        match path {
            "mascot.custom.probe" => Some(777.0),
            _ => None,
        }
    }

    fn boolean(&self, path: &str) -> Option<bool> {
        match path {
            "mascot.custom.flag" => Some(true),
            _ => None,
        }
    }

    fn is_on(&self, target: &str, x: f64, y: f64) -> bool {
        self.is_on_calls
            .borrow_mut()
            .push((target.to_string(), x, y));
        target == "mascot.custom.border"
    }
}

/// queue_spawn の呼び出し記録（#8: BornBehaviour 名が第 4 引数で渡る契約 pin・
/// tests/action_test.rs の SpawnRec と同形）。
#[derive(Debug, Clone, PartialEq)]
struct SpawnRecord {
    image_set_name: String,
    anchor: (i32, i32),
    look_right: bool,
    behavior_name: String,
}

/// 合成モニタ 1 つ分の AreaState（deltas 0・visible 指定。Area.java 既定相当）。
fn area_state(left: i32, top: i32, right: i32, bottom: i32, visible: bool) -> AreaState {
    AreaState {
        left,
        top,
        right,
        bottom,
        dleft: 0,
        dtop: 0,
        dright: 0,
        dbottom: 0,
        visible,
    }
}

/// 複数モニタの screen union（Java `Environment.getScreen()` 相当）。
fn union_area(areas: &[AreaState]) -> AreaState {
    let mut u = areas[0];
    for a in &areas[1..] {
        u.left = u.left.min(a.left);
        u.top = u.top.min(a.top);
        u.right = u.right.max(a.right);
        u.bottom = u.bottom.max(a.bottom);
    }
    u
}

/// EnvironmentView モック。既定: work area=(0,0,1920,1040) / screen=(0,0,1920,1080) /
/// multiscreen=false の単一モニタ。
///
/// queue_spawn は 4 引数（#8 拡張・BornBehaviour 名が queue に伝播する契約）で
/// 観測可能にした。複数モニタのアンカー基準 work area 解決（`resolve_work_area`）を
/// 決定的に検証するため、#7a 拡張メソッドのうち `screens` / `work_area_at` /
/// `work_area_state` / `screen_area` だけを実装する（残りは default のまま）。
struct MockEnv {
    /// monitor index 順の作業領域（`screens` と 1:1）。
    work_areas: Vec<AreaState>,
    /// monitor index 順の画面矩形。
    screens: Vec<AreaState>,
    multiscreen: bool,
    ctx: MockEvalCtx,
    spawns: RefCell<Vec<SpawnRecord>>,
    /// behavior_disabled(name) が true を返す行動名（= toggle 後「無効」を表現する・#9）。
    disabled: Vec<String>,
    /// behavior_disabled の呼び出し記録 (image_set, behavior_name)。
    behavior_checks: RefCell<Vec<(String, String)>>,
}

impl MockEnv {
    fn new() -> Self {
        MockEnv {
            work_areas: vec![area_state(0, 0, 1920, 1040, true)],
            screens: vec![area_state(0, 0, 1920, 1080, true)],
            multiscreen: false,
            ctx: MockEvalCtx {
                is_on_calls: RefCell::new(Vec::new()),
            },
            spawns: RefCell::new(Vec::new()),
            disabled: Vec::new(),
            behavior_checks: RefCell::new(Vec::new()),
        }
    }

    /// behavior_disabled が true（無効）を返す行動名を登録する（#9）。
    fn disable(mut self, name: &str) -> Self {
        self.disabled.push(name.to_string());
        self
    }

    /// multiscreen = true に切り替え、screen を 2560 幅の仮想画面へ変える。
    fn with_multiscreen(mut self) -> Self {
        self.multiscreen = true;
        self.screens = vec![area_state(0, 0, 2560, 1080, true)];
        self
    }

    /// 複数モニタ構成へ差し替える（`screens` / `work_areas` は monitor index 順で 1:1）。
    fn with_monitors(mut self, screens: Vec<AreaState>, work_areas: Vec<AreaState>) -> Self {
        self.screens = screens;
        self.work_areas = work_areas;
        self
    }
}

impl EnvironmentView for MockEnv {
    fn work_area(&self) -> Rect {
        let a = self.work_areas[0];
        Rect {
            left: a.left,
            top: a.top,
            right: a.right,
            bottom: a.bottom,
        }
    }

    fn screen(&self) -> Rect {
        let u = union_area(&self.screens);
        Rect {
            left: u.left,
            top: u.top,
            right: u.right,
            bottom: u.bottom,
        }
    }

    fn multiscreen(&self) -> bool {
        self.multiscreen
    }

    fn eval_context(&self) -> &dyn EvalContext {
        &self.ctx
    }

    fn screen_area(&self) -> AreaState {
        union_area(&self.screens)
    }

    fn screens(&self) -> Vec<AreaState> {
        self.screens.clone()
    }

    fn work_area_at(&self, x: i32, y: i32) -> AreaSlot {
        // AbstractEnvironment.getWorkAreaAt（L186-193）: 点を含む work area を
        // monitor index 順に探し、無ければ invisibleScreen。
        match self.work_areas.iter().position(|a| a.contains(x, y)) {
            Some(i) => AreaSlot::WorkArea(i),
            None => AreaSlot::Invisible,
        }
    }

    fn work_area_state(&self, slot: AreaSlot) -> AreaState {
        match slot {
            AreaSlot::WorkArea(i) => self.work_areas[i],
            AreaSlot::Screen(i) => self.screens[i],
            AreaSlot::Invisible => area_state(0, 0, 0, 0, false),
            AreaSlot::ActiveWindow => todo!("MockEnv は ActiveWindow を使わない"),
        }
    }

    fn behavior_disabled(&self, image_set: &str, behavior_name: &str) -> bool {
        self.behavior_checks
            .borrow_mut()
            .push((image_set.to_string(), behavior_name.to_string()));
        self.disabled.iter().any(|n| n == behavior_name)
    }

    fn queue_spawn(
        &self,
        image_set_name: &str,
        anchor: (i32, i32),
        look_right: bool,
        behavior_name: &str,
    ) {
        self.spawns.borrow_mut().push(SpawnRecord {
            image_set_name: image_set_name.to_string(),
            anchor,
            look_right,
            behavior_name: behavior_name.to_string(),
        });
    }
}

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、枯渇 = 過剰消費として panic する。
struct FakeRng {
    values: Vec<f64>,
    consumed: usize,
}

impl FakeRng {
    fn new(values: &[f64]) -> Self {
        FakeRng {
            values: values.to_vec(),
            consumed: 0,
        }
    }

    fn consumed(&self) -> usize {
        self.consumed
    }
}

impl Rng for FakeRng {
    fn unit(&mut self) -> f64 {
        let v = *self.values.get(self.consumed).unwrap_or_else(|| {
            panic!(
                "FakeRng 枯渇: 乱数が {} 回要求された（用意は {} 個）",
                self.consumed + 1,
                self.values.len()
            )
        });
        self.consumed += 1;
        v
    }
}

// =====================================================================
// 合成データ構築ヘルパ（実資産ファイルは使わない）
// =====================================================================

fn empty_image_set() -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

fn image_set_with(frames: &[(&str, u32, u32)]) -> Arc<ImageSet> {
    let mut map = BTreeMap::new();
    for (name, width, height) in frames {
        map.insert(
            name.to_string(),
            Frame::from_rgba(
                *width,
                *height,
                vec![0u8; (*width as usize) * (*height as usize) * 4],
            ),
        );
    }
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: map,
        warnings: Vec::new(),
        scale: 1.0,
    })
}

fn mascot_at(anchor: (i32, i32)) -> Mascot {
    Mascot::new("TestSet", empty_image_set(), anchor)
}

fn var(source: &str) -> Variable {
    Variable::parse(source)
}

fn pose(image: &str, anchor: (i32, i32), velocity: (i32, i32), duration: i32) -> Pose {
    Pose {
        image: image.to_string(),
        anchor,
        velocity,
        duration,
    }
}

fn anim(poses: Vec<Pose>) -> Animation {
    Animation {
        condition: None,
        poses,
        is_turn: false,
    }
}

fn anim_with(condition: Variable, poses: Vec<Pose>) -> Animation {
    Animation {
        condition: Some(condition),
        poses,
        is_turn: false,
    }
}

fn state(image_ref: &str, center: (i32, i32), width: u32, height: u32) -> ImageState {
    ImageState {
        image_ref: image_ref.to_string(),
        center,
        width,
        height,
    }
}

/// 画面内に十分な bounds を持たせるための合成フレーム状態（128x128・center (64,64)）。
fn on_screen_image() -> ImageState {
    state("shime1.png", (64, 64), 128, 128)
}

fn def(name: &str, frequency: i32, next: Option<NextBehaviorList>) -> BehaviorDef {
    BehaviorDef {
        name: name.to_string(),
        frequency,
        hidden: false,
        toggleable: false,
        action: SequenceChild::Ref {
            name: name.to_string(),
            attrs: VarMap::new(),
        },
        next,
    }
}

/// Toggleable="true" 相当の BehaviorDef（#9）。
fn def_toggle(name: &str, frequency: i32, next: Option<NextBehaviorList>) -> BehaviorDef {
    BehaviorDef {
        toggleable: true,
        ..def(name, frequency, next)
    }
}

fn single(name: &str, frequency: i32) -> BehaviorEntry {
    BehaviorEntry::Single(def(name, frequency, None))
}

fn grouped(conditions: Vec<Variable>, name: &str, frequency: i32) -> BehaviorEntry {
    BehaviorEntry::Group {
        conditions,
        behaviors: vec![def(name, frequency, None)],
    }
}

fn table(entries: Vec<BehaviorEntry>) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig {
        entries,
        ..Default::default()
    })
}

/// 定数付きの行動表（Java `Configuration.constants` 相当）。
fn table_with_constants(entries: Vec<BehaviorEntry>, constants: &[(&str, &str)]) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig {
        entries,
        constants: constants
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    })
}

fn next_list(add: bool, refs: &[(&str, i32)]) -> NextBehaviorList {
    NextBehaviorList {
        add,
        references: refs
            .iter()
            .map(|(name, frequency)| BehaviorRef {
                name: name.to_string(),
                frequency: *frequency,
                condition: None,
            })
            .collect(),
    }
}

// =====================================================================
// animation.rs — Animation / Pose（Animation.java・Pose.java 逐語）
// =====================================================================

#[test]
fn animation_duration_sums_pose_durations() {
    let a = anim(vec![
        pose("p0.png", (0, 0), (0, 0), 30),
        pose("p1.png", (0, 0), (0, 0), 20),
        pose("p2.png", (0, 0), (0, 0), 10),
    ]);
    assert_eq!(animation_duration(&a), 60);
}

#[test]
fn animation_pose_at_walks_poses_and_wraps() {
    let a = anim(vec![
        pose("p0.png", (0, 0), (0, 0), 30),
        pose("p1.png", (0, 0), (0, 0), 20),
        pose("p2.png", (0, 0), (0, 0), 10),
    ]);
    let image_at = |time: i32| animation_pose_at(&a, time).map(|p| p.image.clone());
    // time % duration 後に各 pose.duration を順に引いて time < 0 になった pose
    assert_eq!(image_at(0), Some("p0.png".to_string())); // 0-30 < 0
    assert_eq!(image_at(29), Some("p0.png".to_string()));
    assert_eq!(image_at(45), Some("p1.png".to_string())); // 45-30=15 → 15-20 < 0
    assert_eq!(image_at(59), Some("p2.png".to_string())); // 29-20=9 → 9-10 < 0
    assert_eq!(image_at(60), Some("p0.png".to_string())); // 60 % 60 = 0
    assert_eq!(image_at(62), Some("p0.png".to_string())); // 62 % 60 = 2
    assert_eq!(image_at(130), Some("p0.png".to_string())); // 130 % 60 = 10
}

#[test]
fn animation_is_effective_evaluates_condition() {
    let env = MockEnv::new();
    let ctx = env.eval_context();
    let frame = vec![pose("p.png", (0, 0), (0, 0), 1)];

    // condition なし → 常に true
    let no_condition = anim(frame.clone());
    assert!(animation_is_effective(&no_condition, &mut Variables::new(), ctx).unwrap());

    let mut vars = Variables::new();
    assert!(
        animation_is_effective(&anim_with(var("${1 == 1}"), frame.clone()), &mut vars, ctx)
            .unwrap()
    );
    assert!(
        !animation_is_effective(&anim_with(var("${1 == 2}"), frame.clone()), &mut vars, ctx)
            .unwrap()
    );
    // 不明な mascot パス → panic せず Err（ログ内容は assert しない）
    assert!(animation_is_effective(
        &anim_with(var("${mascot.unknown.path == 1}"), frame),
        &mut vars,
        ctx
    )
    .is_err());
}

#[test]
fn animation_init_and_reset_condition_control_reevaluation() {
    let env = MockEnv::new();
    let ctx = env.eval_context();
    let frame = vec![pose("p.png", (0, 0), (0, 0), 1)];
    let hash = anim_with(var("#{FootX == 0}"), frame.clone()); // フレーム毎に再評価
    let dollar = anim_with(var("${FootX == 0}"), frame); // アクション開始時のみ再評価
    let mut vars = Variables::new();
    vars.inject("FootX", 0.0);

    assert!(animation_is_effective(&hash, &mut vars, ctx).unwrap());
    assert!(animation_is_effective(&dollar, &mut vars, ctx).unwrap());

    // 注入変数を変えても ${} はキャッシュを維持する
    vars.inject("FootX", 5.0);

    // reset_condition（フレーム開始相当）: #{} のキャッシュだけが消える
    animation_reset_condition(&hash, &mut vars).unwrap();
    assert!(!animation_is_effective(&hash, &mut vars, ctx).unwrap());
    animation_reset_condition(&dollar, &mut vars).unwrap();
    assert!(animation_is_effective(&dollar, &mut vars, ctx).unwrap());

    // init_condition（アクション開始相当）: 全式が再評価される
    animation_init_condition(&dollar, &mut vars).unwrap();
    assert!(!animation_is_effective(&dollar, &mut vars, ctx).unwrap());
}

// =====================================================================
// MascotContext / スナップショット（契約 15）
// =====================================================================

#[test]
fn mascot_context_resolves_snapshot_vars_and_delegates_non_env_paths() {
    // [契約変更 #7a・design §1.7(d)/(f)] 旧契約: snapshot 外の全パス（mascot.environment.*
    // を含む）を env.eval_context() へ委譲し、本テストは env パスの委譲値
    // （workArea.left == Some(42.0) / activeIE.visible == Some(true) /
    // floor isOn == true）を pin していた。
    // 新契約: mascot.environment.* は MascotContext が自前解決
    // （snapshot.anchor/look_right + env primitives・Java MascotEnvironment の式と同一）
    // し、eval_context への委譲は非 env パスのみに残る。
    //  - env パスの自前解決値（29 パス）の検証は tests/env_test.rs へ移管
    //    （SynthEnv は拡張 10 メソッドを実装する。MockEnv は拡張メソッドの default 実装
    //    todo!("app impl at #8") に依存するため、ここで env パスを評価しない）
    //  - ここで pin するのは差分契約の後半: snapshot 変数の解決と非 env パスの委譲維持
    let env = MockEnv::new();
    let snapshot = EvalSnapshot {
        anchor: (123, 456),
        look_right: true,
        total_count: 7,
    };
    let ctx = MascotContext {
        snapshot: &snapshot,
        env: &env,
    };

    // スナップショット由来の mascot 変数（変更なし）
    assert_eq!(ctx.number("mascot.anchor.x"), Some(123.0));
    assert_eq!(ctx.number("mascot.anchor.y"), Some(456.0));
    assert_eq!(ctx.number("mascot.totalCount"), Some(7.0));
    assert_eq!(ctx.boolean("mascot.lookRight"), Some(true));

    // 非 env パスは env.eval_context() へ同一パスで委譲される（変更なし・対象が非 env のみに縮小）
    assert_eq!(ctx.number("mascot.custom.probe"), Some(777.0));
    assert_eq!(ctx.boolean("mascot.custom.flag"), Some(true));
    assert!(ctx.is_on("mascot.custom.border", 1.0, 2.0));
    assert!(!ctx.is_on("mascot.custom.other", 1.0, 2.0)); // 未登録ターゲットは false
    assert_eq!(env.ctx.is_on_calls.borrow().len(), 2); // is_on も委譲
}

#[test]
fn eval_snapshot_reflects_mascot_state() {
    let mut m = mascot_at((11, 22));
    assert_eq!(m.image_set_name(), "TestSet");
    m.set_look_right(true);
    m.set_total_count(5);
    assert_eq!(
        m.eval_snapshot(),
        EvalSnapshot {
            anchor: (11, 22),
            look_right: true,
            total_count: 5,
        }
    );
    m.set_anchor((33, 44));
    assert_eq!(m.anchor(), (33, 44));
    assert_eq!(m.eval_snapshot().anchor, (33, 44));
}

// =====================================================================
// Mascot 状態 — set_image / get_bounds / dispose（契約 14）
// =====================================================================

#[test]
fn set_image_dedupes_updates_prev_image_and_get_bounds_restores() {
    let mut m = mascot_at((1000, 500));
    let x = state("shime1.png", (100, 48), 128, 128);
    let y = state("shime2.png", (64, 64), 64, 64);

    // 画像を一度も set していない → bounds は None
    assert_eq!(m.get_bounds(), None);

    m.set_image(Some(x.clone()));
    assert_eq!(m.image_anchor(), Some((100, 48)));
    assert_eq!(
        m.get_bounds(),
        Some(Rect {
            left: 900,
            top: 452,
            right: 1028,
            bottom: 580
        })
    );

    // 同じ値で再 set → no-op（needs_repaint 変化なし）
    let after_first = m.needs_repaint();
    m.set_image(Some(x.clone()));
    assert_eq!(
        m.needs_repaint(),
        after_first,
        "同値の set_image は no-op のはず"
    );
    assert_eq!(
        m.image().map(|i| i.image_ref.clone()),
        Some("shime1.png".to_string())
    );

    // 異なる値 → needs_repaint = true・prev 更新
    m.set_image(Some(y.clone()));
    assert!(m.needs_repaint());
    assert_eq!(
        m.get_bounds(),
        Some(Rect {
            left: 936,
            top: 436,
            right: 1000,
            bottom: 500
        })
    );

    // None → prev を保持し、get_bounds は最後の非 null 画像から復元される
    m.set_image(None);
    assert!(m.needs_repaint());
    assert_eq!(m.image(), None);
    assert_eq!(
        m.get_bounds(),
        Some(Rect {
            left: 936,
            top: 436,
            right: 1000,
            bottom: 500
        })
    );
}

#[test]
fn dispose_marks_remove_pending_and_stops_animating() {
    let mut m = mascot_at((0, 0));
    assert!(!m.remove_pending());
    m.dispose();
    assert!(m.remove_pending());
    assert!(!m.is_animating());
}

// =====================================================================
// BehaviorTable — フラット化 / 頻度選択 / 分岐 / フォールバック（契約 4〜7）
// =====================================================================

#[test]
fn behavior_table_flattens_groups_preserves_order_and_finds_by_name() {
    let cond_a = var("${1 == 1}");
    let cond_b = var("${2 == 2}");
    let t = table(vec![
        BehaviorEntry::Group {
            conditions: vec![cond_a.clone()],
            behaviors: vec![def("Walk", 100, None), def("Stare", 1, None)],
        },
        single("Fall", 200),
        BehaviorEntry::Group {
            conditions: vec![cond_a.clone(), cond_b.clone()],
            behaviors: vec![def("ChaseMouse", 1, None)],
        },
    ]);

    // XML 順保持（Group 展開込み）
    let names: Vec<&str> = t.rows.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, vec!["Walk", "Stare", "Fall", "ChaseMouse"]);

    // Group 条件の AND 積み上げ（Single は条件なし）
    let freqs_and_conds: Vec<(i32, usize)> = t
        .rows
        .iter()
        .map(|r| (r.frequency, r.conditions.len()))
        .collect();
    assert_eq!(freqs_and_conds, vec![(100, 1), (1, 1), (200, 0), (1, 2)]);
    assert_eq!(t.rows[3].conditions, vec![cond_a, cond_b]);

    let fall = t.find("Fall").expect("Fall が見つかるはず");
    assert_eq!(fall.name, "Fall");
    assert!(t.find("Nope").is_none());
}

#[test]
fn build_next_behavior_selects_candidate_by_frequency_in_xml_order() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![single("Walk", 100), single("ChaseMouse", 1)]);

    // random = rng * total_frequency、候補は XML 順で random -= frequency; < 0 で選択
    let mut rng = FakeRng::new(&[0.0]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Walk"); // 0 - 100 < 0 → 先頭候補
    assert_eq!(rng.consumed(), 1); // total_frequency > 0 のとき 1 回だけ消費

    let mut rng = FakeRng::new(&[0.9999]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "ChaseMouse"); // 0.9999*101=100.99 → Walk で残 0.99 → ChaseMouse
    assert_eq!(rng.consumed(), 1);
    assert_eq!(m.anchor(), (500, 500)); // 選択成功時は再配置しない
}

#[test]
fn build_next_behavior_skips_erroring_and_false_conditions() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));

    // 評価エラーの条件 → Err にせずその候補をスキップ。false の条件 → 除外
    let t = table(vec![
        grouped(vec![var("${mascot.unknown.path == 1}")], "Broken", 100),
        grouped(vec![var("${1 == 2}")], "FalseCond", 100),
        single("Good", 1),
    ]);

    // rng 0.5: 正しくスキップされれば total=1 → Good。
    // スキップ漏れなら total>=101 で Broken/FalseCond が選ばれるため判別できる
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Good");
}

#[test]
fn build_next_behavior_resolves_constants_in_conditions() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    m.set_total_count(7);

    // 定数 `maxCount` を条件から参照する（デレマス Anzu の資産と同型）。
    // MockEvalCtx の mascot.totalCount は 7。
    let entries = || {
        vec![
            grouped(vec![var("#{mascot.totalCount < maxCount}")], "Rare", 100),
            single("Common", 1),
        ]
    };

    // maxCount=10 > 7 → Rare が候補（定数が解決される）
    let t = table_with_constants(entries(), &[("maxCount", "10")]);
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Rare");

    // maxCount=3 < 7 → 条件 false → Common
    let t_false = table_with_constants(entries(), &[("maxCount", "3")]);
    let mut rng = FakeRng::new(&[0.0]);
    let runner = t_false
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Common");

    // 対照: 定数なしでは unknown identifier で候補スキップ → Common
    let t_missing = table(entries());
    let mut rng = FakeRng::new(&[0.0]);
    let runner = t_missing
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Common");
}

#[test]
fn build_next_behavior_uses_next_list_refs_when_not_additive() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));

    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 1, Some(next_list(false, &[("RefOnly", 1)])))),
        single("RefOnly", 1),
        single("Fall", 100),
    ]);

    // add=false → next リストの参照のみが候補（total=1）。
    // 誤って top-level も混ざると total=102 で rng 0.9999 は Fall を選ぶため判別できる
    let mut rng = FakeRng::new(&[0.9999]);
    let runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "RefOnly");
}

#[test]
fn build_next_behavior_adds_top_level_when_next_is_additive() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));

    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 1, Some(next_list(true, &[("RefOnly", 1)])))),
        single("Fall", 200),
        single("RefOnly", 1),
    ]);

    // add=true → top-level が候補に加わる（total=1+200+1+1=203）。
    // rng 0.5 → Walk(1) を超え Fall(200) で選ばれる。参照のみなら RefOnly になるため判別できる
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
}

#[test]
fn build_next_behavior_falls_back_repositioning_above_area() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);

    // 候補の total_frequency == 0 → 再配置 + Fall フォールバック
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 0, Some(next_list(false, &[("Fall", 0)])))),
        single("Fall", 100),
    ]);

    // 焼き込み: work_area=(0,0,1920,1040)・rng 0.5 → (int)(0.5*(1920-2))+0+1 = 960 / top-256
    let mut m = mascot_at((500, 500));
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    assert_eq!(m.anchor(), (960, -256));
    assert_eq!(rng.consumed(), 1); // 再配置でのみ 1 回消費

    // rng 0.9999 → (int)(1917.80..) + 1 = 1918（Java (int) キャスト = 切り捨て・丸めでない）
    let mut m = mascot_at((500, 500));
    let mut rng = FakeRng::new(&[0.9999]);
    let _runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.anchor(), (1918, -256));

    // multiscreen = true → work_area ではなく screen が使われる
    let env_multi = MockEnv::new().with_multiscreen();
    let mut m = mascot_at((500, 500));
    let mut rng = FakeRng::new(&[0.5]);
    let _runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env_multi, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.anchor(), (1280, -256)); // screen=(0,0,2560,1080): (int)(0.5*2558)+1
}

/// 契約: multiscreen=false の再配置 area は `env.work_area()`（プライマリ固定）ではなく
/// Java `MascotEnvironment.getWorkArea()` = **アンカーが属する作業領域**
/// （MascotEnvironment.java L66-114 の決定木）。アンカーが副モニタ内なら副モニタの
/// 作業領域で式を評価する。
#[test]
fn build_next_behavior_reposition_uses_anchor_monitor_work_area_when_multiscreen_off() {
    // 横並び 2 モニタ（x=1920 共有）。プライマリ = index 0。
    let env = MockEnv::new().with_monitors(
        vec![
            area_state(0, 0, 1920, 1080, true),
            area_state(1920, 0, 3840, 1080, true),
        ],
        vec![
            area_state(0, 0, 1920, 1040, true),
            area_state(1920, 0, 3840, 1040, true),
        ],
    );
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 0, Some(next_list(false, &[("Fall", 0)])))),
        single("Fall", 100),
    ]);

    let mut m = mascot_at((2000, 500)); // 副モニタ (1920..3840) の作業領域内
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    // 副モニタ作業領域 (1920,0,3840,1040): (int)(0.5*(1920-2)) + 1920 + 1 = 959+1921 = 2880
    // （プライマリ基準の現行実装なら (int)(0.5*1918)+1 = 960 になり FAIL する値）
    assert_eq!(m.anchor(), (2880, -256));
    assert_eq!(rng.consumed(), 1);
}

/// 契約: multiscreen=false かつアンカーが全モニタ外 → `resolve_work_area` は
/// invisibleScreen（0×0・visible=false）を返し、Java quirk どおり式が
/// `(int)(rng * -2) + 1` になる（x ∈ {0,1}）。プライマリ幅の乱択にはならない。
#[test]
fn build_next_behavior_reposition_uses_invisible_area_when_anchor_offscreen() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 0, Some(next_list(false, &[("Fall", 0)])))),
        single("Fall", 100),
    ]);

    // rng 0.5 → (int)(0.5*(0-2)) + 0 + 1 = (int)(-1.0) + 1 = 0
    let mut m = mascot_at((5000, 5000)); // 全モニタ外
    let mut rng = FakeRng::new(&[0.5]);
    let _runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.anchor(), (0, -256));

    // rng 0.0 → (int)(0*(-2)) + 0 + 1 = 1（x ∈ {0,1} の上端）
    let mut m = mascot_at((5000, 5000));
    let mut rng = FakeRng::new(&[0.0]);
    let _runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.anchor(), (1, -256));
}

/// 契約: multiscreen=false でもアンカーが「画面内だが作業領域外」
/// （タスクバー帯相当）なら、getWorkArea の決定木③以前のフォールバック
/// （MascotEnvironment.java L104-109）で**スクリーン領域**が使われる。
#[test]
fn build_next_behavior_reposition_uses_screen_when_anchor_in_taskbar_band() {
    // 左タスクバー: screen=(0,0,1920,1080) / 作業領域=(100,0,1820,1080)
    let env = MockEnv::new().with_monitors(
        vec![area_state(0, 0, 1920, 1080, true)],
        vec![area_state(100, 0, 1820, 1080, true)],
    );
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 0, Some(next_list(false, &[("Fall", 0)])))),
        single("Fall", 100),
    ]);

    let mut m = mascot_at((50, 500)); // 画面内 (0..1920)・作業領域外 (x < 100)
    let mut rng = FakeRng::new(&[0.9999]);
    let _runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    // screen=(0,0,1920,1080): (int)(0.9999*1918) + 0 + 1 = 1917+1 = 1918
    // （作業領域 (100,0,1820,1080) 基準の現行実装なら (int)(0.9999*1718)+101 = 1818 で FAIL）
    assert_eq!(m.anchor(), (1918, -256));
}

/// 契約（回帰）: multiscreen=true は従来どおり `env.screen()`（全画面 union）を使い、
/// アンカーが副モニタ内でもアンカー基準の作業領域には切り替わらない。
#[test]
fn build_next_behavior_reposition_uses_screen_union_when_multiscreen_on() {
    let env = MockEnv::new().with_monitors(
        vec![
            area_state(0, 0, 1920, 1080, true),
            area_state(1920, 0, 3840, 1080, true),
        ],
        vec![
            area_state(0, 0, 1920, 1040, true),
            area_state(1920, 0, 3840, 1040, true),
        ],
    );
    let env = MockEnv {
        multiscreen: true,
        ..env
    };
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 0, Some(next_list(false, &[("Fall", 0)])))),
        single("Fall", 100),
    ]);

    let mut m = mascot_at((2000, 500));
    let mut rng = FakeRng::new(&[0.5]);
    let _runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    // screen union=(0,0,3840,1080): (int)(0.5*3838) + 1 = 1919+1 = 1920
    // （誤ってアンカー基準の副モニタ作業領域を使うと 2880 になり FAIL）
    assert_eq!(m.anchor(), (1920, -256));
}

#[test]
fn build_behavior_builds_named_runner_and_errors_on_unknown() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![single("Walk", 100), single("Fall", 100)]);

    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Walk");

    let runner = t.build_behavior_direct("Walk", &mut factory, &m).unwrap();
    assert_eq!(runner.name, "Walk");

    match t.build_behavior("Nope", &mut m, &env, &mut factory, &mut rng) {
        Err(BehaviorError::UnknownBehavior(n)) => assert_eq!(n, "Nope"),
        Ok(_) => panic!("存在しない Behavior への build_behavior は Err が期待されます"),
        Err(_) => panic!("エラー種別は UnknownBehavior が期待されます"),
    }

    match t.build_behavior_direct("Nope", &mut factory, &m) {
        Err(BehaviorError::UnknownBehavior(n)) => assert_eq!(n, "Nope"),
        Ok(_) => panic!("存在しない Behavior への build_behavior_direct は Err が期待されます"),
        Err(_) => panic!("エラー種別は UnknownBehavior が期待されます"),
    }
}

// =====================================================================
// Toggleable（#9・Configuration.java L481/L496/L540-550/L583-604 逐語）
// =====================================================================

#[test]
fn build_next_behavior_excludes_disabled_toggleable_candidates() {
    // toggleable 行動 Walk が env に無効と返された → 候補から除外され Fall が選ばれる
    let env = MockEnv::new().disable("Walk");
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![
        BehaviorEntry::Single(def_toggle("Walk", 100, None)),
        single("Fall", 100),
    ]);

    // rng 0.0: 誤って Walk が残っていれば Walk が選ばれるため、Fall で判別できる
    let mut rng = FakeRng::new(&[0.0]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    assert_eq!(m.anchor(), (500, 500)); // 真の候補（Fall）が選ばれたため再配置しない
                                        // env へ麻スコットの image set 名と行動名が渡る（Java disabledBehaviors.get(imageSet) 相当）
    assert!(env
        .behavior_checks
        .borrow()
        .iter()
        .any(|(set, name)| set == "TestSet" && name == "Walk"));
}

#[test]
fn build_next_behavior_keeps_non_toggleable_even_if_env_reports_disabled() {
    // 非 toggleable 行動は（Java L583-588: isToggleable() && 判定 の短絡）env 返値に
    // 関わらず必ず候補として残る
    let env = MockEnv::new().disable("Fall");
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![single("Walk", 1), single("Fall", 100)]);

    // rng 0.9999: total=101 → Walk(1) を超え Fall。Fall が誤って除外されると Walk のみ
    let mut rng = FakeRng::new(&[0.9999]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    // 非 toggleable では env への問い合わせが短絡して発生しない（rng [0,1) は選択のみに消費）
    assert_eq!(rng.consumed(), 1);
}

#[test]
fn build_next_behavior_repositions_and_falls_when_all_candidates_disabled() {
    // 候補全滅（唯一の toggleable 行動が無効・total == 0）→ 再配置 + Fall
    let env = MockEnv::new().disable("Walk");
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![
        BehaviorEntry::Single(def_toggle("Walk", 100, None)),
        single("Fall", 0),
    ]);

    // work_area=(0,0,1920,1040)・rng 0.5 → (int)(0.5*(1920-2))+0+1 = 960 / top-256
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(None, &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    assert_eq!(m.anchor(), (960, -256));
    assert_eq!(rng.consumed(), 1); // 再配置でのみ消費
}

#[test]
fn build_next_behavior_filters_disabled_toggleable_ref_candidates() {
    // next リスト参照も isEffective && isBehaviorEnabled で絞られる（Java L496）
    let env = MockEnv::new().disable("RefOnly");
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 1, Some(next_list(false, &[("RefOnly", 1)])))),
        BehaviorEntry::Single(def_toggle("RefOnly", 1, None)),
        single("Fall", 0),
    ]);

    // 誤って参照候補が残れば total=1 で rng 0.5 は RefOnly を選ぶため判別できる
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_next_behavior(Some("Walk"), &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    assert_eq!(m.anchor(), (960, -256));
    assert_eq!(rng.consumed(), 1); // 再配置でのみ消費
}

#[test]
fn build_behavior_repositions_and_falls_for_disabled_toggleable() {
    // 既知名・無効 → 再配置（Java L545-548 逐語）+ Fall 返却
    let env = MockEnv::new().disable("Walk");
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![
        BehaviorEntry::Single(def_toggle("Walk", 100, None)),
        single("Fall", 100),
    ]);

    // work_area=(0,0,1920,1040)・rng 0.5 → (int)(0.5*(1920-2))+0+1 = 960 / top-256
    let mut rng = FakeRng::new(&[0.5]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Fall");
    assert_eq!(m.anchor(), (960, -256));
    assert_eq!(rng.consumed(), 1); // 再配置でのみ消費
}

#[test]
fn build_behavior_keeps_non_toggleable_even_if_env_reports_disabled() {
    // 非 toggleable 行動は env 返値に依らず有効（Java L583-588 の短絡）
    let env = MockEnv::new().disable("Walk");
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![single("Walk", 100), single("Fall", 100)]);

    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Walk");
    assert_eq!(m.anchor(), (500, 500)); // 再配置しない
}

#[test]
fn build_behavior_enabled_consumes_no_random_and_keeps_behavior() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![
        BehaviorEntry::Single(def_toggle("Walk", 100, None)),
        single("Fall", 100),
    ]);

    // 有効な toggleable 行動はそのまま構築され、乱数を消費しない
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(runner.name, "Walk");
    assert_eq!(rng.consumed(), 0);
}

#[test]
fn init_transitions_to_next_behavior_when_action_completes_immediately() {
    let env = MockEnv::new();
    let log = new_log();
    // has_next を常に false にする → init 中に次行動へ遷移
    let mut factory = MockFactory::new(&log).with_script(
        "Walk",
        ActionScript {
            has_next_true_times: 0,
            ..Default::default()
        },
    );
    let mut m = mascot_at((500, 500));

    // Walk の next リスト（add=false）→ 候補は ChaseMouse のみ（単独なので乱数値に非依存）
    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 1, Some(next_list(false, &[("ChaseMouse", 1)])))),
        single("ChaseMouse", 1),
        single("Fall", 100),
    ]);

    let mut rng = FakeRng::new(&[0.5]);
    let mut runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    // init 中の遷移（build_next_behavior・単独候補 total=1）で selection random を 1 回消費
    runner
        .init(&mut m, &env, &t, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(rng.consumed(), 1);

    // init 中に action.has_next == false → build_next_behavior(Some("Walk")) で遷移
    assert_eq!(m.behavior_name(), Some("ChaseMouse"));
    assert_eq!(count_init(&log), 2); // Walk と ChaseMouse の両方で init される
    assert_eq!(
        build_names(&log),
        vec!["Walk".to_string(), "ChaseMouse".to_string()]
    );
}

#[test]
fn tick_advances_time_once_per_tick_and_runs_action() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image())); // 画面内 bounds を保証

    let t = table(vec![single("Walk", 100), single("Fall", 100)]);
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.behavior_name(), Some("Walk")); // set_behavior で init 済み
    let inits_after_setup = count_init(&log);
    assert_eq!(inits_after_setup, 1);

    let mut rng = FakeRng::new(&[]);
    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 1);
    assert_eq!(count_next(&log), 1); // runner.next() → action.next() が 1 回
    assert_eq!(count_init(&log), inits_after_setup); // 遷移なし → init 追加なし

    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 2);
    assert_eq!(count_next(&log), 2);

    // paused → is_animating() == false で何もしない（time 不増）
    m.set_paused(true);
    assert!(!m.is_animating());
    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 2);
    assert_eq!(count_next(&log), 2);

    // animating = false でも何もしない
    m.set_paused(false);
    m.set_animating(false);
    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 2);

    // 再開すると進む
    m.set_animating(true);
    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 3);
    assert_eq!(count_next(&log), 3);
}

#[test]
fn tick_without_behavior_does_not_advance_time() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    let t = table(vec![single("Walk", 100)]);
    let mut rng = FakeRng::new(&[]);

    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 0);
    assert_eq!(count_next(&log), 0);
}

#[test]
fn tick_with_eval_error_disposes_but_still_counts_time() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log).with_script(
        "Walk",
        ActionScript {
            next: Plan::Eval,
            ..Default::default()
        },
    );
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));

    let t = table(vec![single("Walk", 100), single("Fall", 100)]);
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();

    let mut rng = FakeRng::new(&[]);
    m.tick(&env, &t, &mut factory, &mut rng);

    // Eval エラー → dispose 相当（remove_pending = true・animating = false）。
    // ただし time++ は try/catch の外側なので実行される
    assert_eq!(m.time(), 1);
    assert!(m.remove_pending());
    assert!(!m.is_animating());

    // 以降の tick は何もしない
    m.tick(&env, &t, &mut factory, &mut rng);
    assert_eq!(m.time(), 1);
    assert_eq!(count_next(&log), 1);
}

// =====================================================================
// Runner::next の分岐（契約 3）
// =====================================================================

#[test]
fn tick_transitions_to_next_behavior_when_action_completes() {
    let env = MockEnv::new();
    let log = new_log();
    // has_next は init 中の 1 回のみ true → tick 開始時には完了扱い
    let mut factory = MockFactory::new(&log).with_script(
        "Walk",
        ActionScript {
            has_next_true_times: 1,
            ..Default::default()
        },
    );
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));

    let t = table(vec![
        BehaviorEntry::Single(def("Walk", 1, Some(next_list(false, &[("ChaseMouse", 1)])))),
        single("ChaseMouse", 1),
        single("Fall", 100),
    ]);
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.behavior_name(), Some("Walk")); // init 中はまだ遷移しない

    let mut rng = FakeRng::new(&[0.9999]);
    m.tick(&env, &t, &mut factory, &mut rng);

    // has_next == false → build_next_behavior → 候補は ChaseMouse のみ
    assert_eq!(m.behavior_name(), Some("ChaseMouse"));
    assert_eq!(count_next(&log), 0); // 完了済みなので action.next() は呼ばれない
}

#[test]
fn tick_repositions_and_falls_when_off_screen() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((5000, 100)); // 画面右外
    m.set_image(Some(on_screen_image())); // bounds.x = 4936 >= screen.right(1920)

    let t = table(vec![single("Walk", 100), single("Fall", 100)]);
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();

    let mut rng = FakeRng::new(&[0.5]);
    m.tick(&env, &t, &mut factory, &mut rng);

    // off-screen → 再配置式の焼き込み + Fall 遷移。
    // anchor=(5000,100) は全モニタ外のため、Java 準拠（MascotEnvironment.java L104-113）では
    // invisibleScreen 0×0 にフォールバックし、x = (int)(0.5 * -2) + 0 + 1 = 0, y = -256。
    // （旧期待値 (960,-256) は #22 でプライマリ固定の work_area を解消する前の stale pin）
    assert_eq!(m.anchor(), (0, -256));
    assert_eq!(m.behavior_name(), Some("Fall"));
    assert_eq!(rng.consumed(), 1); // 再配置でのみ消費（Fall 構築では消費しない）
}

#[test]
fn tick_clears_cursor_when_no_hotspots_are_active() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500)); // 画面内 → 遷移しない
    m.set_image(Some(on_screen_image()));
    m.set_cursor_position(Some((10, 10)));

    let t = table(vec![single("Walk", 100)]);
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();

    let mut rng = FakeRng::new(&[]);
    m.tick(&env, &t, &mut factory, &mut rng);

    // hotspots 空 → hotspot 状態 Inactive → cursor がクリアされる
    assert_eq!(m.cursor_position(), None);
    assert!(!m.is_hotspot_clicked());
    assert_eq!(m.behavior_name(), Some("Walk")); // 遷移しない
    assert_eq!(m.anchor(), (500, 500));
    assert_eq!(rng.consumed(), 0); // 乱数は消費されない
}

#[test]
fn tick_on_lost_ground_clears_cursor_dragging_and_falls() {
    let env = MockEnv::new();
    let log = new_log();
    let mut factory = MockFactory::new(&log).with_script(
        "Walk",
        ActionScript {
            next: Plan::LostGround,
            ..Default::default()
        },
    );
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    m.set_cursor_position(Some((3, 4)));
    m.set_dragging(true);

    let t = table(vec![single("Walk", 100), single("Fall", 100)]);
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();

    let mut rng = FakeRng::new(&[]);
    m.tick(&env, &t, &mut factory, &mut rng);

    // LostGround → cursor/dragging リセット + Fall 遷移（再配置なし・tick は正常完了扱い）
    assert_eq!(m.cursor_position(), None);
    assert!(!m.is_dragging());
    assert_eq!(m.behavior_name(), Some("Fall"));
    assert_eq!(m.anchor(), (500, 500)); // 再配置されない
    assert_eq!(m.time(), 1);
    assert!(!m.remove_pending());
    assert_eq!(rng.consumed(), 0);
}

// =====================================================================
// mouse_pressed / mouse_released（契約 8・9）
// =====================================================================

#[test]
fn mouse_pressed_starts_drag_or_respects_undraggable() {
    let env = MockEnv::new();
    let t = table(vec![
        single("Walk", 100),
        single("NoDrag", 1),
        single("Dragged", 1),
    ]);

    // (a) is_draggable == true → Dragged へ遷移
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    m.mouse_pressed((10, 10), &env, &t, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.behavior_name(), Some("Dragged"));

    // (b) is_draggable == false → 処理済み（遷移なし）
    let log = new_log();
    let mut factory = MockFactory::new(&log).with_script(
        "Walk",
        ActionScript {
            draggable_ok: false,
            ..Default::default()
        },
    );
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    m.mouse_pressed((10, 10), &env, &t, &mut factory, &mut rng)
        .unwrap();
    assert_eq!(m.behavior_name(), Some("Walk"));
    assert_eq!(count_is_draggable(&log), 1);

    // (c) is_draggable の評価エラー → BehaviorError::Eval
    let log = new_log();
    let mut factory = MockFactory::new(&log).with_script(
        "Walk",
        ActionScript {
            draggable_err: true,
            ..Default::default()
        },
    );
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    let result = m.mouse_pressed((10, 10), &env, &t, &mut factory, &mut rng);
    assert!(matches!(result, Err(BehaviorError::Eval(_))));
}

#[test]
fn mouse_released_throws_when_dragging_and_clears_hotspot_cursor() {
    let env = MockEnv::new();
    let t = table(vec![single("Walk", 100), single("Thrown", 1)]);

    // (a) hotspot クリック中 + ドラッグ中 → cursor クリア + Thrown 遷移
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    m.set_cursor_position(Some((5, 5)));
    m.set_dragging(true);
    m.mouse_released(&env, &t, &mut factory, &mut rng).unwrap();
    assert_eq!(m.cursor_position(), None);
    assert!(!m.is_dragging());
    assert_eq!(m.behavior_name(), Some("Thrown"));

    // (b) hotspot クリック中のみ → cursor だけクリア（遷移なし）
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    m.set_cursor_position(Some((5, 5)));
    m.mouse_released(&env, &t, &mut factory, &mut rng).unwrap();
    assert_eq!(m.cursor_position(), None);
    assert_eq!(m.behavior_name(), Some("Walk"));

    // (c) どちらでもない → 何もしない
    let log = new_log();
    let mut factory = MockFactory::new(&log);
    let mut m = mascot_at((500, 500));
    m.set_image(Some(on_screen_image()));
    let mut rng = FakeRng::new(&[]);
    let runner = t
        .build_behavior("Walk", &mut m, &env, &mut factory, &mut rng)
        .unwrap();
    m.set_behavior(Some(runner), &env, &t, &mut factory, &mut rng)
        .unwrap();
    m.mouse_released(&env, &t, &mut factory, &mut rng).unwrap();
    assert_eq!(m.cursor_position(), None);
    assert_eq!(m.behavior_name(), Some("Walk"));
    assert!(!m.is_dragging());
}

// =====================================================================
// apply_pose（契約 13）
// =====================================================================

#[test]
fn apply_pose_moves_anchor_and_sets_image_with_flip_aware_center() {
    let set = image_set_with(&[("shime1.png", 128, 128)]);
    let p = pose("shime1.png", (100, 48), (3, -2), 1);

    // look_right = false → anchor += (dx, dy)、center は pose.anchor のまま
    let mut m = Mascot::new("TestSet", set.clone(), (1000, 500));
    apply_pose(&p, &mut m);
    assert_eq!(m.anchor(), (1003, 498));
    let img = m.image().expect("フレームが存在するので image は Some");
    assert_eq!(img.image_ref, "shime1.png");
    assert_eq!(img.center, (100, 48));
    assert_eq!(img.width, 128);
    assert_eq!(img.height, 128);
    assert_eq!(m.image_anchor(), Some((100, 48)));

    // look_right = true → x のみ符号反転、center.x は width - anchor.x
    let mut m = Mascot::new("TestSet", set, (1000, 500));
    m.set_look_right(true);
    apply_pose(&p, &mut m);
    assert_eq!(m.anchor(), (997, 498));
    assert_eq!(m.image().unwrap().center, (28, 48));
}

#[test]
fn apply_pose_missing_frame_keeps_previous_image_bounds() {
    let set = image_set_with(&[("shime1.png", 128, 128)]);
    let mut m = Mascot::new("TestSet", set, (1000, 500));

    apply_pose(&pose("shime1.png", (64, 64), (5, 5), 1), &mut m);
    assert!(m.image().is_some());
    assert_eq!(m.anchor(), (1005, 505));

    // 存在しないフレーム → image は None。ただし prev は保持され get_bounds は復元される
    apply_pose(&pose("missing.png", (64, 64), (0, 0), 1), &mut m);
    assert_eq!(m.image(), None);
    assert_eq!(m.anchor(), (1005, 505)); // velocity 0 で移動なし
    assert_eq!(
        m.get_bounds(),
        Some(Rect {
            left: 941,
            top: 441,
            right: 1069,
            bottom: 569
        })
    );
}
