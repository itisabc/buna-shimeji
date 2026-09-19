//! Manager のマスコット右クリック メニュー分類（design-review #1 の分割）。
//!
//! Java `Mascot` ポップアップ L523-553 逐語相当（#9b）。`BehaviorTable` の行を
//! 挿入順に分類する（[`Manager::behavior_menu_items`]）。

use super::behavior_resolver::table_for;
use super::Manager;
use crate::mascot::behavior::BehaviorTable;
use crate::mascot::EnvironmentView;

/// マスコット右クリック メニューの行動分類（Java `Mascot` ポップアップ
/// L523-553 相当・#9b）。双方とも table 挿入順。
pub struct BehaviorMenu {
    /// 有効 && 名前に "/" を含まない非 toggleable 行動（setBehavior メニュー項目）。
    /// toggleable 且つ有効な行動も Java 同様 selectable に出る（L529-547 逐語）。
    pub selectable: Vec<String>,
    /// Allowed Behaviours トグル項目（名前, checked = enabled = 無効リスト非含有）。
    pub toggleable: Vec<(String, bool)>,
}

impl Manager {
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
}
