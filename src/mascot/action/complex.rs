//! ComplexAction 系（Sequence / Select）— design §1.8・Java 正本
//! .tmp/java-ref/action/{ComplexAction,Sequence,Select}.java。
//!
//! Java `ComplexAction`（複数アクションを直列実行する基底クラス）を
//! 「子アクション Box 配列 + currentAction インデックス」で再現する。
//! Sequence のみ:
//! - `setCurrentAction` を
//!   `is_loop ? currentAction % getActions().length : currentAction` のラップで
//!   オーバーライド（Java Sequence.java L34-36 逐語・is_loop = 資産 55 箇所）
//! - `hasNext` を「毎回 seek してから super.hasNext」でオーバーライド
//!   （Java Sequence.java L26-31 逐語）
//!
//! Select はどちらもオーバーライドなし（Java Select.java L15-21 → 基底の
//!   hasNext / setCurrentAction のまま・seek は init の 1 回のみ）。
//!
//! 子アクションの構築（Java ActionBuilder.createActions L445-455）は
//! mod.rs::build_def が担当する（Ref 子 = Ref 側 attrs 優先マージ /
//! Inline 子 = 空パラメータ）。

use super::super::behavior::ActionError;
use super::super::{EnvironmentView, Mascot, Rng};
use super::Base;
use crate::mascot::behavior::Action;

/// Java `ComplexAction` / `Sequence` / `Select` 相当。
///（Select は ComplexAction そのもの・Java Select.java L15-21）
pub(crate) struct Complex {
    /// アニメは持たない（Java `super(schema, List.of(), context)` L26 逐語）。
    pub base: Base,
    actions: Vec<Box<dyn Action>>,
    current_action: i32,
    /// Java `Sequence` 型か（Select は hasNext をオーバーライドしないため
    /// has_next での seek 呼び出しを分岐するためのフラグ）。
    is_sequence: bool,
    /// Loop 属性（Java Sequence.java L19-20・isLoop()）。
    is_loop: bool,
}

impl Complex {
    pub(crate) fn new(
        attrs: crate::config::VarMap,
        actions: Vec<Box<dyn Action>>,
        is_sequence: bool,
        is_loop: bool,
    ) -> Complex {
        Complex {
            base: Base::new(attrs, Vec::new()),
            actions,
            current_action: 0,
            is_sequence,
            is_loop,
        }
    }

    /// Java ComplexAction.setCurrentAction L80-87 + Sequence オーバーライド
    /// L34-36 逐語。
    fn set_current_action(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        value: i32,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        let len = self.actions.len();
        self.current_action = if self.is_loop {
            // Java: currentAction % getActions().length（int `%`・剰余は非負
            //（currentAction が順増加のため））
            value % len as i32
        } else {
            value
        };
        // Java L82-86: base hasNext の間、範囲内の子を init する
        if self.base.base_has_next(mascot, env)? && (self.current_action as usize) < len {
            let index = self.current_action as usize;
            self.actions[index].init(mascot, env, rng)?;
        }
        Ok(())
    }

    /// Java ComplexAction.seek L44-53 逐語。
    pub(crate) fn seek(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        if self.base.base_has_next(mascot, env)? {
            while self.current_action < self.actions.len() as i32 {
                let index = self.current_action as usize;
                if self.actions[index].has_next(mascot, env, rng)? {
                    break;
                }
                self.set_current_action(mascot, env, self.current_action + 1, rng)?;
            }
        }
        Ok(())
    }

    /// Java ComplexAction.init L35-42 逐語。
    pub(crate) fn complex_init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.base.init(mascot);
        if self.base.base_has_next(mascot, env)? {
            self.set_current_action(mascot, env, 0, rng)?;
            self.seek(mascot, env, rng)?;
        }
        Ok(())
    }

    /// Java Sequence.hasNext L26-31 逐語（Sequence のみ）：毎回 seek してから
    /// super.hasNext（ComplexAction.hasNext L56-58）。Select はオーバーライドなし →
    /// seek を呼ばない。
    pub(crate) fn sequence_has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        if self.is_sequence {
            self.seek(mascot, env, rng)?;
        }
        self.complex_has_next(mascot, env, rng)
    }

    /// Java ComplexAction.hasNext L56-58 逐語。
    pub(crate) fn complex_has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        if !self.base.base_has_next(mascot, env)? {
            return Ok(false);
        }
        if self.current_action < 0 || self.current_action >= self.actions.len() as i32 {
            return Ok(false);
        }
        let index = self.current_action as usize;
        self.actions[index].has_next(mascot, env, rng)
    }

    /// Java ComplexAction.tick L61-66 逐語。
    pub(crate) fn complex_tick(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        let index = self.current_action as usize;
        let action = &mut self.actions[index];
        if action.has_next(mascot, env, rng)? {
            action.next(mascot, env, rng)?;
        }
        Ok(())
    }

    /// Java ComplexAction.isDraggable L90-96 逐語（currentAction の子に委譲）。
    fn complex_is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        if self.current_action < self.actions.len() as i32 && self.current_action >= 0 {
            let index = self.current_action as usize;
            self.actions[index].is_draggable(mascot, env, rng)
        } else {
            Ok(true)
        }
    }
}

impl Action for Complex {
    fn init(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        self.complex_init(mascot, env, rng)
    }

    fn has_next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.sequence_has_next(mascot, env, rng)
    }

    fn next(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        // Java ActionBase.next L100-119 に相当（r resetVariables → affordances →
        // refreshHotspots → tick）。
        self.base.next_pre(mascot, env)?;
        self.complex_tick(mascot, env, rng)
    }

    fn is_draggable(
        &mut self,
        mascot: &mut Mascot,
        env: &dyn EnvironmentView,
        rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.complex_is_draggable(mascot, env, rng)
    }
}
