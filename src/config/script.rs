//! スクリプト式評価器 — Java `script` パッケージ相当。
//!
//! 資産 XML の `${...}` / `#{...}` 式を評価する。Nashorn JS のサブセットを
//! 自前の再帰下降パーサで評価する（JS エンジン不使用・任意コード実行なし）。
//! 対応範囲と資産 194 式の内訳は design.md §3-10 / asset-report.md §1。
//!
//! - キャッシュポリシーは Java `Script.java` の needsReevaluation モデルの移植:
//!   eval 成功時は値をキャッシュし、`reset_values()`（フレーム開始）で `#{}` のみ、
//!   `init()`（アクション開始）で全式を再評価状態へ戻す。
//!   ※ Java は Script インスタンス毎に値を保持するが、本実装は `eval(&Variable)` で
//!   インスタンスの同一性を取れないため (source, allow_value_reset) をキャッシュキーにする。
//!   同一式が同一フレーム内で複数箇所から評価されても値を共有するが、資産式は
//!   いずれも単発評価のため挙動への影響はない。
//! - NaN→int 変換は JLS 5.1.3（[`to_java_int`]）。資産の括弧抜け式
//!   `Math.random*100` 2 件は JS/Java 同様に NaN になり、Java 挙動では 0 扱いになる。

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher};

use thiserror::Error;

/// Java `Variable` 相当（式 or 定数）。
#[derive(Debug, Clone, PartialEq)]
pub enum Variable {
    /// `${...}` / `#{...}` 式。`allow_value_reset` は `#{}`（フレーム毎に再評価）か。
    Script {
        source: String,
        allow_value_reset: bool,
    },
    /// Java `Constant` 相当。
    Constant(ConstantValue),
}

/// 定数値（Java `Variable#parseConstant` の結果）。
#[derive(Debug, Clone, PartialEq)]
pub enum ConstantValue {
    Bool(bool),
    Number(f64),
    Text(String),
}

/// 評価結果。資産 194 式は数値 / ブールのみを前提とする（文字列式・連結は資産に 0 件）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EvalValue {
    Number(f64),
    Bool(bool),
}

impl EvalValue {
    pub fn as_number(self) -> Option<f64> {
        match self {
            EvalValue::Number(n) => Some(n),
            EvalValue::Bool(_) => None,
        }
    }

    pub fn as_bool(self) -> Option<bool> {
        match self {
            EvalValue::Bool(b) => Some(b),
            EvalValue::Number(_) => None,
        }
    }
}

/// Java 準拠の double→int 変換（JLS 5.1.3）。
/// NaN→0・0 への切り詰め（trunc toward zero）・範囲外は i32 に飽和。
/// Rust の `as i32` と同値であるためそのまま委譲する。
#[inline]
pub fn to_java_int(v: f64) -> i32 {
    v as i32
}

/// 式が参照する mascot 状態・境界判定の抽象。
/// 実装側（タスク #6/#8 の Mascot / Environment）が mascot 変数と isOn を提供する。
pub trait EvalContext {
    /// `mascot.xxx.yyy` 形式の数値変数。不明パスは None。
    fn number(&self, path: &str) -> Option<f64>;
    /// `mascot.lookRight` 等のブール変数。不明パスは None。
    fn boolean(&self, path: &str) -> Option<bool>;
    /// `xxx.isOn(mascot.anchor)` の境界接触判定
    /// （例: `is_on("mascot.environment.floor", anchor_x, anchor_y)`）。
    fn is_on(&self, target: &str, x: f64, y: f64) -> bool;
}

/// 式評価エラー。panic せず Result で伝播する。エラー後も Variables は再利用可能。
/// ※ thiserror が `source` という名のフィールドをエラー連鎖用と特別扱いするため、
/// 式ソースのフィールド名は `expr` にしている。
#[derive(Debug, Clone, PartialEq, Error)]
#[error("式評価エラー: {message}（式: `{expr}`）")]
pub struct EvalError {
    pub expr: String,
    pub message: String,
}

/// アクション単位の評価状態。注入変数（FootX / TargetY / Gap 等）+ スクリプト値キャッシュ
/// （Java `VariableMap` + `Script.value` 相当）。
pub struct Variables {
    injected: HashMap<String, f64>,
    // キー = (式ソース, allow_value_reset)。値は直近の評価結果。
    cache: HashMap<(String, bool), EvalValue>,
}

impl Default for Variables {
    fn default() -> Self {
        Self::new()
    }
}

impl Variables {
    pub fn new() -> Self {
        Variables {
            injected: HashMap::new(),
            cache: HashMap::new(),
        }
    }

    /// 注入変数を設定する（FootX / TargetY / Gap 等。同名なら上書き）。
    pub fn inject(&mut self, name: impl Into<String>, value: f64) {
        self.injected.insert(name.into(), value);
    }

    /// Java: script/VariableMap.java#init — 全スクリプトを再評価状態へ（アクション開始時）。
    pub fn init(&mut self) {
        self.cache.clear();
    }

    /// Java: script/VariableMap.java#resetValues — `#{}`（allow_value_reset = true）のみ
    /// 再評価状態へ（フレーム開始時）。`${}` はキャッシュを維持する。
    pub fn reset_values(&mut self) {
        self.cache.retain(|key, _| !key.1);
    }

    /// Java: script/Variable.java#get(VariableMap) 相当。
    /// needsReevaluation が立っている時だけ評価し、評価後は値をキャッシュする。
    /// 評価失敗時は log::warn に式とエラーを出して Err を返す（panic しない）。
    pub fn eval(&mut self, var: &Variable, ctx: &dyn EvalContext) -> Result<EvalValue, EvalError> {
        match var {
            Variable::Constant(value) => const_eval(value),
            Variable::Script {
                source,
                allow_value_reset,
            } => {
                let key = (source.clone(), *allow_value_reset);
                if let Some(cached) = self.cache.get(&key) {
                    return Ok(*cached);
                }
                match evaluate(source, &self.injected, ctx) {
                    Ok(value) => {
                        self.cache.insert(key, value);
                        Ok(value)
                    }
                    Err(err) => {
                        log::warn!("スクリプト式を評価できません: {{{}}}（{}）", source, err);
                        Err(err)
                    }
                }
            }
        }
    }
}

/// Java: script/Constant.java#get — 定数はその値を返す。
/// 文字列定数は評価値（数値 / ブール）として扱えないため Err。
fn const_eval(value: &ConstantValue) -> Result<EvalValue, EvalError> {
    match value {
        ConstantValue::Bool(b) => Ok(EvalValue::Bool(*b)),
        ConstantValue::Number(n) => Ok(EvalValue::Number(*n)),
        ConstantValue::Text(t) => Err(EvalError {
            expr: t.clone(),
            message: "文字列定数は評価値として扱えません".to_string(),
        }),
    }
}

impl Variable {
    /// Java: script/Variable.java#parse — 逐語移植。
    /// `${` + `}` / `#{` + `}` で完全包囲される場合は Script、それ以外は parseConstant。
    pub fn parse(source: &str) -> Variable {
        if source.starts_with("${") && source.ends_with('}') {
            Variable::Script {
                source: source[2..source.len() - 1].to_string(),
                allow_value_reset: false,
            }
        } else if source.starts_with("#{") && source.ends_with('}') {
            Variable::Script {
                source: source[2..source.len() - 1].to_string(),
                allow_value_reset: true,
            }
        } else {
            Variable::Constant(Self::parse_constant(source))
        }
    }

    /// Java: script/Variable.java#parseConstant — 逐語移植。
    /// "true" / "false" はブール、double としてパース可なら数値、それ以外は文字列。
    fn parse_constant(source: &str) -> ConstantValue {
        if source == "true" {
            ConstantValue::Bool(true)
        } else if source == "false" {
            ConstantValue::Bool(false)
        } else {
            match source.parse::<f64>() {
                Ok(n) => ConstantValue::Number(n),
                Err(_) => ConstantValue::Text(source.to_string()),
            }
        }
    }
}

// =============================================================================
// 式パーサ（Nashorn JS サブセット・再帰下降）
// =============================================================================

#[derive(Debug, Clone)]
enum Expr {
    Num(f64),
    Bool(bool),
    /// ドット区切りの識別子列（mascot.environment.workArea.left 等）。
    Path(String),
    /// `target.name(args)` 形式（Math.random() / mascot.environment.floor.isOn(mascot.anchor)）。
    Method {
        target: String,
        name: String,
        args: Vec<Expr>,
    },
    Not(Box<Expr>),
    Neg(Box<Expr>),
    Bin {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Ternary {
        cond: Box<Expr>,
        then: Box<Expr>,
        else_: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Ident(String),
    Sym(Sym),
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Sym {
    Plus,
    Minus,
    Star,
    Slash,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    AndAnd,
    OrOr,
    Not,
    Question,
    Colon,
    LParen,
    RParen,
    Dot,
    Comma,
}

/// 式ソースをトークン列へ分解する。空白（空白・改行・タブ・CR）はスキップする。
/// 資産には物理複数行にまたがる式が 14 件あるため改行をまたいで解析できる必要がある。
fn lex(source: &str) -> Result<Vec<Tok>, String> {
    let chars: Vec<char> = source.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        // 数値リテラル（整数部 + 任意の小数部）
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let n = text
                .parse::<f64>()
                .map_err(|_| format!("数値として解釈できません: {text}"))?;
            toks.push(Tok::Num(n));
            continue;
        }
        // 識別子（mascot / FootX / Math / _x / $x 等）
        if c.is_ascii_alphabetic() || c == '_' || c == '$' {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '$') {
                i += 1;
            }
            toks.push(Tok::Ident(chars[start..i].iter().collect()));
            continue;
        }
        // 2 文字記号
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let two_sym = match two.as_str() {
            "==" => Some(Sym::Eq),
            "!=" => Some(Sym::Ne),
            "<=" => Some(Sym::Le),
            ">=" => Some(Sym::Ge),
            "&&" => Some(Sym::AndAnd),
            "||" => Some(Sym::OrOr),
            _ => None,
        };
        if let Some(sym) = two_sym {
            toks.push(Tok::Sym(sym));
            i += 2;
            continue;
        }
        // 1 文字記号
        let one_sym = match c {
            '+' => Some(Sym::Plus),
            '-' => Some(Sym::Minus),
            '*' => Some(Sym::Star),
            '/' => Some(Sym::Slash),
            '<' => Some(Sym::Lt),
            '>' => Some(Sym::Gt),
            '!' => Some(Sym::Not),
            '?' => Some(Sym::Question),
            ':' => Some(Sym::Colon),
            '(' => Some(Sym::LParen),
            ')' => Some(Sym::RParen),
            '.' => Some(Sym::Dot),
            ',' => Some(Sym::Comma),
            _ => None,
        };
        match one_sym {
            Some(sym) => {
                toks.push(Tok::Sym(sym));
                i += 1;
            }
            None => return Err(format!("未知の文字: {c}")),
        }
    }
    Ok(toks)
}

/// 再帰下降パーサ。演算子の優先度・結合規則は Java（Nashorn JS）準拠:
/// 単項（! -）> 乗除 > 加減 > 関係（< <= > >=）> 等価（== !=）> && > || > 三項（最低）。
/// 二項演算は左結合、三項は右結合。
struct Parser<'s> {
    toks: Vec<Tok>,
    pos: usize,
    source: &'s str,
}

impl<'s> Parser<'s> {
    fn err(&self, message: impl Into<String>) -> EvalError {
        EvalError {
            expr: self.source.to_string(),
            message: message.into(),
        }
    }

    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn bump(&mut self) -> Option<Tok> {
        let tok = self.toks.get(self.pos).cloned();
        if tok.is_some() {
            self.pos += 1;
        }
        tok
    }

    fn eat(&mut self, sym: Sym) -> bool {
        if matches!(self.peek(), Some(Tok::Sym(s)) if *s == sym) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, sym: Sym, what: &str) -> Result<(), EvalError> {
        if self.eat(sym) {
            Ok(())
        } else {
            Err(self.err(format!("{what} が必要ですが見つかりません")))
        }
    }

    /// 式全体をパースし、全トークンを消費できていることを確認する。
    fn parse_expr(&mut self) -> Result<Expr, EvalError> {
        let expr = self.parse_ternary()?;
        if self.pos != self.toks.len() {
            return Err(self.err("式の後に余分なトークンがあります"));
        }
        Ok(expr)
    }

    fn parse_ternary(&mut self) -> Result<Expr, EvalError> {
        let cond = self.parse_or()?;
        if self.eat(Sym::Question) {
            let then = self.parse_ternary()?;
            self.expect(Sym::Colon, "三項演算子の `:`")?;
            let else_ = self.parse_ternary()?;
            Ok(Expr::Ternary {
                cond: Box::new(cond),
                then: Box::new(then),
                else_: Box::new(else_),
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_or(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.parse_and()?;
        while self.eat(Sym::OrOr) {
            let rhs = self.parse_and()?;
            lhs = Expr::Or(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_and(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.parse_equality()?;
        while self.eat(Sym::AndAnd) {
            let rhs = self.parse_equality()?;
            lhs = Expr::And(Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_equality(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.parse_relational()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(Sym::Eq)) => BinOp::Eq,
                Some(Tok::Sym(Sym::Ne)) => BinOp::Ne,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_relational()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_relational(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.parse_additive()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(Sym::Lt)) => BinOp::Lt,
                Some(Tok::Sym(Sym::Le)) => BinOp::Le,
                Some(Tok::Sym(Sym::Gt)) => BinOp::Gt,
                Some(Tok::Sym(Sym::Ge)) => BinOp::Ge,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_additive()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_additive(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.parse_multiplicative()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(Sym::Plus)) => BinOp::Add,
                Some(Tok::Sym(Sym::Minus)) => BinOp::Sub,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_multiplicative()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_multiplicative(&mut self) -> Result<Expr, EvalError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek() {
                Some(Tok::Sym(Sym::Star)) => BinOp::Mul,
                Some(Tok::Sym(Sym::Slash)) => BinOp::Div,
                _ => break,
            };
            self.pos += 1;
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, EvalError> {
        if self.eat(Sym::Not) {
            return Ok(Expr::Not(Box::new(self.parse_unary()?)));
        }
        if self.eat(Sym::Minus) {
            return Ok(Expr::Neg(Box::new(self.parse_unary()?)));
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> Result<Expr, EvalError> {
        match self.bump() {
            Some(Tok::Num(n)) => Ok(Expr::Num(n)),
            Some(Tok::Ident(name)) => {
                if name == "true" {
                    return Ok(Expr::Bool(true));
                }
                if name == "false" {
                    return Ok(Expr::Bool(false));
                }
                // ドットでつながった識別子列（mascot.environment.floor 等）。
                // 末尾要素の直後に `(` が続けばメソッド呼び出し
                // （mascot.environment.floor.isOn(mascot.anchor) / Math.random()）。
                let mut segs = vec![name];
                while self.eat(Sym::Dot) {
                    match self.bump() {
                        Some(Tok::Ident(seg)) => segs.push(seg),
                        _ => return Err(self.err("`.` の後は識別子である必要があります")),
                    }
                }
                if self.eat(Sym::LParen) {
                    let name = segs.pop().unwrap_or_default();
                    let args = self.parse_args()?;
                    Ok(Expr::Method {
                        target: segs.join("."),
                        name,
                        args,
                    })
                } else {
                    Ok(Expr::Path(segs.join(".")))
                }
            }
            Some(Tok::Sym(Sym::LParen)) => {
                let expr = self.parse_ternary()?;
                self.expect(Sym::RParen, "括弧を閉じる `)`")?;
                Ok(expr)
            }
            Some(tok) => Err(self.err(format!("予期しないトークン: {tok:?}"))),
            None => Err(self.err("式が必要ですが入力が終了しました")),
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>, EvalError> {
        let mut args = Vec::new();
        if self.eat(Sym::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_ternary()?);
            if self.eat(Sym::Comma) {
                continue;
            }
            self.expect(Sym::RParen, "引数リストを閉じる `)`")?;
            return Ok(args);
        }
    }
}

// =============================================================================
// 評価器
// =============================================================================

struct Interp<'a> {
    source: &'a str,
    injected: &'a HashMap<String, f64>,
    ctx: &'a dyn EvalContext,
}

impl<'a> Interp<'a> {
    fn err(&self, message: impl Into<String>) -> EvalError {
        EvalError {
            expr: self.source.to_string(),
            message: message.into(),
        }
    }

    fn type_err(&self, what: &str, expected: &str, got: EvalValue) -> EvalError {
        let got_name = match got {
            EvalValue::Number(_) => "数値",
            EvalValue::Bool(_) => "ブール",
        };
        self.err(format!("{what} には{expected}が必要ですが {got_name} でした"))
    }

    fn eval(&self, expr: &Expr) -> Result<EvalValue, EvalError> {
        match expr {
            Expr::Num(n) => Ok(EvalValue::Number(*n)),
            Expr::Bool(b) => Ok(EvalValue::Bool(*b)),
            Expr::Path(path) => self.eval_path(path),
            Expr::Method { target, name, args } => self.eval_method(target, name, args),
            Expr::Not(inner) => match self.eval(inner)? {
                EvalValue::Bool(b) => Ok(EvalValue::Bool(!b)),
                got => Err(self.type_err("論理否定 `!`", "ブール", got)),
            },
            Expr::Neg(inner) => match self.eval(inner)? {
                EvalValue::Number(n) => Ok(EvalValue::Number(-n)),
                got => Err(self.type_err("単項 `-`", "数値", got)),
            },
            Expr::Bin { op, lhs, rhs } => self.eval_bin(*op, lhs, rhs),
            Expr::And(lhs, rhs) => {
                // Java && に従い短絡評価する
                if !self.eval_bool(lhs, "&& の左辺")? {
                    Ok(EvalValue::Bool(false))
                } else {
                    Ok(EvalValue::Bool(self.eval_bool(rhs, "&& の右辺")?))
                }
            }
            Expr::Or(lhs, rhs) => {
                // Java || に従い短絡評価する
                if self.eval_bool(lhs, "|| の左辺")? {
                    Ok(EvalValue::Bool(true))
                } else {
                    Ok(EvalValue::Bool(self.eval_bool(rhs, "|| の右辺")?))
                }
            }
            Expr::Ternary { cond, then, else_ } => {
                // 条件側のみを評価する（Java 三項演算子と同じ遅延評価）
                if self.eval_bool(cond, "三項演算子の条件")? {
                    self.eval(then)
                } else {
                    self.eval(else_)
                }
            }
        }
    }

    fn eval_bool(&self, expr: &Expr, what: &str) -> Result<bool, EvalError> {
        match self.eval(expr)? {
            EvalValue::Bool(b) => Ok(b),
            got => Err(self.type_err(what, "ブール", got)),
        }
    }

    fn eval_num(&self, expr: &Expr, what: &str) -> Result<f64, EvalError> {
        match self.eval(expr)? {
            EvalValue::Number(n) => Ok(n),
            got => Err(self.type_err(what, "数値", got)),
        }
    }

    fn eval_bin(&self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<EvalValue, EvalError> {
        use BinOp::{Add, Div, Eq, Ge, Gt, Le, Lt, Mul, Ne, Sub};
        match op {
            Add | Sub | Mul | Div => {
                let a = self.eval_num(lhs, "算術演算の左辺")?;
                let b = self.eval_num(rhs, "算術演算の右辺")?;
                let n = match op {
                    Add => a + b,
                    Sub => a - b,
                    Mul => a * b,
                    Div => a / b,
                    _ => unreachable!(),
                };
                Ok(EvalValue::Number(n))
            }
            Lt | Le | Gt | Ge => {
                let a = self.eval_num(lhs, "比較の左辺")?;
                let b = self.eval_num(rhs, "比較の右辺")?;
                let r = match op {
                    Lt => a < b,
                    Le => a <= b,
                    Gt => a > b,
                    Ge => a >= b,
                    _ => unreachable!(),
                };
                Ok(EvalValue::Bool(r))
            }
            Eq | Ne => {
                let a = self.eval(lhs)?;
                let b = self.eval(rhs)?;
                let r = match (a, b) {
                    (EvalValue::Number(x), EvalValue::Number(y)) => x == y,
                    (EvalValue::Bool(x), EvalValue::Bool(y)) => x == y,
                    _ => return Err(self.type_err("等価比較", "同型の値", b)),
                };
                Ok(EvalValue::Bool(if op == Eq { r } else { !r }))
            }
        }
    }

    /// 識別子パスの解決。優先順位は Math（括弧抜け参照）/ mascot.* / 注入変数。
    fn eval_path(&self, path: &str) -> Result<EvalValue, EvalError> {
        if path.starts_with("Math.") {
            // 括弧の無い `Math.random` 等は JS では関数オブジェクト参照となり、
            // 数値演算では NaN になる。資産の括弧抜け式 2 件
            // （`Math.random*100` → Java (int)NaN = 0）がこの挙動に依存するため
            // エラーにせず NaN を返す。
            return Ok(EvalValue::Number(f64::NAN));
        }
        if path.starts_with("mascot.") {
            if let Some(n) = self.ctx.number(path) {
                return Ok(EvalValue::Number(n));
            }
            if let Some(b) = self.ctx.boolean(path) {
                return Ok(EvalValue::Bool(b));
            }
            return Err(self.err(format!("不明な mascot 変数: {path}")));
        }
        if let Some(n) = self.injected.get(path) {
            return Ok(EvalValue::Number(*n));
        }
        Err(self.err(format!("不明な識別子: {path}")))
    }

    /// メソッド呼び出し。対応範囲は Math.random/abs/min と isOn のみ（資産で使用の全種）。
    fn eval_method(&self, target: &str, name: &str, args: &[Expr]) -> Result<EvalValue, EvalError> {
        if target == "Math" {
            return match (name, args.len()) {
                ("random", 0) => Ok(EvalValue::Number(random_unit())),
                ("abs", 1) => Ok(EvalValue::Number(
                    self.eval_num(&args[0], "Math.abs の引数")?.abs(),
                )),
                ("min", 2) => {
                    let a = self.eval_num(&args[0], "Math.min の第 1 引数")?;
                    let b = self.eval_num(&args[1], "Math.min の第 2 引数")?;
                    Ok(EvalValue::Number(java_min(a, b)))
                }
                _ => Err(self.err(format!(
                    "未対応の Math 関数: Math.{}（引数 {} 個）",
                    name,
                    args.len()
                ))),
            };
        }
        if target.is_empty() {
            return Err(self.err(format!("メソッド呼び出しの対象がありません: {name}")));
        }
        if name == "isOn" && args.len() == 1 {
            let (x, y) = self.eval_point(&args[0])?;
            return Ok(EvalValue::Bool(self.ctx.is_on(target, x, y)));
        }
        Err(self.err(format!("未対応のメソッド呼び出し: {target}.{name}")))
    }

    /// isOn の引数点。`mascot.anchor` はアンカー点 (anchor.x, anchor.y) に展開する
    /// （資産 21 式すべてがこの形式）。それ以外は評価値を x / y 両方に使う。
    fn eval_point(&self, arg: &Expr) -> Result<(f64, f64), EvalError> {
        if let Expr::Path(path) = arg {
            if path == "mascot.anchor" {
                let x = self
                    .ctx
                    .number("mascot.anchor.x")
                    .ok_or_else(|| self.err("不明な mascot 変数: mascot.anchor.x"))?;
                let y = self
                    .ctx
                    .number("mascot.anchor.y")
                    .ok_or_else(|| self.err("不明な mascot 変数: mascot.anchor.y"))?;
                return Ok((x, y));
            }
        }
        let n = self.eval_num(arg, "isOn の引数")?;
        Ok((n, n))
    }
}

/// Java `Math.min(double, double)` 相当。どちらかが NaN なら NaN。
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// Java `Math.random()` 相当の [0,1) 一様乱数。
/// グローバル可変状態を避けるため、std の HashMap 用シード機構（RandomState）から
/// 64bit 値を導出する。`RandomState::new()` は呼び出しごとに異なるシードを返す
/// （OS エントロピー + スレッドローカルなカウンタ）ため、Mutex も不要。
fn random_unit() -> f64 {
    let hasher = RandomState::new().build_hasher();
    let bits = hasher.finish();
    // 上位 53bit を [0,1) の double へ（JS の Math.random と同じ分解能）
    ((bits >> 11) as f64) * (1.0 / ((1u64 << 53) as f64))
}

/// 式 1 本を評価する（パース → AST 評価）。
fn evaluate(
    source: &str,
    injected: &HashMap<String, f64>,
    ctx: &dyn EvalContext,
) -> Result<EvalValue, EvalError> {
    let toks = lex(source).map_err(|message| EvalError {
        expr: source.to_string(),
        message,
    })?;
    let mut parser = Parser {
        toks,
        pos: 0,
        source,
    };
    let expr = parser.parse_expr()?;
    let interp = Interp {
        source,
        injected,
        ctx,
    };
    interp.eval(&expr)
}
