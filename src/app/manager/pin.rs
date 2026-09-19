//! Manager のピン（ドロップした窓の最前面固定）状態機械（design-review #1 の分割）。
//!
//! Java に対応クラスは無く、#30 の追加機能（design §1.10 (z) 追補 30-4〜30-13）。
//! pin の真実は `Environment.pinned` が持ち、Manager は各 Mascot のミラーを同期して
//! holder を特定する（[`Manager::unpin_pinned_window`] / [`Manager::pinned_holder`] /
//! [`Manager::reconcile_pin`]）。保持者の落下 clamp と窓追従は
//! [`Manager::clamp_holder_to_pinned_window`] / [`Manager::follow_pinned_window_bottom`]。

use super::Manager;
use crate::app::environment::PinnedWindow;
use crate::mascot::behavior::{BehaviorFactory, BehaviorTable};
use crate::mascot::{EnvironmentView, Mascot, Rng};

/// #30 item 4: 下端掴み 3 種（behaviors.xml L138-143）かどうか。
/// 保持者の落下 clamp で「既に下端掴みなら再遷移しない」判定に使う。
pub(super) fn is_bottom_behavior(name: Option<&str>) -> bool {
    matches!(
        name,
        Some("ClimbIEBottom") | Some("GrabIEBottomLeftWall") | Some("GrabIEBottomRightWall")
    )
}

/// タスク #30-9(C): ピン保持中に飛び降り系行動（窓外へ落下し 30-8b で pin を
/// 解除してしまう）を、同系統の「飛び降りない」行動へ差し替える写像。
/// 対象外（縁伝い・下端掴み・安全行動自身など）は None を返す（冪等）。
pub(super) fn pin_safe_replacement(name: Option<&str>) -> Option<&'static str> {
    match name {
        Some("JumpFromLeftEdgeOfIE") => Some("SitOnTheLeftEdgeOfIE"),
        Some("JumpFromRightEdgeOfIE") => Some("SitOnTheRightEdgeOfIE"),
        Some("WalkLeftAlongIEAndJump") => Some("WalkLeftAlongIEAndSit"),
        Some("WalkRightAlongIEAndJump") => Some("WalkRightAlongIEAndSit"),
        _ => None,
    }
}

impl Manager {
    /// #30 item 4: ドロップ点直下の窓 W を holder `index` 専用に最前面固定する。
    /// トグル OFF・窓なし・固定失敗では何もしない。成功時は単一 holder 方針に従い
    /// 既存ミラーを全クリアして当該 Mascot のみ `Some(W.id)` にする。
    pub(super) fn pin_dropped_window_at(&mut self, index: usize, point: (i32, i32)) {
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
    pub(super) fn reconcile_pin(&mut self) {
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
    pub(super) fn follow_pinned_window_bottom(mascot: &mut Mascot, pin: &PinnedWindow) -> bool {
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
    pub(super) fn clamp_holder_to_pinned_window(
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
        match table.build_behavior_direct(name, factory, mascot) {
            Ok(runner) => {
                log::info!("pin clamp: forcing behavior `{name}` (Allowed bypass)");
                if let Err(err) = mascot.set_behavior(Some(runner), env, table, factory, rng) {
                    log::error!("pin clamp: failed to set behavior `{name}`: {err}");
                }
            }
            Err(err) => log::error!("pin clamp: failed to build behavior `{name}`: {err}"),
        }
    }
}
