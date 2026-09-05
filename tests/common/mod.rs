//! タスク #3 テスト共通モック（script_eval_test / config_parse_test から `mod common;` で使用）。
//!
//! MockCtx は asset-report.md §1-3 の mascot.* 網羅リストに対応する EvalContext 実装。
//! モックは公開契約（simeji::config::script::EvalContext）のみに依存し、
//! 未対応パスは None / false を返す（= 評価器が未知パスを要求したら Err になる検出器として働く）。
#![allow(dead_code)]

use std::cell::RefCell;

use simeji::config::script::{ConstantValue, EvalContext, EvalValue, Variable, Variables};

/// mascot 状態のモック。フィールドは各テストで書き換えてよい（すべて pub）。
/// 既定値は「十分に内側の画面・ WorkArea / IE」を想定し、資産 194 式が全て定義値を持つように選んである。
pub struct MockCtx {
    pub anchor_x: f64,
    pub anchor_y: f64,
    pub look_right: bool,
    pub total_count: f64,
    pub cursor_x: f64,
    pub cursor_y: f64,
    pub cursor_dx: f64,
    pub cursor_dy: f64,
    pub screen_height: f64,
    pub wa_left: f64,
    pub wa_right: f64,
    pub wa_top: f64,
    pub wa_bottom: f64,
    pub wa_width: f64,
    pub wa_height: f64,
    pub ie_left: f64,
    pub ie_right: f64,
    pub ie_top: f64,
    pub ie_bottom: f64,
    pub ie_width: f64,
    pub ie_height: f64,
    pub ie_visible: bool,
    /// isOn(...) の既知ターゲットへの応答（true/false を一括切替）
    pub all_on: bool,
    /// isOn 呼び出しの記録 (target, x, y)。呼び出し順保持。
    pub is_on_calls: RefCell<Vec<(String, f64, f64)>>,
}

impl MockCtx {
    pub fn new() -> Self {
        MockCtx {
            anchor_x: 100.0,
            anchor_y: 500.0,
            look_right: true,
            total_count: 3.0,
            cursor_x: 300.0,
            cursor_y: 200.0,
            cursor_dx: 5.0,
            cursor_dy: -2.0,
            screen_height: 1080.0,
            wa_left: 0.0,
            wa_right: 1920.0,
            wa_top: 0.0,
            wa_bottom: 1040.0,
            wa_width: 1920.0,
            wa_height: 1040.0,
            ie_left: 200.0,
            ie_right: 1200.0,
            ie_top: 100.0,
            ie_bottom: 900.0,
            ie_width: 1000.0,
            ie_height: 800.0,
            ie_visible: true,
            all_on: true,
            is_on_calls: RefCell::new(Vec::new()),
        }
    }
}

impl Default for MockCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl EvalContext for MockCtx {
    fn number(&self, path: &str) -> Option<f64> {
        match path {
            "mascot.anchor.x" => Some(self.anchor_x),
            "mascot.anchor.y" => Some(self.anchor_y),
            "mascot.totalCount" => Some(self.total_count),
            "mascot.environment.cursor.x" => Some(self.cursor_x),
            "mascot.environment.cursor.y" => Some(self.cursor_y),
            "mascot.environment.cursor.dx" => Some(self.cursor_dx),
            "mascot.environment.cursor.dy" => Some(self.cursor_dy),
            "mascot.environment.screen.height" => Some(self.screen_height),
            "mascot.environment.workArea.left" => Some(self.wa_left),
            "mascot.environment.workArea.right" => Some(self.wa_right),
            "mascot.environment.workArea.top" => Some(self.wa_top),
            "mascot.environment.workArea.bottom" => Some(self.wa_bottom),
            "mascot.environment.workArea.width" => Some(self.wa_width),
            "mascot.environment.workArea.height" => Some(self.wa_height),
            "mascot.environment.activeIE.left" => Some(self.ie_left),
            "mascot.environment.activeIE.right" => Some(self.ie_right),
            "mascot.environment.activeIE.top" => Some(self.ie_top),
            "mascot.environment.activeIE.bottom" => Some(self.ie_bottom),
            "mascot.environment.activeIE.width" => Some(self.ie_width),
            "mascot.environment.activeIE.height" => Some(self.ie_height),
            _ => None,
        }
    }

    fn boolean(&self, path: &str) -> Option<bool> {
        match path {
            "mascot.lookRight" => Some(self.look_right),
            "mascot.environment.activeIE.visible" => Some(self.ie_visible),
            _ => None,
        }
    }

    fn is_on(&self, target: &str, x: f64, y: f64) -> bool {
        self.is_on_calls
            .borrow_mut()
            .push((target.to_string(), x, y));
        matches!(
            target,
            "mascot.environment.floor"
                | "mascot.environment.wall"
                | "mascot.environment.ceiling"
                | "mascot.environment.workArea.leftBorder"
                | "mascot.environment.workArea.rightBorder"
                | "mascot.environment.workArea.topBorder"
                | "mascot.environment.workArea.bottomBorder"
                | "mascot.environment.activeIE.leftBorder"
                | "mascot.environment.activeIE.rightBorder"
                | "mascot.environment.activeIE.topBorder"
                | "mascot.environment.activeIE.bottomBorder"
        ) && self.all_on
    }
}

/// Java Variable.parse 相当の Script 変数を直接構築する（source は ${} / #{} の内側テキスト）。
pub fn script_var(source: &str, allow_value_reset: bool) -> Variable {
    Variable::Script {
        source: source.to_string(),
        allow_value_reset,
    }
}

/// 変数の代表文字列（panic メッセージ用。Variable の派生に依存しない）。
pub fn source_of(var: &Variable) -> String {
    match var {
        Variable::Script { source, .. } => source.clone(),
        Variable::Constant(ConstantValue::Bool(b)) => format!("bool:{b}"),
        Variable::Constant(ConstantValue::Number(n)) => format!("num:{n}"),
        Variable::Constant(ConstantValue::Text(t)) => format!("text:{t}"),
    }
}

/// EvalValue の型名（Debug/PartialEq 派生を要求しないための表示用）。
pub fn describe_value(v: &EvalValue) -> &'static str {
    match v {
        EvalValue::Number(_) => "Number(_)",
        EvalValue::Bool(_) => "Bool(_)",
    }
}

/// 新しい Variables で 1 回評価して Ok の値を返す（キャッシュの干扰なし）。
pub fn eval_ok(ctx: &MockCtx, var: &Variable) -> EvalValue {
    let mut vars = Variables::new();
    match vars.eval(var, ctx) {
        Ok(v) => v,
        Err(_) => panic!(
            "式 {:?} の評価が Err になった（実装の EvalError 型に Debug 派生を要求しないため詳細は省略）",
            source_of(var)
        ),
    }
}

/// 式（source テキスト）を数値として評価。
pub fn eval_num(ctx: &MockCtx, source: &str) -> f64 {
    match eval_ok(ctx, &script_var(source, true)) {
        EvalValue::Number(n) => n,
        other => panic!("式 {:?} は数値のはずが {}", source, describe_value(&other)),
    }
}

/// 式（source テキスト）をブールとして評価。
pub fn eval_bool(ctx: &MockCtx, source: &str) -> bool {
    match eval_ok(ctx, &script_var(source, true)) {
        EvalValue::Bool(b) => b,
        other => panic!("式 {:?} はブールのはずが {}", source, describe_value(&other)),
    }
}

/// 注入変数（FootX / TargetY / Gap 等）を設定してから数値評価。
pub fn eval_num_injected(ctx: &MockCtx, source: &str, injected: &[(&str, f64)]) -> f64 {
    let mut vars = Variables::new();
    for (name, value) in injected {
        vars.inject(*name, *value);
    }
    match vars.eval(&script_var(source, true), ctx) {
        Ok(EvalValue::Number(n)) => n,
        other => panic!(
            "式 {:?} は数値のはずが {}",
            source,
            describe_value_from(other)
        ),
    }
}

/// 注入変数を設定してからブール評価。
pub fn eval_bool_injected(ctx: &MockCtx, source: &str, injected: &[(&str, f64)]) -> bool {
    let mut vars = Variables::new();
    for (name, value) in injected {
        vars.inject(*name, *value);
    }
    match vars.eval(&script_var(source, true), ctx) {
        Ok(EvalValue::Bool(b)) => b,
        other => panic!(
            "式 {:?} はブールのはずが {}",
            source,
            describe_value_from(other)
        ),
    }
}

/// Result 版の値説明（Err も型名だけで表す。EvalError の Debug に依存しない）。
pub fn describe_value_from(r: Result<EvalValue, simeji::config::script::EvalError>) -> &'static str {
    match r {
        Ok(v) => describe_value(&v),
        Err(_) => "Err(_)",
    }
}

/// 焼き込みテスト用: 資産が参照しうる注入変数をすべて設定した Variables。
/// （FootDX は資産 XML では未使用だが Java Dragged が注入するため設定しておく）
pub fn standard_vars() -> Variables {
    let mut vars = Variables::new();
    vars.inject("FootX", 0.0);
    vars.inject("FootDX", 0.0);
    vars.inject("TargetY", 0.0);
    vars.inject("Gap", 0.0);
    vars
}

/// 比較用に空白連続を 1 個に潰す（資産 XML の複数行属性との一致確認用）。
pub fn norm_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}
