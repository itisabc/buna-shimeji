//! Behavior 実行器 — Java `behavior.UserBehavior` + `config.Configuration` の
//! buildNextBehavior / buildBehavior 相当。
//!
//! 逐語移植の正本:
//! - UserBehavior.java: init L122-143 / next L146-232 / mousePressed L242-288 /
//!   mouseReleased L298-319
//! - Configuration.java: buildNextBehavior L459-524 / buildBehavior(name, mascot)
//!   L540-555 / buildBehavior(name) L566-573 / isBehaviorEnabled L583-604
//! - BehaviorBuilder.java: isEffective L374-390 / isNextAdditive L206-230+L444-446
//! - BehaviorRef.java: isEffective L188-204
//!
//! rng 注入の全面徹底: Java の `Math.random()` 呼び出し 1 回 = 注入 rng の
//! [`Rng::unit`] 1 回。隠れ既定乱数は作らない（頻度選択 1 回 + 再配置 1 回のみ）。
//! #7b より Action trait の全メソッドが rng を受け取り、BehaviorRunner は保持
//! 純度のある rng をそのまま渡す（design §1.8(e)）。
//!
//! Phase 1 の意図的な範囲外（doc 開示）:
//! - (B) 参照存在検証（Java validate()）は行わない。存在しない名前の構築は
//!   [`BehaviorError::UnknownBehavior`] として構築時にエラーになる
//! - (C) Toggleable（Allowed Behaviours トグル）は #9 で実装済み。無効判定は
//!   [`EnvironmentView::behavior_disabled`]（true = 無効リストに含まれる）へ委譲し、
//!   app 実装は #9b
//! - (C) Hotspot の contains 判定は資産 hotspot 0 件のため placeholder（常に一致）。
//!   hotspot 経路の isBehaviorEnabled は [`BehaviorTable::build_behavior`] 経由で適用済み
//! - isHidden フィルタは buildNextBehavior には存在しない（Java 正本確認済み）

use super::env::resolve_work_area;
use super::{EnvironmentView, Mascot, MascotContext, Rng};
use crate::config::script::{EvalContext, EvalError, EvalValue, Variable, Variables};
use crate::config::{BehaviorEntry, BehaviorRef, BehaviorsConfig, NextBehaviorList, SequenceChild};

/// Java UserBehavior.BEHAVIORNAME_FALL 相当。
const BEHAVIORNAME_FALL: &str = "Fall";
/// Java UserBehavior.BEHAVIORNAME_DRAGGED 相当。
const BEHAVIORNAME_DRAGGED: &str = "Dragged";
/// Java UserBehavior.BEHAVIORNAME_THROWN 相当。
const BEHAVIORNAME_THROWN: &str = "Thrown";

/// [`Action`] の実行エラー（Java `VariableException` / `LostGroundException` 相当）。
#[derive(Debug, thiserror::Error)]
pub enum ActionError {
    #[error("式評価エラー: {0}")]
    Eval(EvalError),
    #[error("地面を失いました（LostGround）")]
    LostGround,
}

impl From<crate::config::script::EvalError> for ActionError {
    fn from(e: crate::config::script::EvalError) -> Self {
        ActionError::Eval(e)
    }
}

/// Behavior 実行のエラー（Java `BehaviorExecutionException` /
/// `BehaviorInstantiationException` 相当）。
#[derive(Debug, thiserror::Error)]
pub enum BehaviorError {
    #[error("式評価エラー: {0}")]
    Eval(EvalError),
    #[error("存在しない Behavior: {0}")]
    UnknownBehavior(String),
}

/// ホットスポットの走査状態（Java UserBehavior.HotspotState 相当）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HotspotState {
    /// クリック中の hotspot 無し。
    Inactive,
    /// クリック中だが Behavior 未設定。
    ActiveNull,
    /// クリック中で Behavior 設定済み（遷移済み）。
    Active,
}

/// 短期アクション（Java `action.Action` インターフェイス相当）。
/// 実装は #7（ActionKind enum + match）が提供する。
///
/// rng 注入（design §1.8(e)）: Java `Math.random()` 呼び出し 1 回 =
/// [`Rng::unit`] 1 回。Dragged（抵抗延長）と Regist（終了時の向き択一）が消費し、
/// 短絡評価のため条件成立時のみ消費される。BehaviorRunner は保持している
/// rng をそのまま action へ渡す。
pub trait Action {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError>;
    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError>;
    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError>;
    /// ドラッグ可否（Java ActionBase.isDraggable 相当）。
    fn is_draggable(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        Ok(true)
    }
}

/// Action を構築するファクトリ（Java Configuration.buildAction の該当部分相当）。
/// #7/#8 が actions.xml から実装を組み立てる。
pub trait BehaviorFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError>;

    /// 構築物へ適用する per-set scale を設定する（既定は no-op）。
    /// [`BehaviorTable::build_behavior_direct`] が構築前にマスコットの
    /// [`Mascot::scale`] を渡す。XML 資産駆動の実装がこれを構築時
    /// [`scale_pose`](crate::render::imageset::scale_pose) に反映する。
    fn set_scale(&mut self, _scale: f64) {}
}

/// next() 内部の流れ制御（Java next の catch 節の対応を明示するための内部表現）。
enum NextFlow {
    /// VariableException 相当（catch して BehaviorError::Eval へ）。
    Eval(EvalError),
    /// LostGroundException 相当（catch して cursor/dragging リセット + Fall）。
    LostGround,
    /// BehaviorInstantiationException 相当（catch されず tick まで伝播・dispose 対象）。
    Fatal(BehaviorError),
}

impl From<ActionError> for NextFlow {
    fn from(err: ActionError) -> Self {
        match err {
            ActionError::Eval(e) => NextFlow::Eval(e),
            ActionError::LostGround => NextFlow::LostGround,
        }
    }
}

/// ActionError → BehaviorError。LostGround は BehaviorError に相当物が無いため
/// Eval に寄せる（Java では非検査例外として呼び出し側へ伝播する経路・Phase 1 の
/// Action 実装では到達しない）。
fn action_error_to_behavior(err: ActionError) -> BehaviorError {
    match err {
        ActionError::Eval(e) => BehaviorError::Eval(e),
        ActionError::LostGround => BehaviorError::Eval(EvalError {
            expr: "(LostGround)".to_string(),
            message: "LostGround は BehaviorError で表現できないため Eval として伝播します"
                .to_string(),
        }),
    }
}

/// Behavior を代入して初期化する（Java Mascot.setBehavior L962-967 相当）。
/// init 中に遷移が起きた場合は遷移先が既に mascot.behavior に入っているため
/// 元の runner は破棄される。Java は代入→init の順だが、init 中に behavior を
/// 読む実行経路が無いため init→代入の順で等価。
pub(crate) fn set_behavior_and_init(
    mut runner: BehaviorRunner,
    mascot: &mut Mascot,
    env: &dyn EnvironmentView,
    table: &BehaviorTable,
    factory: &mut dyn BehaviorFactory,
    rng: &mut dyn Rng,
) -> Result<(), BehaviorError> {
    let result = runner.init(mascot, env, table, factory, rng);
    if mascot.behavior.is_none() {
        mascot.behavior = Some(runner);
    }
    result
}

/// 再配置式（Java Configuration.java L519-522 / L545-548 逐語）。
/// area は multiscreen ? screen(union) : アンカー基準の work area
/// （[`resolve_work_area`] = Java MascotEnvironment.getWorkArea(boolean) L66-114 の
/// 決定木）。乱数を 1 回消費する。
/// - multiscreen=true: `env.screen()`（全画面 union。Java
///   MascotEnvironment.getScreen L127-134 と一致）
/// - multiscreen=false: アンカーが属する作業領域（含む画面が無ければ invisibleScreen）。
///   design §1.9 に記録した「プライマリ固定」の意図的差異は本対応で解消。
///
/// anchor = ((rng * (area.width - 2)) as i32) + area.left + 1, area.top - 256
/// （Java (int) キャスト = 0 への切り捨て。rng.unit() ∈ [0,1) なので正）。
/// 幅から 2 を引き左端に 1 を足すのは、壁登りではなく落下を開始させるため
/// （Java コメント踏襲）。
fn reposition_above_area(mascot: &mut Mascot, env: &dyn EnvironmentView, rng: &mut dyn Rng) {
    let (left, top, width) = if env.multiscreen() {
        let area = env.screen();
        (area.left, area.top, area.width())
    } else {
        let slot = resolve_work_area(env, mascot.anchor());
        let area = env.work_area_state(slot);
        (area.left, area.top, area.width())
    };
    let x = (rng.unit() * f64::from(width - 2)) as i32 + left + 1;
    mascot.set_anchor((x, top - 256));
}

/// XML の `<Behavior>` 1 つ = 1 実行器（Java `UserBehavior` 相当）。
pub struct BehaviorRunner {
    pub name: String,
    action: Box<dyn Action>,
}

impl BehaviorRunner {
    pub fn new(name: impl Into<String>, action: Box<dyn Action>) -> Self {
        BehaviorRunner {
            name: name.into(),
            action,
        }
    }

    /// Java UserBehavior.init L122-143 逐語。
    /// action.init → has_next が false なら直ちに次行動へ遷移
    /// （Java: mascot.setBehavior(configuration.buildNextBehavior(name, mascot))）。
    /// rng は遷移したときのみ build_next_behavior 経由で 1 回消費される。
    pub fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        self.action
            .init(mascot, env, rng)
            .map_err(action_error_to_behavior)?;
        if !self
            .action
            .has_next(mascot, env, rng)
            .map_err(action_error_to_behavior)?
        {
            let next =
                table.build_next_behavior(Some(self.name.as_str()), mascot, env, factory, rng)?;
            set_behavior_and_init(next, mascot, env, table, factory, rng)?;
        }
        Ok(())
    }

    /// Java UserBehavior.next L146-232 逐語。
    /// ①has_next なら action.next ②hotspot 走査（クリック中のみ・一致無しは cursor
    /// クリア）③完了遷移 or off-screen 再配置 + Fall ④LostGround catch。
    pub fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        match self.next_inner(mascot, env, table, factory, rng) {
            Ok(()) => Ok(()),
            // Java catch (VariableException)
            Err(NextFlow::Eval(err)) => Err(BehaviorError::Eval(err)),
            // Java の BehaviorInstantiationException 相当はそのまま伝播する
            Err(NextFlow::Fatal(err)) => Err(err),
            // Java catch (LostGroundException)
            Err(NextFlow::LostGround) => {
                mascot.set_cursor_position(None);
                mascot.set_dragging(false);
                let fall =
                    table.build_behavior_direct(BEHAVIORNAME_FALL, factory, mascot.scale())?;
                set_behavior_and_init(fall, mascot, env, table, factory, rng)
            }
        }
    }

    fn next_inner(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), NextFlow> {
        // Java: if (action.hasNext()) { action.next(); }
        if self
            .action
            .has_next(mascot, env, rng)
            .map_err(NextFlow::from)?
        {
            self.action.next(mascot, env, rng).map_err(NextFlow::from)?;
        }

        // Java L156-181: hotspot 走査（クリック中のときのみ）
        let mut hotspot_state = HotspotState::Inactive;
        if mascot.is_hotspot_clicked() {
            // (C) contains（常に一致の placeholder）は未実装のため、実在する最初の
            // hotspot を一致したものとして扱う。isBehaviorEnabled は build_behavior
            // （Java buildBehavior(name, mascot) 相当）経由で適用される（#9）。
            // contains 実装は #7/#9。
            if let Some(hotspot) = mascot.hotspots().first() {
                hotspot_state = HotspotState::ActiveNull;
                let behaviour = hotspot.behaviour.clone();
                if !behaviour.is_empty() {
                    hotspot_state = HotspotState::Active;
                    let behavior = table
                        .build_behavior(&behaviour, mascot, env, factory, rng)
                        .map_err(NextFlow::Fatal)?;
                    set_behavior_and_init(behavior, mascot, env, table, factory, rng)
                        .map_err(NextFlow::Fatal)?;
                }
            }
            if hotspot_state == HotspotState::Inactive {
                mascot.set_cursor_position(None);
            }
        }

        // Java L183-214
        if hotspot_state != HotspotState::Active {
            if self
                .action
                .has_next(mascot, env, rng)
                .map_err(NextFlow::from)?
            {
                // off-screen 判定（Java L185-191）。image 無しは Java の 0 サイズ
                // 矩形相当として anchor を使う。
                let (bounds_x, bounds_y, bounds_width) = match mascot.get_bounds() {
                    Some(bounds) => (bounds.left, bounds.top, bounds.width()),
                    None => {
                        let (x, y) = mascot.anchor();
                        (x, y, 0)
                    }
                };
                let screen = env.screen();
                if bounds_x + bounds_width <= screen.left
                    || screen.right <= bounds_x
                    || screen.bottom <= bounds_y
                {
                    log::info!("画面外に移動しました ({bounds_x}, {bounds_y})");
                    reposition_above_area(mascot, env, rng);
                    let fall = table
                        .build_behavior_direct(BEHAVIORNAME_FALL, factory, mascot.scale())
                        .map_err(NextFlow::Fatal)?;
                    set_behavior_and_init(fall, mascot, env, table, factory, rng)
                        .map_err(NextFlow::Fatal)?;
                }
            } else {
                log::info!("Behavior `{}` を完了しました", self.name);
                let next = table
                    .build_next_behavior(Some(self.name.as_str()), mascot, env, factory, rng)
                    .map_err(NextFlow::Fatal)?;
                set_behavior_and_init(next, mascot, env, table, factory, rng)
                    .map_err(NextFlow::Fatal)?;
            }
        }
        Ok(())
    }

    /// Java UserBehavior.mousePressed L242-288 逐語。
    /// 左ボタン判定は呼び出し側（#8/#9）の責務。
    pub fn mouse_pressed(
        &mut self,
        mascot: &mut Mascot,
        point: (i32, i32),
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        let mut handled = false;

        // Java L250-268: hotspot 走査
        if let Some(hotspot) = mascot.hotspots().first() {
            // (C) contains は placeholder（常に一致）。isBehaviorEnabled は
            // build_behavior（Java buildBehavior(name, mascot) 相当）経由で適用される。
            let behaviour = hotspot.behaviour.clone();
            handled = true;
            mascot.set_cursor_position(Some(point));
            if !behaviour.is_empty() {
                let behavior = table.build_behavior(&behaviour, mascot, env, factory, rng)?;
                set_behavior_and_init(behavior, mascot, env, table, factory, rng)?;
            }
        }

        // Java L271-277: ドラッグ抑止確認
        if !handled {
            handled = !self
                .action
                .is_draggable(mascot, env, rng)
                .map_err(action_error_to_behavior)?;
        }

        // Java L279-286: ドラッグ開始
        if !handled {
            let dragged =
                table.build_behavior_direct(BEHAVIORNAME_DRAGGED, factory, mascot.scale())?;
            set_behavior_and_init(dragged, mascot, env, table, factory, rng)?;
        }
        Ok(())
    }

    /// Java UserBehavior.mouseReleased L298-319 逐語。
    pub fn mouse_released(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        table: &BehaviorTable,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<(), BehaviorError> {
        if mascot.is_hotspot_clicked() {
            mascot.set_cursor_position(None);
        }
        // ドラッグ中でなければ何もしない（Java コメント踏襲）
        if mascot.is_dragging() {
            mascot.set_dragging(false);
            let thrown =
                table.build_behavior_direct(BEHAVIORNAME_THROWN, factory, mascot.scale())?;
            set_behavior_and_init(thrown, mascot, env, table, factory, rng)?;
        }
        Ok(())
    }
}

/// [`BehaviorTable`] の 1 行（Java BehaviorBuilder のフラット化結果）。
/// Group の conditions は行へ AND 積み上げ済み（BehaviorDef 側に条件は無い）。
#[derive(Debug, Clone)]
pub struct BehaviorRow {
    pub name: String,
    pub frequency: i32,
    pub hidden: bool,
    /// Allowed Behaviours トグル対象（BehaviorDef.toggleable の伝播）。
    pub toggleable: bool,
    pub conditions: Vec<Variable>,
    pub action: SequenceChild,
    pub next: Option<NextBehaviorList>,
}

/// behaviors.xml のフラット化した行動表（Java Configuration.behaviorBuilders 相当）。
/// rows の順 = XML 順。
#[derive(Debug, Clone)]
pub struct BehaviorTable {
    pub rows: Vec<BehaviorRow>,
}

impl BehaviorTable {
    /// [`BehaviorsConfig`] を XML 順保持でフラット化する。
    /// Group は配下の behaviors を全て行に展開し、Group の conditions を AND 積み上げ。
    /// Single は conditions 空でフラット化。
    pub fn new(config: &BehaviorsConfig) -> Self {
        let mut rows = Vec::new();
        for entry in &config.entries {
            match entry {
                BehaviorEntry::Group {
                    conditions,
                    behaviors,
                } => {
                    for def in behaviors {
                        rows.push(BehaviorRow {
                            name: def.name.clone(),
                            frequency: def.frequency,
                            hidden: def.hidden,
                            toggleable: def.toggleable,
                            conditions: conditions.clone(),
                            action: def.action.clone(),
                            next: def.next.clone(),
                        });
                    }
                }
                BehaviorEntry::Single(def) => rows.push(BehaviorRow {
                    name: def.name.clone(),
                    frequency: def.frequency,
                    hidden: def.hidden,
                    toggleable: def.toggleable,
                    conditions: Vec::new(),
                    action: def.action.clone(),
                    next: def.next.clone(),
                }),
            }
        }
        BehaviorTable { rows }
    }

    /// 名前で行を探す（XML 順の線形走査。Java getBehaviorBuilders().get(name) 相当）。
    pub fn find(&self, name: &str) -> Option<&BehaviorRow> {
        self.rows.iter().find(|row| row.name == name)
    }

    /// 次 Behavior を構築する（Java Configuration.buildNextBehavior L459-524 逐語）。
    ///
    /// - 条件評価用 context は呼び出しごとに fresh な [`Variables::new()`]
    ///   （Java new VariableMap 相当）+ [`MascotContext`]。fresh なので `${}` も
    ///   毎回新規評価される（(E) 対応）
    /// - previous_name が None または前行動が additive（Java isNextAdditive:
    ///   next リスト無し = true / 有り = Add 属性。L206-230 の初期化踏襲）なら
    ///   全 top-level rows が候補
    /// - previous_name に next リストがあればその参照も候補（additive なら両方合流・
    ///   top-level 先・その後 refs）
    /// - 候補条件は conditions AND + frequency != 0（Java isEffective L374-390/L188-204
    ///   逐語）。評価エラーは log::warn してその候補をスキップ（Err にしない）
    /// - 候補はさらに isBehaviorEnabled も通過したもの（Java L481 top-level /
    ///   L496 refs 逐語）。非 toggleable は短絡により env を参照しない・toggleable
    ///   は [`EnvironmentView::behavior_disabled`]（true = 無効リストに含まれる）が
    ///   false（= 無効リスト非含有）のとき通過。未知名の参照候補は除外（Java L602）
    /// - total_frequency > 0 で頻度選択（乱数 1 回・XML 順 walk）。
    ///   決まらない / total == 0 なら再配置（乱数 1 回）+ Fall フォールバック
    pub fn build_next_behavior(
        &self,
        previous_name: Option<&str>,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<BehaviorRunner, BehaviorError> {
        let snapshot = mascot.eval_snapshot();
        let ctx = MascotContext {
            snapshot: &snapshot,
            env,
        };
        let mut vars = Variables::new();

        let mut candidates: Vec<(&str, i64)> = Vec::new();
        let mut total_frequency = 0i64;

        let previous = previous_name.and_then(|name| self.find(name));
        // Java: prevBehaviorBuilder == null || prevBehaviorBuilder.isNextAdditive()
        let previous_additive = match previous {
            Some(row) => row.next.as_ref().is_none_or(|next| next.add),
            None => true,
        };
        if previous_additive {
            for row in &self.rows {
                // Java L481: isEffective(context) && isBehaviorEnabled(builder, mascot)
                //（短絡評価・左から右。非 toggleable では env を呼ばない）
                if Self::row_is_effective(row, &mut vars, &ctx)
                    && Self::is_behavior_enabled(row, mascot.image_set_name(), env)
                {
                    candidates.push((row.name.as_str(), i64::from(row.frequency)));
                    total_frequency += i64::from(row.frequency);
                }
            }
        }

        // Java: prevBehaviorBuilder != null && !getNextBehaviorBuilders().isEmpty()
        if let Some(next) = previous.and_then(|row| row.next.as_ref()) {
            for reference in &next.references {
                // Java L496: isEffective(context) && isBehaviorEnabled(name, mascot)
                //（String オーバーロード = 未知名は false・L598-604）
                if Self::ref_is_effective(reference, &mut vars, &ctx)
                    && self.is_behavior_enabled_by_name(
                        &reference.name,
                        mascot.image_set_name(),
                        env,
                    )
                {
                    candidates.push((reference.name.as_str(), i64::from(reference.frequency)));
                    total_frequency += i64::from(reference.frequency);
                }
            }
        }

        // Java L506-515: 頻度選択
        if total_frequency > 0 {
            let mut random = rng.unit() * total_frequency as f64;
            for (name, frequency) in &candidates {
                random -= *frequency as f64;
                if random < 0.0 {
                    return self.build_behavior_direct(name, factory, mascot.scale());
                }
            }
        }

        // Java L517-523: 候補無し → 再配置して Fall へ
        reposition_above_area(mascot, env, rng);
        self.build_behavior_direct(BEHAVIORNAME_FALL, factory, mascot.scale())
    }

    /// 名前で Behavior を構築する（Java Configuration.buildBehavior(name, mascot)
    /// L540-555 逐語）。
    /// - 名前未検出 → `Err(UnknownBehavior)`（Java は BehaviorInstantiationException
    ///   を throw・再配置はしない）
    /// - 既知名かつ無効（[`Self::is_behavior_enabled`] が false）→ 再配置
    ///   （乱数 1 回消費・Java L545-548 逐語）して Fall を返す（L549 逐語）
    /// - 有効 → [`Self::build_behavior_direct`]（乱数を消費しない）
    pub fn build_behavior(
        &self,
        name: &str,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) -> Result<BehaviorRunner, BehaviorError> {
        let Some(row) = self.find(name) else {
            return Err(BehaviorError::UnknownBehavior(name.to_string()));
        };
        if Self::is_behavior_enabled(row, mascot.image_set_name(), env) {
            self.build_behavior_direct(name, factory, mascot.scale())
        } else {
            log::warn!("Behavior `{name}` は無効化されているため Fall へフォールバックします");
            reposition_above_area(mascot, env, rng);
            self.build_behavior_direct(BEHAVIORNAME_FALL, factory, mascot.scale())
        }
    }

    /// 名前で Behavior を構築する（Java Configuration.buildBehavior(name) L566-573 逐語）。
    /// 構築前に `factory.set_scale(scale)` を適用する（per-set scale を
    /// 構築時 [`scale_pose`](crate::render::imageset::scale_pose) へ伝える）。
    pub fn build_behavior_direct(
        &self,
        name: &str,
        factory: &mut dyn BehaviorFactory,
        scale: f64,
    ) -> Result<BehaviorRunner, BehaviorError> {
        factory.set_scale(scale);
        let row = self
            .find(name)
            .ok_or_else(|| BehaviorError::UnknownBehavior(name.to_string()))?;
        let action = factory.build_action(&row.action)?;
        Ok(BehaviorRunner::new(name, action))
    }

    /// Java BehaviorBuilder.isEffective L374-390 逐語:
    /// frequency == 0 → false・conditions 全てが true → true。
    /// 評価エラーは Java では呼び出し側（buildNextBehavior）で catch されて
    /// 候補スキップになるため、ここでは warn ログして false を返す。
    /// 数値結果は Java の (Boolean) キャスト失敗相当として false 扱い。
    fn row_is_effective(row: &BehaviorRow, vars: &mut Variables, ctx: &dyn EvalContext) -> bool {
        if row.frequency == 0 {
            return false;
        }
        row.conditions
            .iter()
            .all(|condition| match vars.eval(condition, ctx) {
                Ok(EvalValue::Bool(true)) => true,
                Ok(_) => false,
                Err(err) => {
                    log::warn!("Behavior `{}` の条件を評価できません: {err}", row.name);
                    false
                }
            })
    }

    /// Java BehaviorRef.isEffective L188-204 逐語。
    fn ref_is_effective(
        reference: &BehaviorRef,
        vars: &mut Variables,
        ctx: &dyn EvalContext,
    ) -> bool {
        if reference.frequency == 0 {
            return false;
        }
        match &reference.condition {
            Some(condition) => match vars.eval(condition, ctx) {
                Ok(EvalValue::Bool(true)) => true,
                Ok(_) => false,
                Err(err) => {
                    log::warn!(
                        "Behavior 参照 `{}` の条件を評価できません: {err}",
                        reference.name
                    );
                    false
                }
            },
            None => true,
        }
    }

    /// Java Configuration.isBehaviorEnabled(BehaviorBuilder, Mascot) L583-588 逐語:
    /// `builder.isToggleable() && disabledBehaviors.containsKey(imageSet)` が成立する
    /// ときのみ無効リストを引き、それ以外は常に true。短絡評価により非 toggleable では
    /// env を呼ばない（乱数も消費しない・Java 同様）。
    ///
    /// [`EnvironmentView::behavior_disabled`] は「その (image_set, behavior) が
    /// Allowed Behaviours 無効リストに含まれる = true（= トグル OFF）」を返す契約
    /// （app 実装は #9b）のため、`!env.behavior_disabled(...)` が Java の
    /// `!disabledBehaviors.get(imageSet).contains(name)` に対応する。
    ///
    /// #9b から Manager の [`behavior_menu_items`]
    /// `manager::Manager::behavior_menu_items` が同一式を再利用するため
    /// `pub(crate)`（mascot 参照ではなく set 文字列引数・式自体は不変）。
    pub(crate) fn is_behavior_enabled(
        row: &BehaviorRow,
        image_set: &str,
        env: &dyn EnvironmentView,
    ) -> bool {
        !row.toggleable || !env.behavior_disabled(image_set, &row.name)
    }

    /// Java Configuration.isBehaviorEnabled(String, Mascot) L598-604 逐語。
    /// 未知名は false（L602・refs 経路の候補フィルタでのみ使用）。
    fn is_behavior_enabled_by_name(
        &self,
        name: &str,
        image_set: &str,
        env: &dyn EnvironmentView,
    ) -> bool {
        match self.find(name) {
            Some(row) => Self::is_behavior_enabled(row, image_set, env),
            None => false,
        }
    }
}
