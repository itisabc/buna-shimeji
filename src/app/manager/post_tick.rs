//! Manager の tick 後処理（design-review #1 の分割）。
//!
//! ScanMove の到達時要求（自分 / 相手の Behavior 差し替え・向き反転）と、
//! Transform の変身要求（画像セット差し替え + 変身先 Behavior 構築）を、
//! 個体ループ後に一括適用する。Java は action 内で即時適用するが、Rust の action は
//! 他個体・resolver に触れないため要求を溜める（意図的差異・design §1.10 (z-6)/(z-7)）。

use super::behavior_resolver::table_for;
use super::Manager;
use crate::mascot::{
    AffordanceArrival, AffordanceScanEntry, EnvironmentView, Mascot, TransformRequest,
};

impl Manager {
    /// ScanMove 用スナップショットを組む（#32）。index 順 = Java
    /// `Manager.getMascotWithAffordance` の線形走査順（同順で最初の一致が選ばれる）。
    ///
    /// 放送中の個体（affordance 非空）が 1 体も居なければ空を返す（コスト削減・
    /// red-team R1）。空は「全個体を再構築した場合」と内容が等価（全 entry が
    /// affordance 空になり、スキャン側の照合はどのみち一致しない）ため、
    /// 意味を変えずに clone / String alloc を丸ごと省ける。同梱 `conf/` のように
    /// ScanMove を使わない資産では常にこの短絡経路を通る。
    pub(super) fn scan_snapshot(mascots: &[Mascot]) -> Vec<AffordanceScanEntry> {
        if !mascots
            .iter()
            .any(|mascot| !mascot.affordances().is_empty())
        {
            return Vec::new();
        }
        mascots
            .iter()
            .enumerate()
            .map(|(index, mascot)| AffordanceScanEntry {
                index,
                anchor: mascot.anchor(),
                affordances: mascot.affordances().to_vec(),
            })
            .collect()
    }

    /// ScanMove の到達時要求を Java `ScanMove.tick` L122-137 と同じ順序で適用する:
    /// 1. 自分の Behavior（L125）— 失敗したら相手には触らない（Java の catch 位置と同じ）
    /// 2. 相手の Behavior（L127）
    /// 3. `TargetLook` かつ両者の向きが同じなら相手を反転（L128-130）
    ///
    /// index は要求時点のスナップショット値だが、tick 内での削除はループ前のみ・
    /// spawn は次 tick 反映のため、ループ直後の本適用まで index は安定する。
    pub(super) fn apply_affordance_arrival(&mut self, index: usize, arrival: AffordanceArrival) {
        let env: &dyn EnvironmentView = &self.environment;

        // 1. 自分の Behavior（自分の set の table で構築・Java L125）
        let Some(set_name) = self
            .mascots
            .get(index)
            .map(|mascot| mascot.image_set_name().to_string())
        else {
            return;
        };
        let table = table_for(&self.set_tables, &self.table, &set_name);
        let built = table.build_behavior(
            &arrival.behavior,
            &mut self.mascots[index],
            env,
            self.factory.as_mut(),
            self.rng.as_mut(),
        );
        match built {
            Ok(runner) => {
                if let Err(err) = self.mascots[index].set_behavior(
                    Some(runner),
                    env,
                    table,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ) {
                    log::error!(
                        r#"scan arrival: failed to set behavior "{}" for mascot #{index}: {err}"#,
                        arrival.behavior
                    );
                    return;
                }
            }
            Err(err) => {
                log::error!(
                    r#"scan arrival: failed to build behavior "{}" for mascot #{index}: {err}"#,
                    arrival.behavior
                );
                return;
            }
        }

        // 2-3. 相手の Behavior + 向き反転（相手の set の table で構築・Java L127-130）
        let Some(target_index) = arrival.target_index else {
            return;
        };
        let Some(target_set) = self
            .mascots
            .get(target_index)
            .map(|mascot| mascot.image_set_name().to_string())
        else {
            return;
        };
        let target_table = table_for(&self.set_tables, &self.table, &target_set);
        let built = target_table.build_behavior(
            &arrival.target_behavior,
            &mut self.mascots[target_index],
            env,
            self.factory.as_mut(),
            self.rng.as_mut(),
        );
        match built {
            Ok(runner) => {
                if let Err(err) = self.mascots[target_index].set_behavior(
                    Some(runner),
                    env,
                    target_table,
                    self.factory.as_mut(),
                    self.rng.as_mut(),
                ) {
                    log::error!(
                        r#"scan arrival: failed to set behavior "{}" for mascot #{target_index}: {err}"#,
                        arrival.target_behavior
                    );
                    return;
                }
                // Java L128-130: 自分と相手の向きが同じときだけ相手を反転する
                let mine = self.mascots[index].look_right();
                if arrival.flip_look && self.mascots[target_index].look_right() == mine {
                    self.mascots[target_index].set_look_right(!mine);
                }
            }
            Err(err) => {
                log::error!(
                    r#"scan arrival: failed to build behavior "{}" for mascot #{target_index}: {err}"#,
                    arrival.target_behavior
                );
            }
        }
    }

    /// Transform の変身要求を適用する（Java `Transform.transform` L44-54 相当・#33）:
    /// 1. 変身先 set を決める（Java L45: `configuration(TransformMascot) != null ?
    ///    TransformMascot : mascot.getImageSet()`）。空 / 未解決は自分の set のまま。
    /// 2. 解決できた場合のみ [`Mascot::rebind_image_set`] で画像セットを差し替える
    ///    （Java の `setImageSet` は `buildBehavior` より先）。
    /// 3. 差し替え後 set の table で `TransformBehavior` を構築して `setBehavior`
    ///    （Java L49）。
    ///
    /// 構築失敗は log + 現状の Behavior 維持（マスコットは生存・Java L50-53 の
    /// catch + showError 相当）。画像セットは Java 同様に差し替え済みのまま残す。
    pub(super) fn apply_transform(&mut self, index: usize, request: TransformRequest) {
        let env: &dyn EnvironmentView = &self.environment;
        let own_set = self.mascots[index].image_set_name().to_string();

        // 1. 変身先 set の決定（空 / resolver 未解決は自分の set）
        let resolved = if request.image_set.is_empty() || request.image_set == own_set {
            None
        } else {
            self.resolver.as_mut().and_then(|r| r(&request.image_set))
        };
        let (target_name, target_image_set) = match resolved {
            Some(arc) => (request.image_set.clone(), Some(arc)),
            None if request.image_set.is_empty() || request.image_set == own_set => {
                (own_set.clone(), None)
            }
            None => {
                // Java: configuration(TransformMascot) == null → 自分の set を使う
                log::warn!(
                    "transform: could not resolve image set `{}` for mascot #{index}; keeping `{own_set}`",
                    request.image_set
                );
                (own_set.clone(), None)
            }
        };

        // 2. 画像セット差し替え（解決できた場合のみ）
        if let Some(arc) = target_image_set {
            self.mascots[index].rebind_image_set(target_name.clone(), arc);
        }

        // 3. 変身先 set の table で Behavior を構築して設定
        let table = table_for(&self.set_tables, &self.table, &target_name);
        let built = table.build_behavior(
            &request.behavior,
            &mut self.mascots[index],
            env,
            self.factory.as_mut(),
            self.rng.as_mut(),
        );
        let runner = match built {
            Ok(runner) => runner,
            Err(err) => {
                log::error!(
                    r#"transform: failed to build behavior "{}" for mascot #{index}: {err}"#,
                    request.behavior
                );
                return;
            }
        };
        if let Err(err) = self.mascots[index].set_behavior(
            Some(runner),
            env,
            table,
            self.factory.as_mut(),
            self.rng.as_mut(),
        ) {
            log::error!(
                r#"transform: failed to set behavior "{}" for mascot #{index}: {err}"#,
                request.behavior
            );
        }
    }
}
