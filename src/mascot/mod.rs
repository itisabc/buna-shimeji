//! Mascot コア — Java `Mascot`（.tmp/java-ref/Mascot.java）相当。
//!
//! 構造は設計の最適形（design.md §1.5/§1.6）: Java の singleton・EDT・ReadWriteLock を
//! 排除し、環境・乱数・行動表・ファクトリはメソッド引数で注入する（Manager が所有）。
//! ロジックは Java を仕様として逐語移植する:
//! - tick（L613-656）: `isAnimating()`（L997-999 = animating && !paused）のとき
//!   behavior.next() を 1 回呼び、try/catch の外側で time++
//! - setImage（L835-862）: 同値 no-op / prev 更新 / needs_repaint = true
//! - getBounds（L910-918）: anchor - image.center の矩形。image 無しは直前の非 null
//!   画像から復元
//! - dispose（L713-730）: animating = false + affordances クリア + remove_pending
//!   （Manager への削除反映は次 tick・#8）
//!
//! Phase 1 の意図的な範囲外（doc 開示）:
//! - (C) Hotspot の contains 判定 / isBehaviorEnabled（behavior.rs 注記）— 資産 hotspot
//!   0 件のため placeholder。実装は #7/#9
//! - affordances は資産 XML 未使用（asset-report.md §2）のためフィールド+API 保持のみ
//! - image_anchor() は flip 調整済み center を返す（flip 前値は #8 renderer glue が
//!   flip フラグと併用して復元）
//! - needs_repaint のクリア（Java apply 相当）とウィンドウ描画は #8 renderer glue
//! - 効果音実体・DebugWindow・setPaused のトレイ通知は #9

pub mod animation;
pub mod behavior;

use std::sync::Arc;

use crate::config::script::{EvalContext, Variables};
use crate::render::imageset::ImageSet;
use behavior::{BehaviorError, BehaviorFactory, BehaviorRunner, BehaviorTable};

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

/// マスコットから見たデスクトップ環境の抽象（Java `MascotEnvironment` 相当）。
/// #8 の Environment が実装する。
pub trait EnvironmentView {
    fn work_area(&self) -> Rect;
    fn screen(&self) -> Rect;
    fn multiscreen(&self) -> bool;
    /// 条件式評価用の環境コンテキスト（mascot.environment.* 変数と isOn を
    /// mascot.environment プレフィックス込みで解釈する契約）。
    fn eval_context(&self) -> &dyn EvalContext;
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
/// - それ以外のパス（`mascot.environment.workArea.left` 等を含む）は
///   [`EnvironmentView::eval_context`] へ同一パス文字列で委譲する
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
            _ => self.env.eval_context().number(path),
        }
    }

    fn boolean(&self, path: &str) -> Option<bool> {
        match path {
            "mascot.lookRight" => Some(self.snapshot.look_right),
            _ => self.env.eval_context().boolean(path),
        }
    }

    fn is_on(&self, target: &str, x: f64, y: f64) -> bool {
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
            return;
        };
        if let Err(err) = runner.next(self, env, table, factory, rng) {
            log::error!("次の Behavior を取得できませんでした: {err}");
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
            Some(runner) => behavior::set_behavior_and_init(runner, self, env, table, factory, rng),
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

    pub fn set_anchor(&mut self, anchor: (i32, i32)) {
        self.anchor = anchor;
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
