//! Manager — マスコット集合・tick スケジュール（Java `Manager.java` 相当・タスク #8）。
//!
//! Java 正本（.tmp/java-ref/Manager.java）を仕様として逐語移植する:
//! - tick 本体 L201-244: ①環境更新 → ②added 反映 → ③removed 反映 → ④全員 tick →
//!   exitOnLastRemoved
//! - remainOne / remainNone / disposeAll L346-456（末尾 index 降順 dispose）
//! - setBehaviorAll L291-340（構築失敗 → log + dispose）
//! - isPaused / togglePauseAll L465-495（空 = false・allMatch）
//! - getCount / getMascotWithAffordance / hasOverlappingMascotsAtPoint L522-601
//!
//! 構造上の意図的差異（Java 一致検証時に差し引くこと）:
//! 1. Java は内部 Ticker スレッド（L146-184）で 40ms 周期に tick を回すが、本実装は
//!    tao イベントループ 1 本（AGENTS §3）のため tick 到来判定は
//!    [`Manager::tick_due`] / [`Manager::next_delay`] 純関数 + #10 の
//!    `ControlFlow::WaitUntil` で行う
//! 2. Java Ticker L158-165 は遅延を最大 2 tick 分だけ補填するが、AGENTS §5-7
//!    （スリープ復帰後は最大 1 tick だけ進める）に従い elapsed >= 間隔なら常時
//!    40ms 後に 1 tick だけ進める（バーストクランプ・意図的差異）
//! 3. Java `Manager.add` は旧 Manager からの remove や added::remove 相当の重複制御を
//!    持つが（L252-269）、lib 内に Manager は 1 つのため単純な追加キューのみ
//! 4. Java `Manager.remove`（L277-283）に相当する外部削除 API は持たない。
//!    削除は [`Mascot::dispose`](crate::mascot::Mascot::dispose) の remove_pending
//!    フラグ経由のみ（design §1.5）
//! 5. setBehaviorAll の Configuration 取得（L298 Main.getInstance()）は未導入のため、
//!    本実装は全 mascot が同一 BehaviorTable / ファクトリを共有する構造
//!    （design §1.5）。set 単位 conf 差し替えは将来タスク
//! 6. ReadWriteLock / synchronized は排除（単一スレッド・design §1.5）

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::environment::Environment;
use crate::mascot::behavior::{BehaviorFactory, BehaviorTable};
use crate::mascot::{EnvironmentView, Mascot, Rng};
use crate::render::imageset::ImageSet;

/// image set resolver の型（Java `Main.getConfiguration(imageSet)` 相当の注入点）。
type ImageSetResolver = dyn FnMut(&str) -> Option<Arc<ImageSet>>;

/// マスコット集合の所有者（Java `Manager` 相当・スレッド/lock は排除）。
pub struct Manager {
    /// Java `mascots` L42 相当。
    mascots: Vec<Mascot>,
    /// Java `added` LinkedHashSet L51 相当（Rust は Vec・次 tick 一括反映）。
    added: Vec<Mascot>,
    /// Java `getEnvironment()` 相当・Manager が所有。
    environment: Environment,
    /// Java `Main.getInstance().getConfiguration(imageSet)` 相当の代替
    /// （全 mascot 共有・構造上の意図的差異 doc 5）。
    table: BehaviorTable,
    /// Java ファクトリ相当（Box 注入・既存 Mascot::new と同一パターン）。
    factory: Box<dyn BehaviorFactory>,
    /// Java `Math.random()` の注入版（design §1.5）。
    rng: Box<dyn Rng>,
    /// 新規 mascot 生成時の image set 解決（Breed 出生・Java L298 相当）。
    /// 未設定なら spawn をスキップする（log）。
    resolver: Option<Box<ImageSetResolver>>,
    /// Java `exitOnLastRemoved` L72 相当（既定 true）。
    exit_on_last_removed: bool,
    /// Java `enabled` L83 相当（既定 true）。
    enabled: bool,
    /// 全員消滅後の exit 依頼フラグ（Java L240-243 の `Main.exit()` 相当）。
    /// process::exit はしない（#10 がイベントループで消費）。
    exit_flag: bool,
}

impl Manager {
    /// Java `Manager.TICK_INTERVAL` L37 逐語（呼び出し間隔の最小値・ms）。
    /// （テスト契約 pin は assoc の `Manager::TICK_INTERVAL_MS`）
    pub const TICK_INTERVAL_MS: u64 = 40;

    /// tick 到来判定（Java Ticker L158 `cur - prev >= TICK_INTERVAL` 逐語）。
    /// elapsed が [`TICK_INTERVAL_MS`] 未満なら tick はまだ回さない。
    ///
    /// #10 のイベントループ glue が使う純関数（構造差異 doc 1）。
    pub fn tick_due(elapsed: Duration) -> bool {
        elapsed >= Duration::from_millis(Self::TICK_INTERVAL_MS)
    }

    /// 次回 tick までの待ち時間。
    ///
    /// - elapsed < 間隔 → 残り時間（`Thread.sleep` 相当の OS 級待機）
    /// - elapsed >= 間隔 → 常に 40ms 後。
    ///   Java Ticker L158-165 は `cur <= prev + TICK_INTERVAL * 2` のとき
    ///   `prev += TICK_INTERVAL` で最大 2 tick 分の補填をするが、AGENTS §5-7
    ///   により elapsed が複数間隔分でも実行は 1 tick 分のみ・次回 40ms 後
    ///   （バーストクランプ・意図的差異 doc 2）
    pub fn next_delay(elapsed: Duration) -> Duration {
        let interval = Duration::from_millis(Self::TICK_INTERVAL_MS);
        if elapsed < interval {
            interval - elapsed
        } else {
            interval
        }
    }

    /// Java コンストラクタ + フィールド初期化相当。
    /// `factory` / `rng` は Box 注入（design §1.5・Mascot と同一パターン）。
    pub fn new(
        environment: Environment,
        table: BehaviorTable,
        factory: Box<dyn BehaviorFactory>,
        rng: Box<dyn Rng>,
    ) -> Manager {
        Manager {
            mascots: Vec::new(),
            added: Vec::new(),
            environment,
            table,
            factory,
            rng,
            resolver: None,
            exit_on_last_removed: true,
            enabled: true,
            exit_flag: false,
        }
    }

    /// image set resolver を設定する（Java `Main.getConfiguration(imageSet)` 相当・
    /// ロード済み set の参照を返す。未設定または None = spawn スキップ）。
    pub fn set_image_set_resolver(
        &mut self,
        resolver: impl FnMut(&str) -> Option<Arc<ImageSet>> + 'static,
    ) {
        self.resolver = Some(Box::new(resolver));
    }

    /// Mascot を追加キューへ積む（Java `add` L252-269 逐語のうち
    /// manager 二重管理制御を除く部分・反映は次 tick・AGENTS §5-6）。
    pub fn add(&mut self, mascot: Mascot) {
        self.added.push(mascot);
    }

    /// Java `tick` L201-244 逐語:
    /// ①環境更新 → ②追加キュー / spawn キュー / 削除の反映 → ③全員 tick。
    /// 描画（Java は `mascot.apply()` で window 反映）は本 tick ではしない:
    /// #10 が [`Manager::apply_all`] で draw + `needs_repaint` クリアを行う
    /// （Java Mascot.apply L662-708 相当の glue）。
    pub fn tick(&mut self, _now: Instant) {
        if !self.enabled {
            // Java L166 / L503-515: enabled false → tick 呼び出し自体がなされない逐語相当
            return;
        }

        // ① Java L203: 環境更新（NativeFactory 注入を排除して直保持）
        self.environment.tick();

        // Java L209-214: added の一括反映
        if !self.added.is_empty() {
            let added = std::mem::take(&mut self.added);
            self.mascots.extend(added);
        }

        // ② spawn キュー drain（Java Breed L94 は manager.add() 即時だが
        //   AGENTS §5-6 により次 tick 一括反映・意図的差異 design §1.8(f)）。
        //   set 不在 / behavior 構築失敗 → log + スキップ（Java Breed L95-99 逐語）。
        let spawns = self.environment.drain_spawns();
        let env: &dyn EnvironmentView = &self.environment;
        for request in spawns {
            let resolved = match self.resolver.as_mut() {
                Some(resolver) => resolver(&request.image_set_name),
                None => {
                    log::warn!("image set resolver 未設定のため spawn をスキップします");
                    None
                }
            };
            let Some(image_set) = resolved else {
                log::warn!(
                    "image set `{}` を解決できなかったため spawn をスキップします",
                    request.image_set_name
                );
                continue;
            };

            let mut mascot =
                Mascot::new(request.image_set_name.as_str(), image_set, request.anchor);
            // Java Breed.java L90: setLookRight(action.getMascot().isLookRight())
            mascot.set_look_right(request.look_right);
            // Java Breed.java L93: getBornBehavior() で Behavior 構築（第 4 引数伝播・#8）。
            match self.table.build_behavior(
                &request.behavior_name,
                &mut mascot,
                env,
                self.factory.as_mut(),
            ) {
                Ok(runner) => {
                    if let Err(err) = mascot.set_behavior(
                        Some(runner),
                        env,
                        &self.table,
                        self.factory.as_mut(),
                        self.rng.as_mut(),
                    ) {
                        // Java Breed.java L95-99: 構築 / 実行例外 → log + dispose 相当
                        //（未追加のまま破棄・子は Manager に乗らない）
                        log::error!("spawn された Mascot の behavior 初期化に失敗: {err}");
                    } else {
                        // Java L94: manager.add(newMascot)
                        self.mascots.push(mascot);
                    }
                }
                Err(err) => {
                    log::error!(
                        "spawn された Mascot の behavior `{}` 構築に失敗: {err}",
                        request.behavior_name
                    );
                }
            }
        }

        // Java L217-220（removed → Rust は remove_pending フラグ一括反映）
        self.mascots.retain(|mascot| !mascot.remove_pending());

        // Java L223: noMascots
        let no_mascots = self.mascots.is_empty();

        // Java L227-229: 全員 1 tick 進める
        if !no_mascots {
            for mascot in &mut self.mascots {
                mascot.tick(env, &self.table, self.factory.as_mut(), self.rng.as_mut());
            }
        }
        // Java L232-234（mascot.apply ループ）は #10 が [`Manager::apply_all`] で
        // 実施する（Manager 自体は描画しない）

        // Java L240-243 逐語: 全員消滅 → exit 依頼（process::exit はしない）
        if self.exit_on_last_removed && no_mascots {
            self.exit_flag = true;
        }
    }

    /// 全員へ glue 適用する（Java Mascot.apply L662-708 相当の呼び出し点・
    /// #10 が draw + `clear_needs_repaint` を実施できる形）。
    pub fn apply_all(&mut self, apply: impl FnMut(&mut Mascot)) {
        self.mascots.iter_mut().for_each(apply);
    }

    /// 環境の観測点（spawn キュー push 等・Java `getEnvironment` 相当）。
    pub fn environment_view(&self) -> &dyn EnvironmentView {
        &self.environment
    }

    /// Java `getCount()` L522-524 逐語。
    pub fn count(&self) -> usize {
        self.mascots.len()
    }

    /// Java `getCount(String)` L534-549 逐語（set 単位の計数）。
    pub fn count_of(&self, image_set_name: &str) -> usize {
        if self.mascots.is_empty() {
            return 0;
        }
        self.mascots
            .iter()
            .filter(|mascot| mascot.image_set_name() == image_set_name)
            .count()
    }

    /// 空判定（Java L468 `mascots.isEmpty()` 該当部相当）。
    pub fn is_empty(&self) -> bool {
        self.mascots.is_empty()
    }

    /// Java `remainOne()` L346-356 逐語: 先頭 1 体を残し、末尾 index から
    /// 降順に dispose する（dispose 順は観測不能のため Java 同一の降順走査を保持）。
    pub fn remain_one(&mut self) {
        for index in (1..self.mascots.len()).rev() {
            self.mascots[index].dispose();
        }
    }

    /// Java `remainOne(String)` L385-401 逐語: index 降順（末尾→先頭）で走査し、
    /// 同 image set で最初に見つけた 1 体（= リスト末尾側・最後に追加されたもの）を
    /// `isFirst` フラグで keep、それ以降の同 set mascot を dispose する。
    /// Javadoc L379-381 の「The remaining mascot will be the first mascot with the
    /// specified image set」は本体（末尾側を残す）と食い違う上流 quirk
    /// （ユーザー承認により本体準拠）。
    pub fn remain_one_of_set(&mut self, image_set_name: &str) {
        let mut is_first = true;
        for index in (0..self.mascots.len()).rev() {
            if self.mascots[index].image_set_name() == image_set_name && is_first {
                is_first = false;
            } else if self.mascots[index].image_set_name() == image_set_name && !is_first {
                self.mascots[index].dispose();
            }
        }
    }

    /// Java `remainNone(String)` L429-442 逐語（末尾から index 降順 dispose。
    /// dispose 順は観測不能・降順走査を保持）。
    pub fn remain_none_of_set(&mut self, image_set_name: &str) {
        for index in (0..self.mascots.len()).rev() {
            if self.mascots[index].image_set_name() == image_set_name {
                self.mascots[index].dispose();
            }
        }
    }

    /// Java `disposeAll` L447-456 逐語（末尾から index 降順 dispose。
    /// dispose 順は観測不能・降順走査を保持）。削除反映は次 tick。
    pub fn dispose_all(&mut self) {
        for index in (0..self.mascots.len()).rev() {
            self.mascots[index].dispose();
        }
    }

    /// Java `setBehaviorAll(String)` L291-310 逐語（全員へ setBehavior・
    /// 構築 / 実行失敗 → log + dispose（L301-306 逐語・削除は次 tick））。
    pub fn set_behavior_all(&mut self, name: &str) {
        if self.mascots.is_empty() {
            return;
        }
        let env: &dyn EnvironmentView = &self.environment;
        for mascot in &mut self.mascots {
            match self
                .table
                .build_behavior(name, mascot, env, self.factory.as_mut())
            {
                Ok(runner) => {
                    if let Err(err) = mascot.set_behavior(
                        Some(runner),
                        env,
                        &self.table,
                        self.factory.as_mut(),
                        self.rng.as_mut(),
                    ) {
                        log::error!(r#"Behavior "{name}" の設定に失敗: {err}"#);
                        mascot.dispose();
                    }
                }
                Err(err) => {
                    log::error!(r#"Behavior "{name}" の構築に失敗: {err}"#);
                    mascot.dispose();
                }
            }
        }
    }

    /// Java L133-135 逐語（既定 true・L72）。
    pub fn set_exit_on_last_removed(&mut self, exit_on_last_removed: bool) {
        self.exit_on_last_removed = exit_on_last_removed;
    }

    /// 全員消滅 tick 後に true。process::exit はしない（#10 がイベントループで消費）。
    /// （[`Manager::tick`] 内で立つ・Java `Main.exit()` 相当の flag 版）
    pub fn should_exit(&self) -> bool {
        self.exit_flag
    }

    /// Java `isPaused` L465-475 逐語（空 = false・allMatch 契約）。
    pub fn is_paused(&self) -> bool {
        if self.mascots.is_empty() {
            return false;
        }
        self.mascots.iter().all(Mascot::is_paused)
    }

    /// Java `togglePauseAll` L480-495 逐語（空 = 何もしない）。
    /// `mascot.setPausedNoCallback` 相当（トレイ通知は #9）。
    pub fn toggle_pause_all(&mut self) {
        if self.mascots.is_empty() {
            return;
        }
        let is_paused = self.mascots.iter().all(Mascot::is_paused);
        for mascot in &mut self.mascots {
            mascot.set_paused(!is_paused);
        }
    }

    /// Java L513-515 逐語。false 中の tick は Java の「tick 呼び出しがなされない」
    /// 相当として何もしない（追加キュー・環境更新も反映しない）。
    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    /// Java `isEnabled` L503-505 相当。
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Java `getMascotWithAffordance` L558-573 相当（WeakReference 相当として
    /// 借用参照を返す・線形走査の最初の一致）。
    pub fn get_mascot_with_affordance(&mut self, affordance: &str) -> Option<&mut Mascot> {
        if self.mascots.is_empty() {
            return None;
        }
        self.mascots
            .iter_mut()
            .find(|mascot| mascot.affordances().iter().any(|value| value == affordance))
    }

    /// Java `hasOverlappingMascotsAtPoint` L581-601 逐語（同一 anchor 2 体以上）。
    pub fn has_overlapping_mascots_at(&self, anchor: (i32, i32)) -> bool {
        let mut count = 0;
        if !self.mascots.is_empty() {
            for mascot in &self.mascots {
                if mascot.anchor() == anchor {
                    count += 1;
                }
                if count > 1 {
                    return true;
                }
            }
        }
        false
    }
}
