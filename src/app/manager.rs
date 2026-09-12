//! Manager — マスコット集合・tick スケジュール（Java `Manager.java` 相当・タスク #8）。
//!
//! Java 正本（.tmp/java-ref/Manager.java）を仕様として逐語移植する:
//! - tick 本体 L201-244: ①環境更新 → ②added 反映 → ③removed 反映 → ④全員 tick →
//!   exitOnLastRemoved
//! - remainOne / remainNone / disposeAll L346-456（末尾 index 降順 dispose）
//! - setBehaviorAll L291-340（構築失敗 → log + dispose）
//! - isPaused / togglePauseAll L465-495（空 = false・allMatch）
//! - getCount / getMascotWithAffordance / hasOverlappingMascotsAtPoint L522-601
//! - #9b 追加: createMascot L480-505（rng 消費は要求時・build_next_behavior(None)）、
//!   setBehaviorAll(Configuration, name, imageSet) L320-340（per-set 構築）、
//!   setBehaviorAll の Configuration 取得（getConfiguration(mascot.getImageSet()) 相当
//!   = set 別 BehaviorTable オーバーレイ map・(AF)）、
//!   Main.setMascotBehaviorEnabled L526-544 の passthrough、restoreWindows
//!   passthrough（WindowsEnvironment L292-347・Environment 側実装）、
//!   Mascot ポップアップ分類 L523-553（behavior_menu_items）
//! - #9d 追加: Reload（素材ローダは app/reload.rs・本モジュールは参照付け替え）。
//!   Java `Main.reloadAllImageSets`（L547-566）の「全消し + 再作成」は採用せず、
//!   design.md §2 Reload 方針（§1.10 (d) 9d・ユーザー承認）により**意図的差異**の
//!   参照付け替え路線（存続 mascot の anchor 等は維持・ImageSet Arc と行動表のみ差し替え）
//! - #9c 追加: request_spawn_random（Main.createMascot() 無引数版 L466-473 逐語）、
//!   Allowed passthrough 5 種（Environment setter 委譲・Sounds は Phase 1 no-op の
//!   ため作らない）、popup 単体操作（Mascot.java L517-562 相当:
//!   set_behavior_at / toggle_pause_at / dismiss_at）
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
//! 5. setBehaviorAll の Configuration 取得（L298 Main.getInstance()）は resolver /
//!    BehaviorTable 注入に置き換え済み。#9b で Main.getConfiguration(imageSet) 相当
//!    の **set 別 BehaviorTable 上書き map**（[`Manager::set_behavior_table`]+
//!    base フォールバック・(AF)）を導入し、全構築経路（spawn drain / setBehaviorAll
//!    群 / メニュー / mascot.tick の次行動構築）が「マスコット自身の set」（または
//!    要求 set）の table を使う。ファクトリ / rng のみ全 set 共有
//! 6. ReadWriteLock / synchronized は排除（単一スレッド・design §1.5）
//! 7. setBehavior は構造上の簡略: Java L291-310 は Main.setBehavior(…L962-967) 経由
//!    （中で setBehavior も呼ぶ）のため、Mascot::set_behavior から set_behavior_and_init
//!    を経由する同一構造（#8）

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::app::environment::{Environment, PinnedWindow};
use crate::app::reload::ReloadMaterial;
use crate::mascot::behavior::{BehaviorError, BehaviorFactory, BehaviorTable};
use crate::mascot::{EnvironmentView, Mascot, Rng};
use crate::render::imageset::ImageSet;

/// image set resolver の型（Java `Main.getConfiguration(imageSet)` 相当の注入点）。
type ImageSetResolver = dyn FnMut(&str) -> Option<Arc<ImageSet>>;

/// マスコット右クリック メニューの行動分類（Java `Mascot` ポップアップ
/// L523-553 相当・#9b）。双方とも table 挿入順。
pub struct BehaviorMenu {
    /// 有効 && 名前に "/" を含まない非 toggleable 行動（setBehavior メニュー項目）。
    /// toggleable 且つ有効な行動も Java 同様 selectable に出る（L529-547 逐語）。
    pub selectable: Vec<String>,
    /// Allowed Behaviours トグル項目（名前, checked = enabled = 無効リスト非含有）。
    pub toggleable: Vec<(String, bool)>,
}

/// マスコット集合の所有者（Java `Manager` 相当・スレッド/lock は排除）。
pub struct Manager {
    /// Java `mascots` L42 相当。
    mascots: Vec<Mascot>,
    /// Java `added` LinkedHashSet L51 相当（Rust は Vec・次 tick 一括反映）。
    added: Vec<Mascot>,
    /// Java `getEnvironment()` 相当・Manager が所有。
    environment: Environment,
    /// 既定の BehaviorTable（Java Main の既定 Configuration 相当）。
    /// set 別の上書きは [`Manager::set_tables`]（#9b・(AF)）。
    table: BehaviorTable,
    /// Main.getConfiguration(imageSet) 相当の set 別 BehaviorTable オーバーレイ
    /// （#9b・(AF)・9d Reload が登録する）。未登録 set は base `table` にフォールバック。
    set_tables: HashMap<String, BehaviorTable>,
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
    /// tick の retain（[`Manager::tick`] 内）で除去した mascot の「除去前 index」
    /// （#10b-2b・glue の view 同期用）。昇順で蓄積し、[`Manager::take_removed`]
    /// の呼び出しで drain する。dispose_all / reload の全消しは remove_pending を
    /// 立てるだけなので、該当分は次 tick の retain を通って本記録に現れる
    /// （glue は take_removed を唯一の除去同期点として扱ってよい）。
    removed_indices: Vec<usize>,
    /// #30 item 4: ユーザーのドラッグドロップで窓を最前面固定するトグル
    /// （既定 false・トレイ Allowed Behaviours から設定）。
    pin_dropped_window_allowed: bool,
    /// #30 item 7: 直前の押下が保持者の引きはがし（pin 解除）だったか（既定 false）。
    /// true の間は次の解放で再ピンしない（[`Manager::mouse_released_at`]）。
    pin_pull_off: bool,
    /// #30-8b 案A: pin 成立後、保持者が下端掴み行為を一度でも取ったか（既定 false）。
    /// true のとき保持者が Fall / Thrown へ遷移したら unpin する。
    /// pin 成立時・unpin 時に false へリセットする。
    pin_has_clung: bool,
}

/// 「要求 set の table を選ぶ」共通ヘルパ（Java `getConfiguration(imageSet)` 相当）。
/// field-disjoint borrow で呼ぶため静的ヘルパにする（`&self` を取らない）。
fn table_for<'a>(
    set_tables: &'a HashMap<String, BehaviorTable>,
    base: &'a BehaviorTable,
    image_set_name: &str,
) -> &'a BehaviorTable {
    set_tables.get(image_set_name).unwrap_or(base)
}

/// #30 item 4: 下端掴み 3 種（behaviors.xml L138-143）かどうか。
/// 保持者の落下 clamp で「既に下端掴みなら再遷移しない」判定に使う。
fn is_bottom_behavior(name: Option<&str>) -> bool {
    matches!(
        name,
        Some("ClimbIEBottom") | Some("GrabIEBottomLeftWall") | Some("GrabIEBottomRightWall")
    )
}

/// タスク #30-9(C): ピン保持中に飛び降り系行動（窓外へ落下し 30-8b で pin を
/// 解除してしまう）を、同系統の「飛び降りない」行動へ差し替える写像。
/// 対象外（縁伝い・下端掴み・安全行動自身など）は None を返す（冪等）。
fn pin_safe_replacement(name: Option<&str>) -> Option<&'static str> {
    match name {
        Some("JumpFromLeftEdgeOfIE") => Some("SitOnTheLeftEdgeOfIE"),
        Some("JumpFromRightEdgeOfIE") => Some("SitOnTheRightEdgeOfIE"),
        Some("WalkLeftAlongIEAndJump") => Some("WalkLeftAlongIEAndSit"),
        Some("WalkRightAlongIEAndJump") => Some("WalkRightAlongIEAndSit"),
        _ => None,
    }
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
            set_tables: HashMap::new(),
            factory,
            rng,
            resolver: None,
            exit_on_last_removed: true,
            enabled: true,
            exit_flag: false,
            removed_indices: Vec::new(),
            pin_dropped_window_allowed: false,
            pin_pull_off: false,
            pin_has_clung: false,
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

    /// set 別 BehaviorTable を上書き登録する（Java
    /// `Main.getConfiguration(imageSet)` が set 毎の Configuration を返す部分相当・
    /// #9b・(AF)・9d Reload が map に書く前提）。未登録 set は base table に
    /// フォールバックする。
    pub fn set_behavior_table(&mut self, image_set_name: &str, table: BehaviorTable) {
        self.set_tables.insert(image_set_name.to_string(), table);
    }

    /// Main.createMascot(String) L480-505 相当: rng.unit を**呼び出し時に 1 回消費**
    /// して初期向きを決め（L490 setLookRight(Math.random() < 0.5) 逐語）、anchor
    /// (-4000,-4000)・behavior_name None の spawn 要求をキューへ積む。実際の
    /// Mascot 生成・buildNextBehavior(None) 経路の構築・追加は次 tick の
    /// [`Manager::tick`] drain（AGENTS §5-6 追加キューイング・意図的差異 design §1.8(f)）。
    /// resolver 未設定 / 未知 set は drain でスキップされる（既存 drain 挙動踏襲）。
    pub fn request_spawn(&mut self, image_set_name: &str) {
        // Java L490: mascot.setLookRight(Math.random() < 0.5)（乱数消費はこの呼び出し時）
        let look_right = self.rng.unit() < 0.5;
        self.environment_view()
            .queue_spawn_next(image_set_name, (-4000, -4000), look_right);
    }

    /// Java `Main.createMascot()`（無引数版）L466-473 逐語: set 一覧からランダムに
    /// 1 set 選んで spawn 要求する。
    ///
    /// - Java L468-470: `if (length == 0) { return; }` が乱数取得より先のため、
    ///   空スライスでは warn ログ + no-op + **rng を消費しない**
    /// - Java L471: `int random = (int) (length * Math.random())`（0 向け切り捨て）。
    ///   選択用に [`Rng::unit`] を**ちょうど 1 回**消費する。`unit()` は [0,1) 契約の
    ///   ため index は常に範囲内・追加クランプはしない（逐語維持）
    /// - Java L472: `createMascot(imageSets.get(random))`。向き決定の乱数
    ///   （L490）は [`Manager::request_spawn`] 内で 1 回消費するため、spawn 1 体
    ///   あたりの合計消費は 2 回
    pub fn request_spawn_random(&mut self, image_sets: &[String]) {
        if image_sets.is_empty() {
            // Java L468-470: length == 0 → return（乱数取得より先）
            log::warn!("ignoring spawn request: no image sets are available");
            return;
        }
        // Java L471: int random = (int) (length * Math.random())（切り捨て）
        let index = (self.rng.unit() * image_sets.len() as f64) as usize;
        // Java L472: createMascot(imageSets.get(random))
        self.request_spawn(&image_sets[index]);
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
        //   構築は「要求 set」の table を使う（Java L298
        //   getConfiguration(imageSet) 相当・#9b (AF)）:
        //   None → buildNextBehavior(null)（Main.java L497 逐語）/
        //   Some(name) → buildBehavior(name)（Breed.java L93 逐語。
        //   Some("")= BornBehaviour 省略 → Java 同様構築 Err → スキップ）。
        let spawns = self.environment.drain_spawns();
        let env: &dyn EnvironmentView = &self.environment;
        for request in spawns {
            let resolved = match self.resolver.as_mut() {
                Some(resolver) => resolver(&request.image_set_name),
                None => {
                    log::warn!("skipping spawn: image set resolver is not configured");
                    None
                }
            };
            let Some(image_set) = resolved else {
                log::warn!(
                    "skipping spawn: could not resolve image set `{}`",
                    request.image_set_name
                );
                continue;
            };

            let mut mascot =
                Mascot::new(request.image_set_name.as_str(), image_set, request.anchor);
            // Java Breed.java L90: setLookRight(action.getMascot().isLookRight())
            mascot.set_look_right(request.look_right);
            let table = table_for(&self.set_tables, &self.table, &request.image_set_name);
            // Java Breed.java L93 / Main.java L497: born behavior 構築（第 4 引数伝播・#8）
            let built = match &request.behavior_name {
                None => table.build_next_behavior(
                    None,
                    &mut mascot,
                    env,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ),
                Some(name) => table.build_behavior(
                    name,
                    &mut mascot,
                    env,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ),
            };
            match built {
                Ok(runner) => {
                    if let Err(err) = mascot.set_behavior(
                        Some(runner),
                        env,
                        table,
                        self.factory.as_mut(),
                        self.rng.as_mut(),
                    ) {
                        // Java Breed.java L95-99 / Main.java L498-503: 構築 / 実行例外 →
                        // log + dispose 相当（未追加のまま破棄・子は Manager に乗らない）
                        log::error!("failed to initialize behavior of spawned mascot: {err}");
                    } else {
                        // Java L94: manager.add(newMascot)
                        self.mascots.push(mascot);
                    }
                }
                Err(err) => {
                    log::error!(
                        "failed to build behavior `{}` of spawned mascot: {err}",
                        request.behavior_name.as_deref().unwrap_or("(null)")
                    );
                }
            }
        }

        // Java L217-220（removed → Rust は remove_pending フラグ一括反映）。
        // 除去した mascot の除去前 index を removed_indices に記録する
        // （#10b-2b・glue の view 同期用。昇順になる）
        let mut index = 0usize;
        let mut removed = Vec::new();
        self.mascots.retain(|mascot| {
            let keep = !mascot.remove_pending();
            if !keep {
                removed.push(index);
            }
            index += 1;
            keep
        });
        self.removed_indices.append(&mut removed);

        // タスク #17: Java Mascot.getTotalCount L986-988 = manager.getCount() の
        // live 参照を再現する。spawn 反映 / 除去反映の後・全員 tick の前に
        // 各マスコットへ現在の生存数を配線し、tick 中の条件評価
        //（例: conf/behaviors.xml の `#{mascot.totalCount < 50}`）が最新値を
        // 参照するようにする（spawn で増えた子も同 tick から正しい値を持つ）。
        let total_count = self.mascots.len() as i32;
        for mascot in &mut self.mascots {
            mascot.set_total_count(total_count);
        }

        // #30 item 5: pin の真実（Environment.pinned）とミラーを同期する。
        // env.tick の auto unpin（窓クローズ）/ 保持マスコット削除 / dragging を扱う。
        self.reconcile_pin();

        // Java L223: noMascots
        let no_mascots = self.mascots.is_empty();

        // #30-8b 案A: 保持者が下端掴み行為を一度でも取った後 Fall / Thrown へ遷移したら
        // pin を解除する。解除はループ内で &mut self を呼べない（&mut self.mascots を
        // 借用中）ためローカルに要求を溜め、ループ後に反映する。
        let mut has_clung = self.pin_has_clung;
        let mut request_unpin = false;

        // Java L227-229: 全員 1 tick 進める。
        // 構築は「マスコット自身の set」の table を使う（#9b・(AF)・
        // Java buildNextBehavior は mascot 自身の Configuration で呼ばれるため）。
        if !no_mascots {
            let pin = self.environment.pinned_window();
            let env: &dyn EnvironmentView = &self.environment;
            for mascot in &mut self.mascots {
                // #30 item 2: pin 窓を activeIE として見せるのは保持者の tick 中のみ。
                // 非保持者はグローバルな active window のまま（Advisor P0-1 隔離）。
                let is_holder = pin.is_some_and(|pin| mascot.pinned_window() == Some(pin.id));
                self.environment.set_holder_scope(if is_holder {
                    pin.map(|pin| pin.holder)
                } else {
                    None
                });
                let set_name = mascot.image_set_name().to_string();
                let table = table_for(&self.set_tables, &self.table, &set_name);
                // #30-8a: 保持者のみ、mascot.tick の前に前 tick の窓矩形差分から
                // アンカーを窓下辺へ厳密追従させる（80px 超の純並進は追従しない）。
                // 適用したら delta を消し、tick 内 border_move との二重適用を防ぐ。
                if is_holder {
                    let holder_pin = pin.expect("is_holder implies pin is Some");
                    if Self::follow_pinned_window_bottom(mascot, &holder_pin) {
                        self.environment.clear_pinned_delta();
                    }
                }
                mascot.tick(env, table, self.factory.as_mut(), self.rng.as_mut());
                // #30 item 4 / #30-8b: 保持者の tick 後のみ落下 clamp を適用する。
                // 下端掴み行為を取った保持者が Fall / Thrown へ遷移した tick は
                // clamp をスキップし（アンカーを窓下辺へ引き戻さない）、解除を要求する。
                if is_holder {
                    // #30-9(C): 保持者が飛び降り系行動へ入っていたら、同系統の
                    // 「飛び降りない」行動へ差し替える（30-8b による pin 自動解除を防ぐ）。
                    if let Some(safe) = pin_safe_replacement(mascot.behavior_name()) {
                        match table.build_behavior_direct(
                            safe,
                            self.factory.as_mut(),
                            mascot.scale(),
                        ) {
                            Ok(runner) => {
                                log::info!(
                                    "pin guard: replacing jump behavior with `{safe}` (Allowed bypass)"
                                );
                                if let Err(err) = mascot.set_behavior(
                                    Some(runner),
                                    env,
                                    table,
                                    self.factory.as_mut(),
                                    self.rng.as_mut(),
                                ) {
                                    log::error!(
                                        "pin guard: failed to set behavior `{safe}`: {err}"
                                    );
                                }
                            }
                            Err(err) => {
                                log::error!("pin guard: failed to build behavior `{safe}`: {err}")
                            }
                        }
                    }
                    if is_bottom_behavior(mascot.behavior_name()) {
                        has_clung = true;
                    }
                    if has_clung && matches!(mascot.behavior_name(), Some("Fall") | Some("Thrown"))
                    {
                        request_unpin = true;
                    } else {
                        let pin = pin.expect("is_holder implies pin is Some");
                        Self::clamp_holder_to_pinned_window(
                            mascot,
                            pin,
                            table,
                            env,
                            self.factory.as_mut(),
                            self.rng.as_mut(),
                        );
                    }
                }
                self.environment.set_holder_scope(None);
            }
        }

        // #30-8b: ループ後にフラグを確定し、解除要求があれば unpin する
        //（unpin_pinned_window が pin_has_clung を false へ戻す）。
        self.pin_has_clung = has_clung;
        if request_unpin {
            self.unpin_pinned_window();
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

    /// tick の retain で除去した mascot の「除去前 index」を昇順で返し、
    /// 蓄積を空にする（drain・#10b-2b・glue の view 同期用）。
    /// 呼ぶまで tick 間で蓄積され（毎 tick リセットでない）、呼んだら空になる。
    /// dispose_all / reload 由来の除去も次 tick の retain を通って含まれる
    /// （フィールド doc を参照）。
    pub fn take_removed(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.removed_indices)
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
    /// 各マスコットは「自身の set」の table で構築する（L298
    /// getConfiguration(mascot.getImageSet()) 相当・#9b (AF)）。
    pub fn set_behavior_all(&mut self, name: &str) {
        if self.mascots.is_empty() {
            return;
        }
        let env: &dyn EnvironmentView = &self.environment;
        for mascot in &mut self.mascots {
            // Java L296: Configuration configuration =
            //   Main.getInstance().getConfiguration(mascot.getImageSet())
            let set_name = mascot.image_set_name().to_string();
            let table = table_for(&self.set_tables, &self.table, &set_name);
            match table.build_behavior(name, mascot, env, self.factory.as_mut(), self.rng.as_mut())
            {
                Ok(runner) => {
                    if let Err(err) = mascot.set_behavior(
                        Some(runner),
                        env,
                        table,
                        self.factory.as_mut(),
                        self.rng.as_mut(),
                    ) {
                        log::error!(r#"failed to set behavior "{name}": {err}"#);
                        mascot.dispose();
                    }
                }
                Err(err) => {
                    log::error!(r#"failed to build behavior "{name}": {err}"#);
                    mascot.dispose();
                }
            }
        }
    }

    /// Java 3 引数 overload `setBehaviorAll(Configuration, name, imageSet)`
    /// L320-340 逐語: 該当 set のマスコットのみ「その set」の table で構築 +
    /// setBehavior・他 set は無傷。構築 / 実行失敗（L329-334 逐語・該当 set の
    /// マスコットの catch は if の外側のため同一）→ log + そのマスコットのみ
    /// dispose（削除は次 tick）。
    pub fn set_behavior_all_of_set(&mut self, image_set_name: &str, name: &str) {
        if self.mascots.is_empty() {
            return;
        }
        let env: &dyn EnvironmentView = &self.environment;
        for mascot in &mut self.mascots {
            // Java L327: if (mascot.getImageSet().equals(imageSet))
            if mascot.image_set_name() != image_set_name {
                continue;
            }
            let table = table_for(&self.set_tables, &self.table, image_set_name);
            match table.build_behavior(name, mascot, env, self.factory.as_mut(), self.rng.as_mut())
            {
                Ok(runner) => {
                    if let Err(err) = mascot.set_behavior(
                        Some(runner),
                        env,
                        table,
                        self.factory.as_mut(),
                        self.rng.as_mut(),
                    ) {
                        log::error!(r#"failed to set behavior "{name}": {err}"#);
                        mascot.dispose();
                    }
                }
                Err(err) => {
                    log::error!(r#"failed to build behavior "{name}": {err}"#);
                    mascot.dispose();
                }
            }
        }
    }

    /// 単一マスコット版 setBehavior（#9c・Java `Mascot` popup の SetBehaviour
    /// L517-522 / L538 相当）: `index` のマスコットのみ「自分の set」の table で
    /// 構築する（[`table_for`] = Java L522 `getConfiguration(imageSet)` 相当・
    /// [`Manager::set_behavior_all`] のループ本体と同一構造）。
    /// 構築 / 実行失敗 → log + そのマスコットのみ dispose（Java
    /// `Manager.setBehaviorAll` L291-310 の catch 準拠・削除反映は次 tick）。
    /// index 範囲外 → warn + no-op（メニュー構築時と MenuEvent 時点の集合ずれ
    /// に対する防御）。
    pub fn set_behavior_at(&mut self, index: usize, name: &str) {
        let Some(mascot) = self.mascots.get_mut(index) else {
            log::warn!("set_behavior_at: ignoring out-of-range index {index}");
            return;
        };
        let env: &dyn EnvironmentView = &self.environment;
        let set_name = mascot.image_set_name().to_string();
        let table = table_for(&self.set_tables, &self.table, &set_name);
        match table.build_behavior(name, mascot, env, self.factory.as_mut(), self.rng.as_mut()) {
            Ok(runner) => {
                if let Err(err) = mascot.set_behavior(
                    Some(runner),
                    env,
                    table,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ) {
                    log::error!(r#"failed to set behavior "{name}": {err}"#);
                    mascot.dispose();
                }
            }
            Err(err) => {
                log::error!(r#"failed to build behavior "{name}": {err}"#);
                mascot.dispose();
            }
        }
    }

    /// 単一マスコット版 pause トグル（#9c・Java `Mascot` popup pauseItem
    /// L559-560 逐語 `setPaused(!isPaused())`）。index 範囲外 → warn + no-op。
    pub fn toggle_pause_at(&mut self, index: usize) {
        let Some(mascot) = self.mascots.get_mut(index) else {
            log::warn!("toggle_pause_at: ignoring out-of-range index {index}");
            return;
        };
        mascot.set_paused(!mascot.is_paused());
    }

    /// index のマスコットの pause 状態（#10b-2c・popup の「一時停止/再開」
    /// ラベル切替用の読み出し）。index 範囲外は None。
    pub fn is_paused_at(&self, index: usize) -> Option<bool> {
        self.mascots.get(index).map(Mascot::is_paused)
    }

    /// index のマスコットの画像 set 名（#10b-2c・popup 構築の
    /// [`Manager::behavior_menu_items`] 引数の取得点）。index 範囲外は None。
    pub fn image_set_name_at(&self, index: usize) -> Option<String> {
        self.mascots
            .get(index)
            .map(|mascot| mascot.image_set_name().to_string())
    }

    /// index のマスコットへマウスボタン押下を転送する（#10b-2c・
    /// Java `Main` の MouseListener → `Mascot.mousePressed` L430-447 相当。
    /// `point` は**スクリーン座標**契約（hotspot 記録用・Dragged の差分計算は
    /// Environment の cursor（スクリーン座標）を使用するため同一空間）。
    /// 左ボタン判定は呼び出し側（#10b-2c）の責務。index 範囲外 → warn + Ok。
    ///
    /// #30 item 7: 押した個体が現在の pin holder なら、掴んだ時点で即座に
    /// 最前面固定を解除する（R16「引きはがし＝解除」）。この引きはがしの解放では
    /// 再ピンしない（[`Manager::pin_pull_off`]）。
    pub fn mouse_pressed_at(
        &mut self,
        index: usize,
        point: (i32, i32),
    ) -> Result<(), BehaviorError> {
        if self.mascots.get(index).is_none() {
            log::warn!("mouse_pressed_at: ignoring out-of-range index {index}");
            return Ok(());
        }

        // pin が存在することを前提に判定する（pin 無しで `None == None` を真に
        // しないよう `is_some_and` を使う）。判定に使う `mascots[index]` の参照は
        // ここで解放する。
        let on_holder = self
            .environment
            .pinned_window()
            .is_some_and(|pin| self.mascots[index].pinned_window() == Some(pin.id));
        if on_holder {
            self.unpin_pinned_window();
        }
        self.pin_pull_off = on_holder;

        let mascot = &mut self.mascots[index];
        let env: &dyn EnvironmentView = &self.environment;
        let set_name = mascot.image_set_name().to_string();
        let table = table_for(&self.set_tables, &self.table, &set_name);
        mascot.mouse_pressed(point, env, table, self.factory.as_mut(), self.rng.as_mut())
    }

    /// index のマスコットへマウスボタン解放を転送する（#10b-2c・
    /// Java `Mascot.mouseReleased` L455-471 相当）。index 範囲外 → warn + Ok。
    ///
    /// #30 item 4: `point`（スクリーン座標・解放点）を追加。トグル ON かつ解放点
    /// 直下に窓 W があれば `mascot.mouse_released(...)` の**前に** W を holder
    /// `index` 専用に最前面固定する（固定成功時のみ当該 Mascot のミラーを
    /// `Some(W.id)` にする。OFF / 窓なし / 固定失敗は既存 Thrown を維持）。
    ///
    /// #30 item 7: 直前の押下が保持者の引きはがし（[`Manager::pin_pull_off`]）なら
    /// この解放では再ピンせず、フラグを消費するだけにする。
    pub fn mouse_released_at(
        &mut self,
        index: usize,
        point: (i32, i32),
    ) -> Result<(), BehaviorError> {
        if self.mascots.get(index).is_none() {
            log::warn!("mouse_released_at: ignoring out-of-range index {index}");
            return Ok(());
        }

        if self.pin_pull_off {
            self.pin_pull_off = false;
        } else {
            self.pin_dropped_window_at(index, point);
        }

        let mascot = &mut self.mascots[index];
        let env: &dyn EnvironmentView = &self.environment;
        let set_name = mascot.image_set_name().to_string();
        let table = table_for(&self.set_tables, &self.table, &set_name);
        mascot.mouse_released(env, table, self.factory.as_mut(), self.rng.as_mut())
    }

    /// #30 item 4: ドロップ点直下の窓 W を holder `index` 専用に最前面固定する。
    /// トグル OFF・窓なし・固定失敗では何もしない。成功時は単一 holder 方針に従い
    /// 既存ミラーを全クリアして当該 Mascot のみ `Some(W.id)` にする。
    fn pin_dropped_window_at(&mut self, index: usize, point: (i32, i32)) {
        if !self.pin_dropped_window_allowed {
            return;
        }
        let Some((id, _rect)) = self.environment.window_at_point(point.0, point.1) else {
            return;
        };
        if !self.environment.pin_window(id, index) {
            return;
        }
        // #30-8b: pin 成立時に「しがみつき済み」をリセットする（新規 pin への持ち越し防止）。
        self.pin_has_clung = false;
        for mascot in &mut self.mascots {
            mascot.set_pinned_window(None);
        }
        if let Some(mascot) = self.mascots.get_mut(index) {
            mascot.set_pinned_window(Some(id));
        }
        // プロセス異常終了（panic）時に WS_EX_TOPMOST を best-effort で剥がすための
        // 復元ターゲットを登録する。元から TOPMOST だった窓（was_topmost == true）は
        // 我々が付けたのではないため対象外（None 登録 = 解除）。
        let panic_target = self
            .environment
            .pinned_window()
            .filter(|pin| !pin.was_topmost)
            .map(|pin| pin.id);
        crate::win::os_source::set_panic_unpin_window(panic_target);
    }

    /// #30 item 1/4: トレイ「Allowed Behaviours」の pin トグルを設定する。
    /// OFF 時は即 unpin + 全ミラークリア（design item 4/5）。
    pub fn set_pin_dropped_window_allowed(&mut self, allowed: bool) {
        self.pin_dropped_window_allowed = allowed;
        if !allowed {
            self.unpin_pinned_window();
        }
    }

    /// #30 item 5: pin を解除し、全マスコットのミラーをクリアする
    /// （トグル OFF / Reload / RestoreWindows / DismissAll の解除フック・30-5 が使う）。
    pub fn unpin_pinned_window(&mut self) {
        self.environment.unpin_window();
        for mascot in &mut self.mascots {
            mascot.set_pinned_window(None);
        }
        // #30-8b: 解除時に「しがみつき済み」を持ち越さない。
        self.pin_has_clung = false;
        // panic 復元ターゲットも解除する（#30-5）。
        crate::win::os_source::set_panic_unpin_window(None);
    }

    /// #30 item 6: 現在 pin を保持しているマスコットの index を返す
    /// （pin が無ければ `None`）。
    ///
    /// pin の真実（[`Environment::pinned_window`]）と各 Mascot のミラー
    /// （[`Mascot::pinned_window`]）を突き合わせ、ミラーが pin 窓 id と一致する
    /// マスコットの**現在の** index を探索して返す。`PinState.holder` の保存値は
    /// 使わない（削除で index がずれても追随できないため）。
    /// `src/main.rs` がピン保持中に保持マスコット窓をピン対象窓より前面へ再アサート
    /// する対象特定に使う。pin が無い間は `None` を返すため新規コストはゼロ。
    pub fn pinned_holder(&self) -> Option<usize> {
        let pin = self.environment.pinned_window()?;
        self.mascots
            .iter()
            .position(|mascot| mascot.pinned_window() == Some(pin.id))
    }

    /// #30 item 5: pin の真実（`Environment.pinned`）とミラーを同期する。
    /// - pin なし: ミラーを全クリア（env.tick の `window_frame` None による auto unpin）
    /// - 保持マスコットが見つからない（削除済み）/ `dragging`（引きはがし）:
    ///   unpin + 全ミラークリア
    /// - 有効: 保持者以外の残存ミラーをクリア
    ///
    /// 保持者特定は index ではなくミラー（`pinned_window == pin.id`）で行うため、
    /// 削除による index ずれでも gating に渡す `pin.holder` と整合する。
    fn reconcile_pin(&mut self) {
        let Some(pin) = self.environment.pinned_window() else {
            for mascot in &mut self.mascots {
                mascot.set_pinned_window(None);
            }
            return;
        };
        let holder = self
            .mascots
            .iter()
            .position(|mascot| mascot.pinned_window() == Some(pin.id));
        let invalid = holder.is_none_or(|index| self.mascots[index].is_dragging());
        if invalid {
            self.environment.unpin_window();
            for mascot in &mut self.mascots {
                mascot.set_pinned_window(None);
            }
            return;
        }
        let holder = holder.expect("holder is Some when not invalid");
        for (index, mascot) in self.mascots.iter_mut().enumerate() {
            if index != holder && mascot.pinned_window() == Some(pin.id) {
                mascot.set_pinned_window(None);
            }
        }
    }

    /// #30-8a: 保持マスコットのアンカーを、pin 窓の前 tick からの矩形差分から
    /// 新しい窓下辺へ厳密追従させる。適用したら true（呼び出し側が
    /// [`Environment::clear_pinned_delta`] を呼ぶ）。
    ///
    /// 追従条件・規則（design §1.10(z) 追補 30-8a）:
    /// - アンカーが前 tick の窓下辺上（`y == old_bottom` かつ
    ///   `x ∈ [old_left, old_right]`）のときのみ。窓側面を登る局面（`y < bottom`）は
    ///   対象外（登りを妨げない）。
    /// - サイズ変化（`dleft != dright || dtop != dbottom`）は 80px しきい値の対象外で
    ///   常に追従する。
    /// - 純並進で 1 tick の最大変位が 80px 超なら追従しない（ユーザー承認 案Y・
    ///   既存の防ジャンプガードへ委譲し LostGround → Fall させる）。
    /// - `y` は新 bottom に、`x` は窓の水平移動に比例（前幅 0 の縮退ではゼロ除算しない）。
    fn follow_pinned_window_bottom(mascot: &mut Mascot, pin: &PinnedWindow) -> bool {
        let (x, y) = mascot.anchor();
        let window = pin.rect;

        // 前 tick の矩形（現在値 - delta）。
        let old_left = window.left - pin.dleft;
        let old_right = window.right - pin.dright;
        let old_bottom = window.bottom - pin.dbottom;

        // 前 tick の窓下辺上に無いアンカーは追従しない。
        if y != old_bottom || x < old_left || x > old_right {
            return false;
        }

        let pure_translation = pin.dleft == pin.dright && pin.dtop == pin.dbottom;
        if pure_translation && (pin.dleft.abs() > 80 || pin.dtop.abs() > 80) {
            return false; // 案Y: 速い純並進は追従せず既存ガードに委ねる
        }

        // 水平は窓幅の比例で厳密化（前幅 0 の縮退時はゼロ除算回避）。
        let old_width = old_right - old_left;
        let new_x = if old_width == 0 {
            x + pin.dleft
        } else {
            (x - old_left) * window.width() / old_width + window.left
        };
        mascot.set_anchor((new_x, window.bottom));
        true
    }

    /// #30 item 4: 保持マスコットの落下 clamp。tick 後、anchor が pin 窓 W の水平
    /// 範囲内かつ下端以深（`anchor.y >= W.bottom`）なら `(anchor.x, W.bottom)` に補正し、
    /// 水平位置に応じた下端掴み行為へ強制遷移する。Allowed 判定は意図的に bypass する
    /// （明示ユーザー操作・design item 4）。既に下端掴み 3 種なら再遷移しない。
    fn clamp_holder_to_pinned_window(
        mascot: &mut Mascot,
        pin: PinnedWindow,
        table: &BehaviorTable,
        env: &dyn EnvironmentView,
        factory: &mut dyn BehaviorFactory,
        rng: &mut dyn Rng,
    ) {
        let (x, y) = mascot.anchor();
        let window = pin.rect;
        if x < window.left || x > window.right || y < window.bottom {
            return;
        }
        if is_bottom_behavior(mascot.behavior_name()) {
            return;
        }
        mascot.set_anchor((x, window.bottom));
        let midpoint = window.left + (window.right - window.left) / 2;
        let name = if x < midpoint {
            "GrabIEBottomLeftWall"
        } else if x > midpoint {
            "GrabIEBottomRightWall"
        } else {
            "ClimbIEBottom"
        };
        match table.build_behavior_direct(name, factory, mascot.scale()) {
            Ok(runner) => {
                log::info!("pin clamp: forcing behavior `{name}` (Allowed bypass)");
                if let Err(err) = mascot.set_behavior(Some(runner), env, table, factory, rng) {
                    log::error!("pin clamp: failed to set behavior `{name}`: {err}");
                }
            }
            Err(err) => log::error!("pin clamp: failed to build behavior `{name}`: {err}"),
        }
    }

    /// index のマスコットのカーソル位置を更新する（#10b-2c・
    /// Java `Mascot.setCursorPosition` L1320-1332 相当・CursorMoved 経路。
    /// `point` (= Some) はスクリーン座標契約）。index 範囲外 → warn + no-op。
    pub fn set_cursor_position_at(&mut self, index: usize, point: Option<(i32, i32)>) {
        let Some(mascot) = self.mascots.get_mut(index) else {
            log::warn!("set_cursor_position_at: ignoring out-of-range index {index}");
            return;
        };
        mascot.set_cursor_position(point);
    }

    /// 単一マスコット版 Dismiss（#9c・Java `Mascot` popup disposeMenu L562-563 逐語
    /// `dispose()`）。remove_pending を立てるのみ・削除反映は次 tick。
    /// index 範囲外 → warn + no-op。
    pub fn dismiss_at(&mut self, index: usize) {
        let Some(mascot) = self.mascots.get_mut(index) else {
            log::warn!("dismiss_at: ignoring out-of-range index {index}");
            return;
        };
        mascot.dispose();
    }

    /// Reload（タスク #9d）: 全マスコットの画像セット参照付け替え + 行動表の全入れ替え。
    ///
    /// Java `Main.reloadAllImageSets`（Main.java L547-566）は「全消し + 再作成」だが、
    /// design.md §2 Reload 方針（§1.10 (d) 9d・ユーザー承認）により**意図的差異**として
    /// 参照付け替え路線を採用する: 存続マスコットの anchor / look_right / paused /
    /// dragging は維持し、ImageSet Arc と行動表だけを差し替える。
    ///
    /// - 空 materials: [`Manager::dispose_all`]（remove_pending・削除は次 tick）+
    ///   set_tables クリア・base table は変更しない
    /// - 非空 materials:
    ///   1. base table を materials[0]（既定 set = 辞書順先頭）の table で置換し、
    ///      set_tables を全消しの上で全 materials 分を再登録する（既定 set 分は
    ///      clone して base と set_tables の両方へ・stale エントリは残らない）
    ///   2. マスコットを **index 順** で処理する: 自 set が materials に残存 →
    ///      自 set 名のまま新 [`ImageSet`](crate::render::imageset::ImageSet) オブジェクトへ
    ///      [`Mascot::rebind_image_set`](crate::mascot::Mascot::rebind_image_set) /
    ///      消滅 → materials[0] の set 名・image_set へ付け替え
    ///      （design §2「Reload 時に既存 ImageSet 参照」行相当）
    ///   3. behavior 再構築: 現在の behavior 名が「付け替え後 set の table」に存在かつ
    ///      enabled（[`BehaviorTable::is_behavior_enabled`] の同一式で pre-check）なら
    ///      同名再構築（pre-check 済みのため build_behavior の再配置分岐は不通 =
    ///      同名経路では rng を消費しない）。存在しない / 無効 / behavior 無しは
    ///      [`BehaviorTable::build_next_behavior`]（previous None = Java createMascot
    ///      L493 の buildNextBehavior(null) と同一経路）で再選択。どちらも新 runner
    ///      で進行をリセットする（anchor / look_right / paused / dragging 維持）
    ///   4. 構築 Err / set_behavior Err → log + そのマスコットのみ dispose
    ///      （Java setBehaviorAll L291-340 の catch 部相当・他は無傷）
    ///
    /// Environment の disabled map（Allowed Behaviours）・image set resolver・
    /// added キューは一切変更しない。
    pub fn reload(&mut self, materials: Vec<ReloadMaterial>) {
        // #30 item 5: 資産差し替えで旧 pin（TOPMOST）を残さない。起動時 reload は
        // pin 無しのため no-op。
        self.unpin_pinned_window();

        if materials.is_empty() {
            // 空: 全員 dispose 扱い（削除は次 tick の retain）+ set_tables クリア +
            // base table 変更なし
            self.dispose_all();
            self.set_tables.clear();
            return;
        }

        // rebind 用の set 名 → 新 Arc 対応（materials は table 抽出で消費するため
        // 先に作る。残存 set も新オブジェクトへ付け替える）
        let new_sets: HashMap<String, Arc<ImageSet>> = materials
            .iter()
            .map(|m| (m.name.clone(), Arc::clone(&m.image_set)))
            .collect();
        // 既定 set（materials[0] = 辞書順先頭）の情報（消滅 set フォールバック用）
        let first_name = materials[0].name.clone();
        let first_image_set = Arc::clone(&materials[0].image_set);

        // 1. base table を materials[0] の table で置換 + set_tables 全消し再登録
        //（既定 set 分は clone して base と set_tables の両方へ）
        let mut materials = materials.into_iter();
        let first = materials
            .next()
            .expect("materials is non-empty (checked above)");
        self.set_tables.clear();
        self.set_tables.insert(first.name, first.table.clone());
        self.table = first.table;
        for material in materials {
            self.set_tables.insert(material.name, material.table);
        }

        // 2-4. index 順で走査（rng 消費順が index 順に依存するため固定）
        let env: &dyn EnvironmentView = &self.environment;
        for mascot in &mut self.mascots {
            // 2. 自 set の扱い決定（残存 = 新 Arc へ付け替え / 消滅 = 既定 set へ）
            let own_set = mascot.image_set_name().to_string();
            match new_sets.get(&own_set) {
                Some(new_arc) => mascot.rebind_image_set(own_set.clone(), Arc::clone(new_arc)),
                None => mascot.rebind_image_set(first_name.clone(), Arc::clone(&first_image_set)),
            }
            let set_name = mascot.image_set_name().to_string();

            // 3. behavior 再構築（判定は「付け替え後 set」の table で行う）
            let table = table_for(&self.set_tables, &self.table, &set_name);
            let current_name = mascot.behavior_name().map(str::to_string);
            let same_name_enabled = match &current_name {
                Some(name) => table
                    .find(name)
                    .is_some_and(|row| BehaviorTable::is_behavior_enabled(row, &set_name, env)),
                None => false,
            };
            let built = match (&current_name, same_name_enabled) {
                (Some(name), true) => table.build_behavior(
                    name,
                    mascot,
                    env,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ),
                _ => table.build_next_behavior(
                    None,
                    mascot,
                    env,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ),
            };

            // 4. 構築 Err / set_behavior Err → そのマスコットのみ dispose（他は無傷）
            match built {
                Ok(runner) => {
                    let runner_name = runner.name.clone();
                    if let Err(err) = mascot.set_behavior(
                        Some(runner),
                        env,
                        table,
                        self.factory.as_mut(),
                        self.rng.as_mut(),
                    ) {
                        log::error!(
                            r#"failed to set behavior "{runner_name}" after reload: {err}"#
                        );
                        mascot.dispose();
                    }
                }
                Err(err) => {
                    log::error!("failed to build behavior after reload: {err}");
                    mascot.dispose();
                }
            }
        }
    }

    /// Java L133-135 逐語（既定 true・L72）。
    pub fn set_exit_on_last_removed(&mut self, exit_on_last_removed: bool) {
        self.exit_on_last_removed = exit_on_last_removed;
    }

    /// Allowed Behaviours トグルの passthrough（Main.setMascotBehaviorEnabled
    /// L526-544 逐語のリスト変異は [`Environment::set_behavior_enabled`]・#9b）。
    pub fn set_behavior_enabled(&mut self, image_set: &str, name: &str, enabled: bool) {
        self.environment
            .set_behavior_enabled(image_set, name, enabled);
    }

    /// Allowed Settings passthrough 5 種（#9c・Settings.java L32-37 / L89-94 相当）。
    /// [`Environment`] の同名 setter 群への委譲。`sounds` は Environment setter が
    /// 存在しないため passthrough を作らない（Phase 1 no-op・design §3-12・
    /// トレイ側は settings 永続化のみ）。
    pub fn set_breeding_allowed(&mut self, allowed: bool) {
        self.environment.set_breeding_allowed(allowed);
    }

    /// [`Environment::set_transients_enabled`] への委譲（#9c）。
    pub fn set_transients_enabled(&mut self, enabled: bool) {
        self.environment.set_transients_enabled(enabled);
    }

    /// [`Environment::set_transformation_allowed`] への委譲（#9c）。
    pub fn set_transformation_allowed(&mut self, allowed: bool) {
        self.environment.set_transformation_allowed(allowed);
    }

    /// [`Environment::set_throwing_allowed`] への委譲（#9c）。
    pub fn set_throwing_allowed(&mut self, allowed: bool) {
        self.environment.set_throwing_allowed(allowed);
    }

    /// [`Environment::set_multiscreen`] への委譲（#9c）。
    pub fn set_multiscreen(&mut self, multiscreen: bool) {
        self.environment.set_multiscreen(multiscreen);
    }

    /// 無効行動 map の全体置換 passthrough（#10b-2c・settings.toml 復元注入の
    /// 起動時適用経路）。
    /// [`Environment::set_disabled_behaviors`] への委譲（[`EnvironmentView`] には
    /// 置かれていないため index 系 passthrough と同様の Manager 経由とする）。
    pub fn set_disabled_behaviors(&mut self, disabled: BTreeMap<String, Vec<String>>) {
        self.environment.set_disabled_behaviors(disabled);
    }

    /// 画面外の窓を作業領域へ戻す（WindowsEnvironment.restoreWindows L292-347
    /// の実装は [`Environment`]・tray RestoreWindows の供給経路・#9b）。
    pub fn restore_windows(&mut self) {
        self.environment.restore_windows();
    }

    /// マスコット右クリック メニューの行動分類（Java `Mascot` ポップアップ
    /// L523-553 逐語相当・#9b）。要求 set（Java `getConfiguration(imageSet)` 相当・
    /// 未知 set は base table で動作）の table を挿入順で走査する:
    /// - hidden → 完全スキップ（L526）
    /// - 名前に "/" を含む → 完全スキップ（L529 / L548 の contains("/") 否定）
    /// - 有効な非 toggleable → selectable のみ（L529-547）
    /// - toggleable → toggleable に (name, checked = enabled) 追加（L549-556）かつ
    ///   有効なら selectable にも（L529 の behaviorEnabled && !contains("/")）
    /// - frequency は参照しない（Java も参照しない）
    ///
    /// 無効判定は [`BehaviorTable::is_behavior_enabled`] の同一式（Java L583-588
    /// 短絡: 非 toggleable は常に有効 = 「無効な非 toggleable」は到達不能）。
    pub fn behavior_menu_items(&self, image_set_name: &str) -> BehaviorMenu {
        let table = table_for(&self.set_tables, &self.table, image_set_name);
        let env: &dyn EnvironmentView = &self.environment;
        let mut menu = BehaviorMenu {
            selectable: Vec::new(),
            toggleable: Vec::new(),
        };
        for row in &table.rows {
            // Java L526: if (!config.isBehaviorHidden(behaviorName))
            if row.hidden || row.name.contains('/') {
                continue;
            }
            // Java L528: boolean behaviorEnabled =
            //   config.isBehaviorEnabled(behaviorName, this)
            let enabled = BehaviorTable::is_behavior_enabled(row, image_set_name, env);
            if !row.toggleable {
                // Java L529-547: behaviorEnabled && !contains("/") → setBehaviorMenu
                if enabled {
                    menu.selectable.push(row.name.clone());
                }
            } else {
                // Java L549-556: isBehaviorToggleable → allowedBehaviorsMenu に
                // (displayName, behaviorEnabled) で追加
                menu.toggleable.push((row.name.clone(), enabled));
                // L529: toggleable && 有効 は selectable にも出る
                if enabled {
                    menu.selectable.push(row.name.clone());
                }
            }
        }
        menu
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
