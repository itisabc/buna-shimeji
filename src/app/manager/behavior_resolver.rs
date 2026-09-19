//! Manager の BehaviorTable 解決と setBehavior 適用（design-review #1 の分割）。
//!
//! Java `Manager.setBehaviorAll` L291-340 の構築・適用部分を担当する。table の解決は
//! [`table_for`]（Java `Main.getConfiguration(imageSet)` 相当・#9b (AF)）で行い、
//! table 選択規則は `manager::menu` / `manager::post_tick` / `manager.rs` と共有する。

use std::collections::HashMap;

use super::Manager;
use crate::mascot::behavior::BehaviorTable;
use crate::mascot::EnvironmentView;

/// 「要求 set の table を選ぶ」共通ヘルパ（Java `getConfiguration(imageSet)` 相当）。
/// field-disjoint borrow で呼ぶため静的ヘルパにする（`&self` を取らない）。
pub(super) fn table_for<'a>(
    set_tables: &'a HashMap<String, BehaviorTable>,
    base: &'a BehaviorTable,
    image_set_name: &str,
) -> &'a BehaviorTable {
    set_tables.get(image_set_name).unwrap_or(base)
}

impl Manager {
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
}
