//! Mascot コア — Java `Mascot`（.tmp/java-ref/Mascot.java）相当。
//!
//! 構造は設計の最適形（design.md §1.5/§1.6）: Java の singleton・EDT・ReadWriteLock を
//! 排除し、環境・乱数・行動表・ファクトリはメソッド引数で注入する（Manager が所有）。
//! ロジックは Java を仕様として逐語移植する:
//! - tick（L613-656）: `isAnimating()`（L997-999 = animating && !paused）かつ
//!   behavior 有りのとき behavior.next() を 1 回呼び time++（L613-625・behavior
//!   無しでは time は進まない）
//! - setImage（L835-862）: 同値 no-op / prev 更新 / needs_repaint = true
//! - getBounds（L910-918）: anchor - image.center の矩形。image 無しは直前の非 null
//!   画像から復元
//! - dispose（L713-730）: animating = false + affordances クリア + remove_pending
//!   （Manager への削除反映は次 tick・#8）
//!
//! Phase 1 の意図的な範囲外（doc 開示）:
//! - (C) Hotspot の contains 判定 / isBehaviorEnabled（behavior.rs 注記）— 資産 hotspot
//!   0 件のため placeholder。実装は #7/#9
//! - affordances は デレマスしめじ v1.9 資産が使用する（Java ActionBase の
//!   `Affordance` 属性放送・ScanMove の探索対象）。broadcast は action 側で実装済み
//! - ScanMove 到達時の自分自身の Behavior 差し替えは
//!   [`Mascot::request_behavior`] に要求を積み、Manager がループ後に反映する
//!   （Java `ScanMove.tick` の `mascot.setBehavior(...)` 即時呼び出しとの
//!   意図的差異・design §1.10 記録）
//! - image_anchor() は flip 調整済み center を返す（flip 前値は #8 renderer glue が
//!   flip フラグと併用して復元）
//! - needs_repaint のクリア（Java apply 相当）とウィンドウ描画は #8 renderer glue
//! - 効果音実体・DebugWindow・setPaused のトレイ通知は #9

pub mod action;
pub mod animation;
pub mod behavior;
pub mod env;
pub mod rng;

use std::sync::Arc;

use crate::config::script::{EvalContext, Variables};
use crate::render::imageset::ImageSet;
use behavior::{BehaviorError, BehaviorFactory, BehaviorRunner, BehaviorTable};
use env::{
    is_env_path, resolve_env_is_on, resolve_env_path, AreaSlot, AreaState, CursorState, EnvValue,
};

/// 矩形（Java `Area` / `Rectangle` 相当の最小セット。right/bottom は含まない）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// 幅（Java Area.getWidth() = right - left 相当）。
    pub fn width(&self) -> i32 {
        self.right - self.left
    }
}

/// ScanMove（アフォーダンス探索）用の、あるマスコット 1 体分の観測値。
/// Manager が個体 tick の後にスナップショットを更新するため、後続の個体は
/// 同 tick の最新状態を見る（Java の `Manager.getMascotWithAffordance` が live に
/// リストを走査する挙動の観察等価）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffordanceScanEntry {
    /// Manager の `mascots` 内 index（[`AffordanceReaction`] の宛先指定に使う）。
    pub index: usize,
    /// 現在の anchor（Java `Mascot.getAnchor()` 相当）。
    pub anchor: (i32, i32),
    /// 放送中の affordance（Java `Mascot.getAffordances()` 相当）。
    pub affordances: Vec<String>,
}

/// ScanMove が到達時に要求する反応（自分と相手の Behavior 差し替え・向き反転）。
/// Java は action の中から `mascot.setBehavior(...)` / `targetMascot.setBehavior(...)`
/// を直接呼ぶが、Rust の action は他個体へ触れない（`&dyn EnvironmentView` + 自 mascot
/// の `&mut` のみ）ため、要求を積んで Manager がループ後に Java と同じ順序
/// （自分 → 相手 → 向き反転）で適用する（pin の前後で要求を溜める既存パターンと同型）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AffordanceArrival {
    /// 自分に設定する Behavior 名（Java `Behaviour` 属性）。
    pub behavior: String,
    /// 相手の index（[`AffordanceScanEntry::index`]）。相手不在は None。
    pub target_index: Option<usize>,
    /// 相手に設定する Behavior 名（Java `TargetBehaviour` 属性）。
    pub target_behavior: String,
    /// 相手の向きを自分と逆にするか（Java `TargetLook` 属性）。
    pub flip_look: bool,
}

/// マスコットから見たデスクトップ環境の抽象（Java `MascotEnvironment` / `Environment`
/// 相当）。#8 の Environment が実装する。
///
/// 既存 4 メソッド（work_area / screen / multiscreen / eval_context）のシグネチャは
/// #6 で確定済み。#7a（design §1.7(d)）で**同一 trait へ 10 メソッドを追加**した
/// （Action trait が `&dyn EnvironmentView` 固定のため別 trait 化は downcast 必須と
/// なり契約違反）。追加メソッドは OS 依存のため app 側（#8）が実装する。既存の
/// 実装者（テストダブル等）を compile 可能に保つため default 実装は `todo!` であり、
/// 呼ばれると panic する。純関数側（env.rs）はこれらの primitive からスナップショットを
/// 取って評価する。
///
/// #32 で追加した ScanMove 用 2 メソッド（[`EnvironmentView::affordance_scan`] /
/// [`EnvironmentView::queue_affordance_reaction`]）は default 実装を持ち、未実装の
/// テストダブルでもスキャン相手なし / 要求破棄として振る舞う（既存メソッドの
/// `todo!` 方針とは異なる: 呼ばれても panic しない方が安全なため）。
// default 実装は todo! スタブ（引数を消費しない）のため、trait 配下のみ
// 未使用引数警告を抑止する（実装者側の impl には影響しない）。
#[allow(unused_variables)]
pub trait EnvironmentView {
    fn work_area(&self) -> Rect;
    fn screen(&self) -> Rect;
    fn multiscreen(&self) -> bool;
    /// 条件式評価用の環境コンテキスト。
    ///
    /// **platform primitives のみ返せばよい**: `mascot.environment.*` パスは
    /// [`MascotContext`] が `env` モジュールの純関数（design §1.7(f)）で自前解決する
    /// ため、実装側は非 env パス（mascot.custom.* 等のカスタム変数）にのみ応答すればよい。
    fn eval_context(&self) -> &dyn EvalContext;

    /// 画面全体（全モニタ矩形の union・Java `Environment.getScreen()` /
    /// `AbstractEnvironment.screen` 相当）。
    fn screen_area(&self) -> AreaState {
        todo!("app impl at #8")
    }

    /// 全モニタ矩形の列挙（monitor index 順・Java `Environment.getScreens()` /
    /// `AbstractEnvironment.complexScreen` 相当）。
    /// [`AreaSlot::WorkArea(i)`] / [`AreaSlot::Screen(i)`] の `i` は同一モニタ順序を
    /// 共有する（work_area_state の契約参照）。
    fn screens(&self) -> Vec<AreaState> {
        todo!("app impl at #8")
    }

    /// 点を含む work area のスロット（Java `Environment.getWorkAreaAt(int, int)` /
    /// `AbstractEnvironment.getWorkAreaAt` L186-193 相当）。見つからなければ
    /// [`AreaSlot::Invisible`] を返す。
    fn work_area_at(&self, x: i32, y: i32) -> AreaSlot {
        todo!("app impl at #8")
    }

    /// スロットの現在値（Java `Area` への live 参照値のスナップショット相当）。
    /// [`AreaSlot::Invisible`] は invisibleScreen（全 0・visible=false・
    /// AbstractEnvironment L93-98）を返す。スロット index は `screens()` と同じ
    /// monitor index 順で 1:1 対応する。
    fn work_area_state(&self, slot: AreaSlot) -> AreaState {
        todo!("app impl at #8")
    }

    /// アクティブウィンドウの矩形（Java `Environment.getActiveWindow()` 相当・
    /// gating 前の生値。gating は env.rs `active_ie_effective` が行う）。
    fn active_window(&self) -> AreaState {
        todo!("app impl at #8")
    }

    /// アクティブウィンドウ ID。無ければ 0（Java `Environment.getActiveWindowId()` 相当）。
    fn active_window_id(&self) -> i64 {
        todo!("app impl at #8")
    }

    /// アクティブウィンドウの左上を (x, y) へ移動する
    /// （Java `Environment.moveActiveWindow(int, int)` 相当・内部で SetWindowPos）。
    fn move_active_window(&self, x: i32, y: i32) {
        todo!("app impl at #8")
    }

    /// カーソル位置と移動量（Java `Environment.getCursor()` / `Location` 相当）。
    fn cursor(&self) -> CursorState {
        todo!("app impl at #8")
    }

    /// 画像スケーリング倍率（settings.scale の供給経路）。
    fn scaling(&self) -> f64 {
        todo!("app impl at #8")
    }

    /// 投げ（ThrowIE）許可設定（settings.throwing の供給経路）。
    fn throwing_allowed(&self) -> bool {
        todo!("app impl at #8")
    }

    /// 増殖許可設定（settings.breeding の供給経路・design §1.8(f)）。
    fn breeding_allowed(&self) -> bool {
        todo!("app impl at #8")
    }

    /// Transients（Breed の BornTransient 経路）許可設定
    /// （settings.transients の供給経路）。
    fn transients_enabled(&self) -> bool {
        todo!("app impl at #8")
    }

    /// 変身許可設定（settings.transformation の供給経路。Transform は stub のため
    /// Phase 1 の実行経路では未使用・#9 の Toggleable 供給と合わせる）。
    fn transformation_allowed(&self) -> bool {
        todo!("app impl at #8")
    }

    /// Breed 用の追加マスコット要求をキューへ積む
    /// （Java は manager.add() 即時。Rust は次 tick 一括反映 = AGENTS.md §5-6 追加/
    /// 削除キューイング踏襲 → 意図的差異・design §1.8(f)。キューの所有と反映は
    /// Manager（#8）。anchor は出生計算済みの値、look_right は親の向き、
    /// behavior_name は BornBehaviour 属性の評価結果（#8 で 4 引数化・省略時 ""））。
    fn queue_spawn(
        &self,
        image_set_name: &str,
        anchor: (i32, i32),
        look_right: bool,
        behavior_name: &str,
    ) {
        todo!("app impl at #8")
    }

    /// createMascot 経路の追加マスコット要求をキューへ積む（Main.createMascot
    /// L480-505 相当・#9b）。behavior_name は None = drain 時に
    /// buildNextBehavior(None) で構築することを示す（[`Manager::request_spawn`]
    /// `manager::Manager::request_spawn` 経路）。
    fn queue_spawn_next(&self, image_set_name: &str, anchor: (i32, i32), look_right: bool) {
        todo!("app impl at #9b")
    }

    /// 画面外の窓を作業領域へ戻す（Java `WindowsEnvironment.restoreWindows`
    /// L292-347 相当・tray RestoreWindows の供給経路・#9b）。
    fn restore_windows(&self) {
        todo!("app impl at #9b")
    }

    /// Allowed Behaviours トグルの無効判定（Java `Configuration.isBehaviorEnabled`
    /// L583-588 の `disabledBehaviors.get(set).contains(name)` 部相当・#9）。
    ///
    /// 意味論（テストダブル・behavior.rs 呼び出し側と一致）:
    /// **true = その (image_set, behavior) が Allowed Behaviours 無効リストに
    /// 含まれる（= トグル OFF・無効）**。behavior.rs の `is_behavior_enabled` は
    /// `!toggleable || !env.behavior_disabled(set, name)` で Java 等価式
    /// （`!toggleable || !disabled.contains(name)`・短絡評価）を組み立てる。
    ///
    /// 既定実装は false = 無効リスト空 = Java 既定（全 Behavior 有効）。
    /// app 実装（#9b）は settings の無効リスト含有判定で差し替える。
    fn behavior_disabled(&self, image_set: &str, behavior_name: &str) -> bool {
        false
    }

    /// ScanMove 用の読み取り専用スナップショット（Manager が個体 tick の後に更新）。
    /// 既定は空 = スキャン相手なし（Java の「該当 affordance を持つ個体が居ない」
    /// と同じ扱いになる）。実装は clone を返す（小型・呼び出しは 1 tick 数回）。
    fn affordance_scan(&self) -> Vec<AffordanceScanEntry> {
        Vec::new()
    }
}

/// Java `Math.random()` 相当の [0,1) 一様乱数の抽象。
/// グローバル可変状態を避けるため呼び出し側（#8 Manager / テスト）から注入する
/// （design.md §1.5）。Java の Math.random() 呼び出し 1 回 = unit() 1 回。
pub trait Rng {
    fn unit(&mut self) -> f64;
}

/// ホットスポット（Java `animation.Hotspot` の Phase 1 最小セット）。
/// 資産に hotspot 定義が 0 件のため contains 判定は未実装（behavior.rs の (C) 注記）。
#[derive(Debug, Clone)]
pub struct Hotspot {
    /// クリック時に遷移する Behavior 名。空文字列は Java の null（遷移なし）相当。
    pub behaviour: String,
}

/// 現在表示中の画像の状態（Java `MascotImage` の描画に必要な部分相当）。
/// `center` は flip（look_right）調整済みの画像アンカー。
#[derive(Debug, Clone, PartialEq)]
pub struct ImageState {
    pub image_ref: String,
    pub center: (i32, i32),
    pub width: u32,
    pub height: u32,
}

/// 条件式評価のために mascot 変数を固定したスナップショット。
/// buildNextBehavior の条件評価はこのスナップショットに対して行われるため、
/// 評価結果が同一 tick 内の mascot 状態の途中変化に依存しない。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EvalSnapshot {
    pub anchor: (i32, i32),
    pub look_right: bool,
    pub total_count: i32,
}

/// [`EvalSnapshot`] + 環境コンテキストの合成（Java buildNextBehavior の
/// `new VariableMap()` + `context.put("mascot", mascot)` 相当）。
///
/// - `mascot.anchor.x` / `mascot.anchor.y` / `mascot.totalCount` / `mascot.lookRight`
///   はスナップショットから返す
/// - `mascot.environment.*`（値 18 パス + isOn ターゲット 11 パス = 29 パス）は
///   `env` モジュールの純関数で**自前解決**する（design §1.7(f)。snapshot.anchor /
///   look_right + env primitives。eval_context の実装が env パスを知らなくてもよい）
/// - それ以外のパス（`mascot.custom.*` 等のカスタム変数）は
///   [`EnvironmentView::eval_context`] へ同一パス文字列で委譲する
///   （#7a で env パスが委譲対象から除外された）
pub struct MascotContext<'a> {
    pub snapshot: &'a EvalSnapshot,
    pub env: &'a dyn EnvironmentView,
}

impl EvalContext for MascotContext<'_> {
    fn number(&self, path: &str) -> Option<f64> {
        match path {
            "mascot.anchor.x" => Some(f64::from(self.snapshot.anchor.0)),
            "mascot.anchor.y" => Some(f64::from(self.snapshot.anchor.1)),
            "mascot.totalCount" => Some(f64::from(self.snapshot.total_count)),
            _ if is_env_path(path) => {
                match resolve_env_path(self.env, self.snapshot.anchor, path) {
                    Some(EnvValue::Number(n)) => Some(n),
                    _ => None,
                }
            }
            _ => self.env.eval_context().number(path),
        }
    }

    fn boolean(&self, path: &str) -> Option<bool> {
        match path {
            "mascot.lookRight" => Some(self.snapshot.look_right),
            _ if is_env_path(path) => {
                match resolve_env_path(self.env, self.snapshot.anchor, path) {
                    Some(EnvValue::Boolean(b)) => Some(b),
                    _ => None,
                }
            }
            _ => self.env.eval_context().boolean(path),
        }
    }

    fn is_on(&self, target: &str, x: f64, y: f64) -> bool {
        if is_env_path(target) {
            return resolve_env_is_on(self.env, self.snapshot.look_right, target, x, y);
        }
        self.env.eval_context().is_on(target, x, y)
    }
}

/// マスコット 1 体（Java `Mascot` 相当）。
/// behavior の実行に必要な環境・行動表・ファクトリ・乱数はメソッド引数で受け取り、
/// Mascot は保持しない（design.md §1.5: 所有は Manager）。
pub struct Mascot {
    image_set_name: String,
    /// 不変の画像セット（design.md §1.5: 不変データのみ Arc 共有）。
    /// animation.rs の apply_pose からフレームを引くため crate 内公開。
    pub(crate) image_set: Arc<ImageSet>,
    /// 実行中の Behavior（Java `Mascot.behavior` 相当）。
    /// 遷移の take/put-back を behavior.rs からも行うため crate 内公開。
    pub(crate) behavior: Option<BehaviorRunner>,
    anchor: (i32, i32),
    look_right: bool,
    time: i32,
    animating: bool,
    paused: bool,
    dragging: bool,
    needs_repaint: bool,
    total_count: i32,
    variables: Variables,
    image: Option<ImageState>,
    prev_image: Option<ImageState>,
    cursor: Option<(i32, i32)>,
    sound: Option<String>,
    affordances: Vec<String>,
    hotspots: Vec<Hotspot>,
    remove_pending: bool,
    /// #30 item 5: pin 窓の holder 特定用ミラー。真実は `Environment.pinned` で、
    /// Manager が必ず同期する（非保持・非 pin は None）。
    pinned_window: Option<i64>,
    /// ScanMove 到達時に要求された反応（#32・Manager がループ後に適用）。
    /// action は自 Behavior を構築できない（table / factory を持たない）ため要求を積む
    /// （Java `ScanMove.tick` L122-137 の setBehavior 呼び出し相当・意図的差異）。
    affordance_arrival: Option<AffordanceArrival>,
}

impl Mascot {
    /// Java コンストラクタ + フィールド初期化相当（Mascot.java L102-263）。
    /// ウィンドウ生成は #8 renderer glue が担当するためここには無い。
    pub fn new(
        image_set_name: impl Into<String>,
        image_set: Arc<ImageSet>,
        anchor: (i32, i32),
    ) -> Self {
        Mascot {
            image_set_name: image_set_name.into(),
            image_set,
            behavior: None,
            anchor,
            look_right: false,
            time: 0,
            animating: true,
            paused: false,
            dragging: false,
            needs_repaint: true,
            total_count: 1,
            variables: Variables::new(),
            image: None,
            prev_image: None,
            cursor: None,
            sound: None,
            affordances: Vec::new(),
            hotspots: Vec::new(),
            remove_pending: false,
            pinned_window: None,
            affordance_arrival: None,
        }
    }

    /// 1 tick 進める（Mascot.java L613-656 逐語）。
    /// animating でない（= paused または停止済み）か behavior が無ければ何もしない。
    /// behavior.next() が Err のときはログ + dispose（Java はエラー表示 + dispose）。
    /// time++ は try/catch の外側なのでエラー時も実行される。
    pub fn tick(
        &mut self,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) {
        if !self.is_animating() {
            return;
        }
        // take/put-back: next 内の遷移は self.behavior を直接差し替えるため、
        // 遷移済みなら take した古い runner は破棄する。
        let Some(mut runner) = self.behavior.take() else {
            // Java L613-625: time++ は `behavior != null` の内側（behavior 無しでは増えない）
            return;
        };
        if let Err(err) = runner.next(self, env, table, factory, rng) {
            log::error!("failed to get next Behavior: {err}");
            self.dispose();
        }
        self.time += 1;
        if self.behavior.is_none() {
            self.behavior = Some(runner);
        }
    }

    /// マウスボタン押下（Mascot.java L430-447 の behavior 委譲部分逐語）。
    /// ポップアップ（右クリック）判定と左ボタン判定は呼び出し側（#8/#9）の責務。
    pub fn mouse_pressed(
        &mut self,
        point: (i32, i32),
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        if self.paused {
            return Ok(());
        }
        let Some(mut runner) = self.behavior.take() else {
            return Ok(());
        };
        let result = runner.mouse_pressed(self, point, env, table, factory, rng);
        if self.behavior.is_none() {
            self.behavior = Some(runner);
        }
        result
    }

    /// マウスボタン解放（Mascot.java L455-471 の behavior 委譲部分逐語）。
    pub fn mouse_released(
        &mut self,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        if self.paused {
            return Ok(());
        }
        let Some(mut runner) = self.behavior.take() else {
            return Ok(());
        };
        let result = runner.mouse_released(self, env, table, factory, rng);
        if self.behavior.is_none() {
            self.behavior = Some(runner);
        }
        result
    }

    /// Behavior を設定して初期化する（Java setBehavior L962-967 逐語）。
    /// Java は代入→init の順で既存 behavior を**置き換える**ため、こちらも先に
    /// 旧 runner を破棄してから init する（#9d: Reload の同名再構築 / 再選択が
    /// 既存 behavior 持ちの mascot に対して新 runner を反映するために必須）。
    /// init 中に遷移が起きた場合は遷移先が採用される（Java も setBehavior が
    /// 再帰的に呼ばれるため同じ構造・design.md 補足 9 参照）。
    pub fn set_behavior(
        &mut self,
        behavior: Option<BehaviorRunner>,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        match behavior {
            Some(runner) => {
                // Java L963: this.behavior = behavior（旧 runner は置き換え = 破棄）。
                // 破棄してから init することで、init 中の遷移がなければ新 runner が
                // set_behavior_and_init 内で代入される（旧 runner を保持したままでは
                // 「none なら代入」の guard により新 runner が捨てられていた）。
                self.behavior = None;
                behavior::set_behavior_and_init(runner, self, env, table, factory, rng)
            }
            None => {
                self.behavior = None;
                Ok(())
            }
        }
    }

    /// 実行中の Behavior 名（Java getBehavior の name 部分相当）。
    pub fn behavior_name(&self) -> Option<&str> {
        self.behavior.as_ref().map(|runner| runner.name.as_str())
    }

    /// アンカー（Java getAnchor/setAnchor L804-816）。
    pub fn anchor(&self) -> (i32, i32) {
        self.anchor
    }

    /// アンカー更新（Java setAnchor + `Mascot.apply` L661-690 の位置反映）。
    ///
    /// Java `apply` は 2 層構造: 位置（bounds）は needsRepaint と無関係に
    /// 「窓 bounds と mascot bounds に差分があれば setBounds する」（L678-682）を
    /// 毎 tick 行い、needsRepaint は画像の再描画（L683-696）のみに使う。Rust 版は
    /// 窓位置反映も needs_repaint 経由で行うため、anchor 変化時に needs_repaint
    /// を立てる（画像固定の移動フレームで窓移動がスキップされる実機バグの修正・
    /// 「bounds 差分 → setBounds」の観察等価）。
    /// 同値呼び出しは Java の bounds 同値スキップ（差分無しで setBounds しない）の
    /// 観察等価として needs_repaint を一切変えない（立てない・既に立った要求は
    /// 消さない）。
    pub fn set_anchor(&mut self, anchor: (i32, i32)) {
        if self.anchor != anchor {
            self.anchor = anchor;
            self.needs_repaint = true;
        }
    }

    pub fn look_right(&self) -> bool {
        self.look_right
    }

    /// Java setLookRight L886-887。
    pub fn set_look_right(&mut self, look_right: bool) {
        self.look_right = look_right;
    }

    /// 生成からの tick 数（Java getTime L941-943）。
    pub fn time(&self) -> i32 {
        self.time
    }

    /// Java isAnimating L997-999。
    pub fn is_animating(&self) -> bool {
        self.animating && !self.paused
    }

    pub fn set_animating(&mut self, animating: bool) {
        self.animating = animating;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Java setPaused L1246-1253 の paused 更新部分（トレイ通知は #9）。
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
    }

    pub fn is_dragging(&self) -> bool {
        self.dragging
    }

    pub fn set_dragging(&mut self, dragging: bool) {
        self.dragging = dragging;
    }

    pub fn cursor_position(&self) -> Option<(i32, i32)> {
        self.cursor
    }

    /// #30 item 5: このマスコットが保持中の pin 窓 id（holder 特定用ミラー）。
    /// 真実は `Environment.pinned` で、Manager が同期する。
    pub fn pinned_window(&self) -> Option<i64> {
        self.pinned_window
    }

    /// #30 item 5: pin 窓ミラーを設定する（Manager 専用・holder 特定用）。
    pub fn set_pinned_window(&mut self, id: Option<i64>) {
        self.pinned_window = id;
    }

    /// Java setCursorPosition L1320-1332（カーソル描画更新は renderer glue）。
    pub fn set_cursor_position(&mut self, cursor: Option<(i32, i32)>) {
        self.cursor = cursor;
    }

    pub fn image(&self) -> Option<&ImageState> {
        self.image.as_ref()
    }

    /// 現在画像のアンカー（flip 調整済み center）。Java は prevImageAnchor を使うが
    /// ここでは現在画像の center を返す。flip 前値は #8 renderer glue が
    /// flip フラグと併用して復元する。
    pub fn image_anchor(&self) -> Option<(i32, i32)> {
        self.image.as_ref().map(|state| state.center)
    }

    /// Java setImage L835-862 逐語: 同値（ImageState PartialEq）は no-op、
    /// 異値は prev 更新（None のときは prev 保持）+ needs_repaint = true。
    pub fn set_image(&mut self, image: Option<ImageState>) {
        if self.image == image {
            return;
        }
        if let Some(new_image) = &image {
            self.prev_image = Some(new_image.clone());
        }
        self.image = image;
        self.needs_repaint = true;
    }

    /// 現在のバウンディング矩形（Java getBounds L910-918 逐語）。
    /// image が None のときは直前の非 null 画像（prev）から復元する。
    /// どちらも無い場合は None（Java は 0 サイズ矩形を返すが、描画対象が無いことを
    /// 伝えるため None にする・テスト契約）。
    pub fn get_bounds(&self) -> Option<Rect> {
        let state = self.image.as_ref().or(self.prev_image.as_ref())?;
        let left = self.anchor.0 - state.center.0;
        let top = self.anchor.1 - state.center.1;
        Some(Rect {
            left,
            top,
            right: left + state.width as i32,
            bottom: top + state.height as i32,
        })
    }

    /// 再描画要否（Java needsRepaint）。クリア（apply 相当）は #8 renderer glue。
    pub fn needs_repaint(&self) -> bool {
        self.needs_repaint
    }

    /// 再描画要否フラグをクリアする（Java `Mascot.apply` L693-696 の
    /// `needsRepaint = false` 相当・Manager.apply_all glue から呼ばれる・#8）。
    pub fn clear_needs_repaint(&mut self) {
        self.needs_repaint = false;
    }

    /// 再描画要否フラグを設定する（Reload 後の全マスコット強制再描画用・#10b-2c）。
    /// [`Mascot::rebind_image_set`] は needs_repaint を立てないため、Reload 成功後に
    /// wiring（main）が全マスコットへ再設定する（design.md §1.10 (c)「Reload 後の
    /// 再描画は wiring が MascotView::reset() で全ビュー再描画」の mascot 側補完・
    /// 新資産で同一 pose → set_image 同値 no-op の場合 needs_repaint が立たない
    /// 経路の遮断用）。
    pub fn set_needs_repaint(&mut self, needs_repaint: bool) {
        self.needs_repaint = needs_repaint;
    }

    /// リソース解放 + Manager からの削除依頼（Java dispose L713-730 のうち
    /// ウィンドウ破棄を除く部分）。削除反映は次 tick（remove_pending・#8）。
    pub fn dispose(&mut self) {
        self.animating = false;
        // affordances をクリアしてインタラクションに参加しないようにする（Java コメント踏襲）
        self.affordances.clear();
        self.remove_pending = true;
    }

    pub fn remove_pending(&self) -> bool {
        self.remove_pending
    }

    pub fn affordances(&self) -> &[String] {
        &self.affordances
    }

    /// affordances を丸ごと設定する（design §1.8(b) の next() 毎 affordances 更新の
    /// 検証用 setter。実運用の更新は action がクリア / 追加で行う）。
    pub fn set_affordances(&mut self, affordances: Vec<String>) {
        self.affordances = affordances;
    }

    /// affordances を全消去する（Java ActionBase.next L108-110 相当）。
    pub(crate) fn clear_affordances(&mut self) {
        self.affordances.clear();
    }

    /// affordances に 1 件追加する（Java ActionBase.next L112 相当）。
    pub(crate) fn add_affordance(&mut self, affordance: String) {
        self.affordances.push(affordance);
    }

    /// ScanMove 到達時の反応を要求する（#32）。action は自 Behavior を構築できないため
    /// 要求を積み、Manager がループ後に Java と同じ順序（自分 → 相手 → 向き反転）で
    /// 適用する。1 tick に 1 回しか到達しないため後勝ちで実質 1 件。
    pub(crate) fn request_affordance_arrival(&mut self, arrival: AffordanceArrival) {
        self.affordance_arrival = Some(arrival);
    }

    /// 溜まった到達時要求を取り出す（Manager がループ後に適用・#32）。
    pub fn take_affordance_arrival(&mut self) -> Option<AffordanceArrival> {
        self.affordance_arrival.take()
    }

    /// スクリプト用カスタム変数マップ（Java getVariables L1339-1344）。
    /// 現経路では未使用（Phase 2 の agent 用フック）。
    pub fn variables(&self) -> &Variables {
        &self.variables
    }

    pub fn set_total_count(&mut self, total_count: i32) {
        self.total_count = total_count;
    }

    pub fn image_set_name(&self) -> &str {
        &self.image_set_name
    }

    /// Reload（#9d）用の画像セット付け替え。`image_set_name` と `image_set`
    /// （Arc）の **2 フィールド差し替えのみ** を行う。
    /// 実行中 behavior（runner）/ anchor / look_right / time / paused / dragging /
    /// needs_repaint は一切変更しない（同名 behavior の再構築・再選択・
    /// 再描画要求は呼び出し側（[`crate::app::manager::Manager::reload`]）の責務。
    /// design.md §2「Reload 時に既存 ImageSet 参照」行の新系付け替え相当）。
    pub fn rebind_image_set(
        &mut self,
        image_set_name: impl Into<String>,
        image_set: Arc<ImageSet>,
    ) {
        self.image_set_name = image_set_name.into();
        self.image_set = image_set;
    }

    /// 保持中の画像セットへの参照（Arc の deref・Reload (#9d) の差し替え観測点）。
    pub fn image_set(&self) -> &ImageSet {
        &self.image_set
    }

    /// 保持中の画像セットの [`Arc`] clone（不変データ共有・design.md §1.5）。
    /// bin / glue 側（描画・Reload 系）から参照を持つための取得点。
    pub fn image_set_arc(&self) -> Arc<ImageSet> {
        Arc::clone(&self.image_set)
    }

    /// 保持中の画像セットの解決済み scale（Java `Mascot.getScaling()` /
    /// `ImagePairs.getScaling()` 相当）。action の init は env ではなくこの値を
    /// 参照する（per-set scale を行動・物理へ反映・Reload 後の rebind にも追随）。
    pub fn scale(&self) -> f64 {
        self.image_set.scale
    }

    pub fn sound(&self) -> Option<&str> {
        self.sound.as_deref()
    }

    pub fn set_sound(&mut self, sound: Option<String>) {
        self.sound = sound;
    }

    pub fn hotspots(&self) -> &[Hotspot] {
        &self.hotspots
    }

    pub fn set_hotspots(&mut self, hotspots: Vec<Hotspot>) {
        self.hotspots = hotspots;
    }

    /// ホットスポット クリック中か（Java isHotspotClicked L1296 逐語）。
    pub fn is_hotspot_clicked(&self) -> bool {
        self.cursor.is_some()
    }

    /// 条件式評価用のスナップショット。buildNextBehavior の条件評価は
    /// この値に固定される（呼び出し時点の mascot 状態）。
    pub fn eval_snapshot(&self) -> EvalSnapshot {
        EvalSnapshot {
            anchor: self.anchor,
            look_right: self.look_right,
            total_count: self.total_count,
        }
    }
}
