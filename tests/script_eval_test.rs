//! タスク #3: src/config/script.rs（式評価器）の契約テスト。
//!
//! 検証対象は公開契約のみ:
//! - Variable::Script / Variable::Constant の評価とキャッシュポリシー
//!   （eval 値はキャッシュ。reset_values() で #{} のみ再評価状態へ、init() で全再評価状態へ）
//! - 式文法: 四則・比較・論理・三項・単項 -/!・括弧・Math.random/abs/min・isOn()
//! - mascot.* 変数（asset-report.md §1-3 網羅リスト）+ 注入変数（FootX/TargetY/Gap）
//! - to_java_int（JLS 5.1.3: NaN→0・trunc toward zero・飽和）
//! - EvalError（未対応式は panic せず Err）
//! - 資産 194 式の全件評価（roxmltree で conf 実物から抽出）
//!
//! 実装 (src/config/) は未存在のため cargo test は compile error = RED が正常。

mod common;

use std::collections::BTreeMap;

use common::{
    describe_value, describe_value_from, eval_bool, eval_bool_injected, eval_num,
    eval_num_injected, eval_ok, norm_ws, script_var, standard_vars, MockCtx,
};
use shimeji::config::script::{
    to_java_int, ConstantValue, EvalContext, EvalValue, Variable, Variables,
};
use shimeji::config::VarMap;

// =====================================================================
// to_java_int（Java (int) キャスト準拠 / JLS 5.1.3）
// =====================================================================

#[test]
fn to_java_int_truncates_toward_zero() {
    // floor ではなく trunc（-2.5 → -2）
    assert_eq!(to_java_int(2.5), 2);
    assert_eq!(to_java_int(-2.5), -2);
    assert_eq!(to_java_int(2.99), 2);
    assert_eq!(to_java_int(-2.99), -2);
    assert_eq!(to_java_int(0.5), 0);
    assert_eq!(to_java_int(-0.5), 0);
    assert_eq!(to_java_int(1.999_999), 1);
    assert_eq!(to_java_int(-1.999_999), -1);
    assert_eq!(to_java_int(42.0), 42);
    assert_eq!(to_java_int(-42.0), -42);
    assert_eq!(to_java_int(0.0), 0);
}

#[test]
fn to_java_int_nan_is_zero() {
    // Java (int)NaN == 0（資産の括弧抜け式 2 件の Java 準拠規約）
    assert_eq!(to_java_int(f64::NAN), 0);
}

#[test]
fn to_java_int_saturates_out_of_range() {
    assert_eq!(to_java_int(f64::INFINITY), i32::MAX);
    assert_eq!(to_java_int(f64::NEG_INFINITY), i32::MIN);
    assert_eq!(to_java_int(2_147_483_647.0), 2_147_483_647);
    assert_eq!(to_java_int(2_147_483_648.0), i32::MAX); // 2^31 以上は飽和
    assert_eq!(to_java_int(-2_147_483_648.0), i32::MIN);
    assert_eq!(to_java_int(-2_147_483_649.0), i32::MIN);
    assert_eq!(to_java_int(1e10), i32::MAX);
    assert_eq!(to_java_int(-1e10), i32::MIN);
    assert_eq!(to_java_int(f64::MAX), i32::MAX);
    assert_eq!(to_java_int(f64::MIN), i32::MIN);
}

#[test]
fn to_java_int_matches_java_cast_semantics() {
    // JLS 5.1.3 = Rust の f64 as i32（NaN→0・trunc・飽和）と同値であること
    for v in [
        0.0,
        -0.0,
        1.5,
        -1.5,
        123456.789,
        -123456.789,
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        1e9,
        -1e9,
        f64::MAX,
        f64::MIN,
    ] {
        assert_eq!(to_java_int(v), v as i32, "v = {v}");
    }
}

// =====================================================================
// EvalValue 変換 / 定数評価
// =====================================================================

#[test]
fn eval_value_converters() {
    let n = EvalValue::Number(2.5);
    assert_eq!(n.as_number(), Some(2.5));
    assert_eq!(n.as_bool(), None);
    let b = EvalValue::Bool(true);
    assert_eq!(b.as_bool(), Some(true));
    assert_eq!(b.as_number(), None);
}

#[test]
fn constant_variables_evaluate_to_their_value() {
    let ctx = MockCtx::new();
    match eval_ok(&ctx, &Variable::Constant(ConstantValue::Number(2.5))) {
        EvalValue::Number(n) => assert_eq!(n, 2.5),
        other => panic!("数値定数のはずが {}", describe_value(&other)),
    }
    match eval_ok(&ctx, &Variable::Constant(ConstantValue::Bool(false))) {
        EvalValue::Bool(b) => assert!(!b),
        other => panic!("ブール定数のはずが {}", describe_value(&other)),
    }
}

// =====================================================================
// 式文法: 四則・優先順位・結合則・括弧
// =====================================================================

#[test]
fn arithmetic_precedence_and_associativity() {
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "1+2*3"), 7.0); // * が + より強い
    assert_eq!(eval_num(&ctx, "2*3+1"), 7.0);
    assert_eq!(eval_num(&ctx, "(1+2)*3"), 9.0); // 括弧
    assert_eq!(eval_num(&ctx, "10-2-3"), 5.0); // 左結合 (11 ではない)
    assert_eq!(eval_num(&ctx, "100/10/2"), 5.0); // 左結合
    assert_eq!(eval_num(&ctx, "10/4"), 2.5); // 除算は実数
    assert_eq!(eval_num(&ctx, "2+3*4-1"), 13.0);
}

#[test]
fn unary_minus() {
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "-5+10"), 5.0);
    assert_eq!(eval_num(&ctx, "-15-5"), -20.0);
    assert_eq!(eval_num(&ctx, "-(1+2)"), -3.0);
    // 資産 ChaseMouse の Gap 式と同形: 単項 - に関数呼び出し結果を適用
    assert_eq!(eval_num(&ctx, "-Math.abs(0-7)"), -7.0);
}

/// 単項 `+`（実資産に 6 出現）。デレマスしめじ v1.9 の Nagi
/// 「カーソルに近づきジャンプしてグライダー」が
/// `mascot.lookRight ? +20 : -20` / `+15+Math.random()*5` を使う。
/// 未対応だと式が Err → 既定値フォールバックになり落下・ジャンプが壊れる。
#[test]
fn unary_plus() {
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "+15"), 15.0);
    assert_eq!(eval_num(&ctx, "+15-Math.abs(0-5)"), 10.0);
    assert_eq!(eval_num(&ctx, "+mascot.environment.cursor.x+50"), 350.0);
    // 三項の両腕に単項 + / - が現れる実資産形（Nagi InitialVX）
    assert_eq!(eval_num(&ctx, "mascot.lookRight ? +20 : -20"), 20.0);
    let mut facing_left = MockCtx::new();
    facing_left.look_right = false;
    assert_eq!(
        eval_num(&facing_left, "mascot.lookRight ? +20 : -20"),
        -20.0
    );
    assert_eq!(
        eval_num(
            &facing_left,
            "mascot.lookRight ? +mascot.environment.cursor.x+50 : mascot.environment.cursor.x-50"
        ),
        250.0
    );
    // Math.random と混在する形（参照実装の InitialVX）
    let v = eval_num(
        &ctx,
        "mascot.lookRight ? +15+Math.random()*5 : -15-Math.random()*5",
    );
    assert!(
        (15.0..20.0).contains(&v),
        "単項 + 付き三項が有限値へ評価される: {v}"
    );
}

#[test]
fn multiline_and_extra_whitespace_sources() {
    // 資産には物理複数行にまたがる式が 14 件ある（空白・改行を許すこと）
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "1 +\n  2"), 3.0);
    assert_eq!(eval_num(&ctx, "  ( 1 + 2 ) * 3 "), 9.0);
    assert_eq!(
        eval_num(&ctx, "mascot.environment.screen.height\n/\n2"),
        540.0
    );
}

// =====================================================================
// 式文法: 比較・論理・三項
// =====================================================================

#[test]
fn numeric_comparisons() {
    let ctx = MockCtx::new();
    assert!(eval_bool(&ctx, "1 < 2"));
    assert!(!eval_bool(&ctx, "2 < 1"));
    assert!(!eval_bool(&ctx, "1 > 2"));
    assert!(eval_bool(&ctx, "2 > 1"));
    assert!(eval_bool(&ctx, "2 >= 2"));
    assert!(!eval_bool(&ctx, "3 >= 4"));
    assert!(eval_bool(&ctx, "2 <= 2"));
    assert!(!eval_bool(&ctx, "3 <= 2"));
    assert!(eval_bool(&ctx, "1 == 1"));
    assert!(!eval_bool(&ctx, "1 == 2"));
    assert!(!eval_bool(&ctx, "1 != 1"));
    assert!(eval_bool(&ctx, "1 != 2"));
    // 比較は数値比較（算術が先に評価される）
    assert!(eval_bool(&ctx, "1+1 >= 2"));
}

#[test]
fn logical_operators() {
    let ctx = MockCtx::new();
    assert!(eval_bool(&ctx, "1 < 2 && 2 < 3"));
    assert!(!eval_bool(&ctx, "1 < 2 && 3 < 2"));
    assert!(eval_bool(&ctx, "3 < 2 || 2 < 3"));
    assert!(!eval_bool(&ctx, "3 < 2 || 4 < 3"));
    assert!(!eval_bool(&ctx, "!(1 < 2)")); // 単項 !
    assert!(eval_bool(&ctx, "!(3 < 2)"));
    // && が || より強い: (1<2 && 2<3) || (3<2)
    assert!(eval_bool(&ctx, "1 < 2 && 2 < 3 || 3 < 2"));
}

#[test]
fn ternary_operator() {
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "1 < 2 ? 10 : 20"), 10.0);
    assert_eq!(eval_num(&ctx, "2 < 1 ? 10 : 20"), 20.0);
    // 資産 FallFromWall の Offset X 式と同形（mascot.lookRight で分岐）
    assert_eq!(eval_num(&ctx, "mascot.lookRight ? -1 : 1"), -1.0);
    let mut ctx2 = MockCtx::new();
    ctx2.look_right = false;
    assert_eq!(eval_num(&ctx2, "mascot.lookRight ? -1 : 1"), 1.0);
    // 資産 PullUpShimeji / Divided の InitialVX 式
    assert_eq!(eval_num(&ctx, "mascot.lookRight ? -20 : 20"), -20.0);
    assert_eq!(eval_num(&ctx, "mascot.lookRight ? 10 : -10"), 10.0);
}

// =====================================================================
// Math 関数
// =====================================================================

#[test]
fn math_random_is_in_unit_interval() {
    // Math.random() はゼロ引数・[0,1)。（新鮮な Variables でキャッシュ無効化）
    for _ in 0..200 {
        let ctx = MockCtx::new();
        let mut vars = standard_vars();
        let r = match vars.eval(&script_var("Math.random()", true), &ctx) {
            Ok(EvalValue::Number(n)) => n,
            Ok(other) => panic!("Math.random() は数値のはずが {}", describe_value(&other)),
            Err(_) => panic!("Math.random() の評価が Err になった"),
        };
        assert!((0.0..1.0).contains(&r), "Math.random() = {r} が [0,1) 外");
    }
}

#[test]
fn math_random_composes_like_asset_durations() {
    let ctx = MockCtx::new();
    // 資産 Duration の典型形: ${500+Math.random()*1000}
    for _ in 0..100 {
        let v = eval_num(&ctx, "500+Math.random()*1000");
        assert!(
            (500.0..1500.0).contains(&v),
            "500+Math.random()*1000 = {v} が [500,1500) 外"
        );
    }
}

#[test]
fn math_abs_and_min() {
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "Math.abs(0-7)"), 7.0);
    assert_eq!(eval_num(&ctx, "Math.abs(3.5)"), 3.5);
    assert_eq!(eval_num(&ctx, "Math.min(3, 7)"), 3.0);
    assert_eq!(eval_num(&ctx, "Math.min(7, 3)"), 3.0); // 引数順非依存
    assert_eq!(eval_num(&ctx, "Math.min(2.5, 2.5)"), 2.5);
}

#[test]
fn chase_mouse_gap_expression_with_double_min() {
    // 資産 ChaseMouse の Gap 属性値（Math.min を同一式内で 2 回呼ぶ実物・空白を正規化）
    let source = norm_ws(
        "mascot.anchor.x < mascot.environment.cursor.x ?
            -Math.min( mascot.environment.cursor.x-mascot.anchor.x, Math.random()*200 ) :
            Math.min( mascot.anchor.x-mascot.environment.cursor.x, Math.random()*200 )",
    );
    let ctx = MockCtx::new();
    // 既定: anchor.x=100 < cursor.x=300 → 真側 = -Math.min(200, r*200) ∈ [-200, 0]
    for _ in 0..100 {
        let v = eval_num(&ctx, &source);
        assert!((-200.0..=0.0).contains(&v), "Gap 式(真側) = {v}");
    }
    // 偽側: anchor.x=400 > cursor.x=100 → Math.min(300, r*200) ∈ [0, 200]
    let mut ctx2 = MockCtx::new();
    ctx2.anchor_x = 400.0;
    ctx2.cursor_x = 100.0;
    for _ in 0..100 {
        let v = eval_num(&ctx2, &source);
        assert!((0.0..=200.0).contains(&v), "Gap 式(偽側) = {v}");
    }
}

// =====================================================================
// NaN 変例 2 件（Math.random 括弧抜け = Java でも 0 扱い）
// =====================================================================

#[test]
fn bare_math_random_times_number_is_nan() {
    // JS/Java と同様に Math.random（関数オブジェクト）×数値 = NaN。エラーにしない。
    let ctx = MockCtx::new();
    assert!(eval_num(&ctx, "Math.random*100").is_nan());
}

#[test]
fn climb_along_wall_targetx_nan_branch() {
    // 資産 actions.xml ClimbAlongWall の TargetX 三項式の else 側（括弧抜け）→ NaN
    let source = "mascot.lookRight ? mascot.environment.workArea.left+Math.random()*100 : mascot.environment.workArea.right-Math.random*100";
    let ctx = MockCtx::new();
    let mut ctx_false = MockCtx::new();
    ctx_false.look_right = false;
    assert!(eval_num(&ctx_false, source).is_nan());
    // NaN は Java (int) で 0（to_java_int 準拠）
    assert_eq!(to_java_int(eval_num(&ctx_false, source)), 0);
    // 真側は正常値: workArea.left + random*100 = r*100 ∈ [0, 100)
    for _ in 0..100 {
        let v = eval_num(&ctx, source);
        assert!(
            (0.0..100.0).contains(&v),
            "ClimbAlongWall TargetX(真側) = {v}"
        );
    }
}

#[test]
fn climb_ie_wall_targetx_nan_branch() {
    // 資産 actions.xml ClimbIEWall の TargetX（同じく else 側が括弧抜け）→ NaN
    let source = "mascot.lookRight ? mascot.environment.activeIE.left+Math.random()*100 : mascot.environment.activeIE.right-Math.random*100";
    let mut ctx = MockCtx::new();
    ctx.look_right = false;
    assert!(eval_num(&ctx, source).is_nan());
    assert_eq!(to_java_int(eval_num(&ctx, source)), 0);
}

// =====================================================================
// isOn(...)（境界パス → EvalContext::is_on）
// =====================================================================

#[test]
fn is_on_floor_receives_target_and_anchor_point() {
    // 資産 Fall / Thrown 内の条件式と同形
    let ctx = MockCtx::new();
    assert!(eval_bool(
        &ctx,
        "mascot.environment.floor.isOn(mascot.anchor)"
    ));
    // target パスと引数点（mascot.anchor → anchor.x, anchor.y）が EvalContext に渡ること
    assert_eq!(
        ctx.is_on_calls.borrow().as_slice(),
        [("mascot.environment.floor".to_string(), 100.0, 500.0)]
    );
    // 応答が式に反映される
    let mut ctx_off = MockCtx::new();
    ctx_off.all_on = false;
    assert!(!eval_bool(
        &ctx_off,
        "mascot.environment.floor.isOn(mascot.anchor)"
    ));
}

#[test]
fn is_on_border_or_condition() {
    // 資産 ChaseMouse の壁寄りシーケンス条件（|| と 2 つの isOn）
    let ctx = MockCtx::new();
    assert!(eval_bool(
        &ctx,
        "mascot.environment.workArea.leftBorder.isOn(mascot.anchor) || mascot.environment.activeIE.rightBorder.isOn(mascot.anchor)"
    ));
    let mut ctx_off = MockCtx::new();
    ctx_off.all_on = false;
    assert!(!eval_bool(
        &ctx_off,
        "mascot.environment.workArea.leftBorder.isOn(mascot.anchor) || mascot.environment.activeIE.rightBorder.isOn(mascot.anchor)"
    ));
}

#[test]
fn is_on_ternary_selects_border_by_look_right() {
    // 資産 behaviors.xml「Work Area 壁」グループ条件（三項で isOn の対象が切り替わる）
    let source = norm_ws(
        "mascot.lookRight ? mascot.environment.workArea.rightBorder.isOn(mascot.anchor) :
            mascot.environment.workArea.leftBorder.isOn(mascot.anchor)",
    );
    let ctx = MockCtx::new(); // lookRight=true → rightBorder を見る
    assert!(eval_bool(&ctx, &source));
    let guard = ctx.is_on_calls.borrow();
    let called: Vec<&str> = guard.iter().map(|(t, _, _)| t.as_str()).collect();
    assert!(
        called.contains(&"mascot.environment.workArea.rightBorder"),
        "lookRight=true では rightBorder の isOn が呼ばれる（記録: {called:?}）"
    );

    let mut ctx2 = MockCtx::new(); // lookRight=false → leftBorder を見る
    ctx2.look_right = false;
    assert!(eval_bool(&ctx2, &source));
    let guard2 = ctx2.is_on_calls.borrow();
    let called2: Vec<&str> = guard2.iter().map(|(t, _, _)| t.as_str()).collect();
    assert!(
        called2.contains(&"mascot.environment.workArea.leftBorder"),
        "lookRight=false では leftBorder の isOn が呼ばれる（記録: {called2:?}）"
    );
}

#[test]
fn chase_mouse_dash_ie_ceiling_conditions() {
    // 資産 ChaseMouse 内 Select の 2 条件（isOn && 比較、実物は複数行）
    let left = norm_ws(
        "mascot.environment.activeIE.topBorder.isOn(mascot.anchor) &&
                    mascot.anchor.x < (mascot.environment.activeIE.left+mascot.environment.activeIE.right)/2",
    );
    let right = norm_ws(
        "mascot.environment.activeIE.topBorder.isOn(mascot.anchor) &&
                    mascot.anchor.x >= (mascot.environment.activeIE.left+mascot.environment.activeIE.right)/2",
    );
    let mut ctx = MockCtx::new();
    // 既定: topBorder=on, IE 中央=(200+1200)/2=700, anchor.x=100 → 左のみ真
    assert!(eval_bool(&ctx, &left));
    assert!(!eval_bool(&ctx, &right));
    ctx.anchor_x = 800.0; // 中央より右
    assert!(!eval_bool(&ctx, &left));
    assert!(eval_bool(&ctx, &right));
}

// =====================================================================
// mascot.* 変数（number / boolean パス）
// =====================================================================

#[test]
fn numeric_mascot_paths_are_resolved_via_context() {
    let ctx = MockCtx::new();
    assert_eq!(eval_num(&ctx, "mascot.anchor.x"), 100.0);
    assert_eq!(eval_num(&ctx, "mascot.anchor.y"), 500.0);
    assert_eq!(eval_num(&ctx, "mascot.environment.screen.height/2"), 540.0);
    // 資産 WalkRightAlongFloorAndSit の TargetX: right-100-random*300 ∈ (1520, 1820]
    for _ in 0..50 {
        let v = eval_num(
            &ctx,
            "mascot.environment.workArea.right-100-Math.random()*300",
        );
        assert!((1520.0..=1820.0).contains(&v), "TargetX = {v}");
    }
    // 資産 JumpFromBottomOfIE の TargetX（括弧つき複合式）
    for _ in 0..50 {
        let v = eval_num(
            &ctx,
            "(mascot.anchor.x*3+mascot.environment.activeIE.left+Math.random()*mascot.environment.activeIE.width)/4",
        );
        assert!((125.0..375.0).contains(&v), "TargetX(IE) = {v}");
    }
}

#[test]
fn boolean_mascot_paths_are_resolved_via_context() {
    let ctx = MockCtx::new();
    assert!(eval_bool(&ctx, "mascot.lookRight"));
    // 資産 behaviors.xml「IE Is Visible」グループ条件と同形
    assert!(eval_bool(&ctx, "mascot.environment.activeIE.visible"));
    let mut ctx2 = MockCtx::new();
    ctx2.ie_visible = false;
    assert!(!eval_bool(&ctx2, "mascot.environment.activeIE.visible"));
}

#[test]
fn total_count_condition_splits_into_two() {
    // 資産 SplitIntoTwo / PullUpShimeji の条件: #{mascot.totalCount < 50}
    let ctx = MockCtx::new(); // totalCount=3
    assert!(eval_bool(&ctx, "mascot.totalCount < 50"));
    let mut ctx49 = MockCtx::new();
    ctx49.total_count = 49.0;
    assert!(eval_bool(&ctx49, "mascot.totalCount < 50"));
    let mut ctx50 = MockCtx::new();
    ctx50.total_count = 50.0;
    assert!(!eval_bool(&ctx50, "mascot.totalCount < 50"));
}

#[test]
fn cursor_y_condition_sit_and_look_at_mouse() {
    // 資産 SitAndLookAtMouse の第 1 アニメ条件
    let source = "mascot.environment.cursor.y < mascot.environment.screen.height/2";
    let ctx = MockCtx::new(); // cursor.y=200 < 540
    assert!(eval_bool(&ctx, source));
    let mut ctx2 = MockCtx::new();
    ctx2.cursor_y = 900.0;
    assert!(!eval_bool(&ctx2, source));
}

// =====================================================================
// 注入変数（FootX / TargetY / Gap）
// =====================================================================

#[test]
fn injected_footx_pinched_conditions() {
    // 資産 Pinched アニメの条件（Dragged が FootX を注入）
    let mut ctx = MockCtx::new();
    ctx.cursor_x = 200.0;
    let first = "FootX < mascot.environment.cursor.x-50";
    assert!(eval_bool_injected(&ctx, first, &[("FootX", 0.0)]));
    assert!(!eval_bool_injected(&ctx, first, &[("FootX", 200.0)]));
    // 中央域の複合条件: #{FootX >= cursor.x-10 && FootX < cursor.x+10}
    let mid = "FootX >= mascot.environment.cursor.x-10 && FootX < mascot.environment.cursor.x+10";
    assert!(eval_bool_injected(&ctx, mid, &[("FootX", 200.0)]));
    assert!(!eval_bool_injected(&ctx, mid, &[("FootX", 185.0)]));
}

#[test]
fn injected_targety_climbwall_animations() {
    // 資産 ClimbWall アニメ 2 条件（Move が TargetY を注入）
    let mut ctx = MockCtx::new(); // anchor.y=500
    ctx.anchor_y = 500.0;
    let up = "TargetY < mascot.anchor.y";
    let down = "TargetY >= mascot.anchor.y";
    assert!(eval_bool_injected(&ctx, up, &[("TargetY", 0.0)]));
    assert!(!eval_bool_injected(&ctx, up, &[("TargetY", 600.0)]));
    assert!(eval_bool_injected(&ctx, down, &[("TargetY", 600.0)]));
    assert!(!eval_bool_injected(&ctx, down, &[("TargetY", 0.0)]));
}

#[test]
fn injected_gap_composes_with_cursor() {
    // 資産 ChaseMouse の Dash TargetX: #{mascot.environment.cursor.x+Gap}
    let ctx = MockCtx::new(); // cursor.x=300
    assert_eq!(
        eval_num_injected(&ctx, "mascot.environment.cursor.x+Gap", &[("Gap", -50.0)]),
        250.0
    );
    assert_eq!(
        eval_num_injected(&ctx, "mascot.environment.cursor.x+Gap", &[("Gap", 25.5)]),
        325.5
    );
}

#[test]
fn unknown_injected_variable_is_error() {
    // 注入していない変数名（コンテキストにも存在しない識別子）は Err
    let ctx = MockCtx::new();
    let mut vars = standard_vars(); // FootX 等は入っているが "Nope" は未注入
    let v = script_var("Nope+1", true);
    assert!(vars.eval(&v, &ctx).is_err());
}

// =====================================================================
// キャッシュポリシー（Java Script.java の needsReevaluation モデル）
// - eval が評価した値はキャッシュされる（${} も #{} も同様）
// - reset_values()（フレーム開始）: #{} のみ再評価状態へ → ${} はキャッシュ維持
// - init()（アクション開始）: 全 Script を再評価状態へ
// =====================================================================

/// 値が外部から変えられる最小モック（キャッシュ検証専用）。
struct StatefulCtx {
    v: std::cell::Cell<f64>,
}

impl EvalContext for StatefulCtx {
    fn number(&self, path: &str) -> Option<f64> {
        if path == "mascot.v" {
            Some(self.v.get())
        } else {
            None
        }
    }
    fn boolean(&self, _path: &str) -> Option<bool> {
        None
    }
    fn is_on(&self, _target: &str, _x: f64, _y: f64) -> bool {
        false
    }
}

#[test]
fn cache_policy_reevaluation_gated_by_reset_and_init() {
    // Java Script.java の needsReevaluation モデル（orch 確定版）:
    // - 連続 eval では ${} も #{} もキャッシュが返る（reset/init を呼ぶまで再評価されない）
    // - reset_values()（フレーム開始）後の eval: #{} のみ再評価、${} はキャッシュのまま
    // - init()（アクション開始）後の eval: ${} も #{} も再評価される
    // EvalValue/EvalError は派生トレイトを要求しない形で値を取り出すヘルパ
    fn grab(vars: &mut Variables, ctx: &StatefulCtx, var: &Variable, label: &str) -> f64 {
        match vars.eval(var, ctx) {
            Ok(EvalValue::Number(n)) => n,
            Ok(EvalValue::Bool(_)) => panic!("{label} は数値のはずが Bool"),
            Err(_) => panic!("{label} の評価が Err になった"),
        }
    }

    let ctx = StatefulCtx {
        v: std::cell::Cell::new(1.0),
    };
    let dollar = script_var("mascot.v*2", false); // ${...}: 開始時 1 回評価
    let hash = script_var("mascot.v*3", true); // #{...}: 毎フレーム再評価
    let mut vars = Variables::new();

    // 初回 eval: 両方とも評価される
    assert_eq!(grab(&mut vars, &ctx, &dollar, "dollar"), 2.0);
    assert_eq!(grab(&mut vars, &ctx, &hash, "hash"), 3.0);

    // モック状態を変えても、reset/init なしの連続 eval ではどちらもキャッシュが返る
    ctx.v.set(10.0);
    assert_eq!(grab(&mut vars, &ctx, &dollar, "dollar"), 2.0);
    assert_eq!(grab(&mut vars, &ctx, &hash, "hash"), 3.0);

    // reset_values()（フレーム開始）: #{} のみ再評価される
    vars.reset_values();
    assert_eq!(grab(&mut vars, &ctx, &hash, "hash"), 30.0);
    // ${} はキャッシュを維持（10.0 ではなく初回の値）
    assert_eq!(grab(&mut vars, &ctx, &dollar, "dollar"), 2.0);

    // init()（アクション開始）: ${} も #{} も再評価される
    vars.init();
    assert_eq!(grab(&mut vars, &ctx, &dollar, "dollar"), 20.0);
    assert_eq!(grab(&mut vars, &ctx, &hash, "hash"), 30.0);
}

// =====================================================================
// EvalError（panic しないこと）
// =====================================================================

#[test]
fn eval_errors_are_returned_not_panicked() {
    let ctx = MockCtx::new();
    let mut vars = standard_vars();

    let bad_sources = [
        "",                           // 空式
        "mascot.environment.unknown", // 不明な mascot パス（モックは None）
        "someUnknownIdentifier",      // 不明な識別子
        "1 +",                        // 構文エラー（式が途切れ）
        "(1+2",                       // 括弧閉じ欠落
        "*3",                         // 二項演算子が式の先頭
        "1 ? 2",                      // 三項の不完全な形
        "1 <",                        // 比較の右辺欠落
    ];
    for source in bad_sources {
        let var = script_var(source, true);
        let result = vars.eval(&var, &ctx);
        assert!(result.is_err(), "式 {source:?} は Err になるべき");
    }
    // エラー後も Variables は再利用できる（panic も破壊もない）
    match vars.eval(&script_var("1+1", true), &ctx) {
        Ok(EvalValue::Number(n)) => assert_eq!(n, 2.0),
        other => panic!(
            "エラー後に正常評価できるはずが {}",
            describe_value_from(other)
        ),
    }
}

// =====================================================================
// 資産 194 式の全件評価（最重要の焼き込み）
// =====================================================================

/// conf 実物の全属性値から ${...} / #{...} 式を抽出する（資産は式が属性にのみ現れる）。
/// 式は完全包囲（先頭が ${ か #{}、末尾が }）のみ。14 件の複数行式は属性値単位で自然に取れる。
fn extract_expressions(path: &std::path::Path) -> Vec<(String, String)> {
    let text = std::fs::read_to_string(path).expect("conf ファイルが読めない");
    let doc = roxmltree::Document::parse(&text).expect("XML としてパースできる");
    let mut found = Vec::new();
    for node in doc.root_element().descendants() {
        for attr in node.attributes() {
            let value = attr.value();
            if (value.starts_with("${") || value.starts_with("#{")) && value.ends_with('}') {
                found.push((
                    format!("{}@{}", node.tag_name().name(), attr.name()),
                    value[2..value.len() - 1].to_string(),
                ));
            } else if value.contains("${") || value.contains("#{") {
                // 完全包囲でない式があるなら式リストを出して判明させる
                panic!("完全包囲でない式表記を検出: {} = {:?}", attr.name(), value);
            }
        }
    }
    found
}

#[test]
fn bake_all_194_asset_expressions_evaluate_without_error() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("conf");
    let actions = extract_expressions(&dir.join("actions.xml"));
    let behaviors = extract_expressions(&dir.join("behaviors.xml"));

    // asset-report.md §1-1: actions 141 ${} + 29 #{} = 170、behaviors 3 + 21 = 24、計 194
    assert_eq!(actions.len(), 170, "actions.xml の式数");
    assert_eq!(behaviors.len(), 24, "behaviors.xml の式数");

    let mut failures: Vec<String> = Vec::new();
    for (file, list) in [("actions.xml", &actions), ("behaviors.xml", &behaviors)] {
        for (where_, source) in list {
            let ctx = MockCtx::new();
            let mut vars = standard_vars(); // FootX/FootDX/TargetY/Gap 注入済み
            let var = script_var(source, true);
            if vars.eval(&var, &ctx).is_err() {
                failures.push(format!("{file}:{where_} #{{{source}}}"));
            }
        }
    }
    if !failures.is_empty() {
        // 対応外式のリストをテスト出力に流して判明させる
        println!("=== 評価できなかった式 ({} 件) ===", failures.len());
        for f in &failures {
            println!("{f}");
        }
    }
    assert!(
        failures.is_empty(),
        "資産 {} 式中評価不能 {} 件（上記リスト参照）",
        actions.len() + behaviors.len(),
        failures.len()
    );
}

// =====================================================================
// ActionReference 属性（attrs）の識別子空間
//
// Java ActionRef.java L66 / ActionBuilder.createVariables L486-507:
// ActionReference の全属性を子アクションの VariableMap（= 式評価の識別子空間）
// へ載せる。これにより ChaseMouse の Dash 参照が持つ Gap 属性
// （conf/actions.xml L668-671）を `#{mascot.environment.cursor.x+Gap}` から
// 解決できる。識別子解決順: Math.* → mascot.* → injected → attrs → 不明エラー。
// =====================================================================

/// Script 属性（例: "${X+1}"）を attrs に載せ、識別子 A が
/// attrs → Script → injected 変数 X と再帰解決される。
#[test]
fn attrs_script_attribute_resolves_through_injected() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.inject("X", 1.0);
    vars.set_attrs(attr_map(&[("A", "${X+1}")]));
    assert_eq!(eval_attr_num(&mut vars, &ctx, "A"), 2.0);
    assert_eq!(eval_attr_num(&mut vars, &ctx, "A+1"), 3.0);
}

/// Constant 属性（数値 / ブール）はその値を識別子として返す。
#[test]
fn attrs_constant_attributes_resolve_to_their_values() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.set_attrs(attr_map(&[("N", "2.5"), ("B", "true")]));
    assert_eq!(eval_attr_num(&mut vars, &ctx, "N"), 2.5);
    assert!(eval_attr_bool(&mut vars, &ctx, "B"));
}

/// 文字列定数（Constant::Text）は評価値（数値 / ブール）にできないため Err。
#[test]
fn attrs_text_constant_is_error() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.set_attrs(attr_map(&[("T", "hello")]));
    assert!(
        vars.eval(&script_var("T+1", true), &ctx).is_err(),
        "文字列定数の attrs は Err（数値 / ブールとして扱えない）"
    );
}

/// Java Script.java の needsReevaluation モデルを attrs 経由でも保つ:
/// `${X+1}` 由来は injected X を変えても reset まで同値、
/// `#{}` 属性は reset_values() 後に再評価、init() で全再評価。
#[test]
fn attrs_honor_java_reset_and_init_cache_semantics() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.inject("X", 1.0);
    vars.set_attrs(attr_map(&[("Dollar", "${X+1}"), ("Hash", "#{X+1}")]));

    assert_eq!(eval_attr_num(&mut vars, &ctx, "Dollar"), 2.0);
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Hash"), 2.0);

    // injected を変えても reset / init を呼ぶまでどちらもキャッシュが返る
    vars.inject("X", 10.0);
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Dollar"), 2.0);
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Hash"), 2.0);

    // reset_values()（フレーム開始）: #{} のみ再評価、${} はキャッシュ維持
    vars.reset_values();
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Hash"), 11.0);
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Dollar"), 2.0);

    // init()（アクション開始）: ${} も再評価
    vars.init();
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Dollar"), 11.0);
    assert_eq!(eval_attr_num(&mut vars, &ctx, "Hash"), 11.0);
}

/// 回帰: attrs を設定しても、injected にも attrs にも無い識別子は従来通り Err。
#[test]
fn attrs_unknown_identifier_still_errors() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.set_attrs(attr_map(&[("A", "${1+1}")]));
    assert!(
        vars.eval(&script_var("Nope+1", true), &ctx).is_err(),
        "未知識別子は attrs 導入後も Err"
    );
}

/// 回帰: 同名の識別子は injected が attrs に優先する。
#[test]
fn injected_takes_priority_over_attrs() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.inject("X", 5.0);
    vars.set_attrs(attr_map(&[("X", "${1+1}")]));
    assert_eq!(
        eval_attr_num(&mut vars, &ctx, "X"),
        5.0,
        "同名は injected が attrs より優先される"
    );
}

// =====================================================================
// #16: 再帰深さガード（自己参照 / 相互参照 / 過深チェーン）
//
// attrs の式は eval_path → Variables::eval_quiet を相互再帰して解決されるため、
// 循環参照があるとガード無しでは無限再帰しスタックオーバーフローで abort する
// （Java の StackOverflowError 相当）。深さ上限 64 超過時は既存の EvalError 経路で
// Err を返し、プロセスを落とさない。公開 API のシグネチャは不変。
// =====================================================================

/// 契約 1: 自己参照 attrs（`${X}` / `${X+1}`）はクラッシュせず Err。
#[test]
fn attrs_self_reference_errors_without_stack_overflow() {
    let ctx = MockCtx::new();

    let mut direct = Variables::new();
    direct.set_attrs(attr_map(&[("X", "${X}")]));
    assert!(
        direct.eval(&script_var("X", true), &ctx).is_err(),
        "自己参照 attrs X = ${{X}} は Err（無限再帰しない）"
    );

    let mut plus = Variables::new();
    plus.set_attrs(attr_map(&[("X", "${X+1}")]));
    assert!(
        plus.eval(&script_var("X", true), &ctx).is_err(),
        "自己参照 attrs X = ${{X+1}} は Err（無限再帰しない）"
    );
}

/// 契約 2: 相互参照 attrs（A → B → A）は Err。
#[test]
fn attrs_mutual_reference_errors_without_stack_overflow() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.set_attrs(attr_map(&[("A", "${B}"), ("B", "${A}")]));
    assert!(
        vars.eval(&script_var("A", true), &ctx).is_err(),
        "相互参照 attrs A=${{B}}, B=${{A}} は Err（無限再帰しない）"
    );
}

/// 契約 3: 正当な参照チェーンは従来どおり解決する（回帰）。
/// attrs 10 段連鎖と、attrs → injected / mascot への参照を確認する。
#[test]
fn attrs_valid_chain_still_resolves() {
    let ctx = MockCtx::new();

    // A0=${A1+1}, A1=${A2+1}, ..., A8=${A9+1}, A9=${X}（injected X=1）→ A0 = 10
    let mut pairs: Vec<(String, String)> = Vec::new();
    for i in 0..9 {
        pairs.push((format!("A{i}"), format!("${{A{}+1}}", i + 1)));
    }
    pairs.push(("A9".to_string(), "${X}".to_string()));
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();

    let mut vars = Variables::new();
    vars.inject("X", 1.0);
    vars.set_attrs(attr_map(&refs));
    assert_eq!(
        eval_attr_num(&mut vars, &ctx, "A0"),
        10.0,
        "10 段の attrs 連鎖が解決される"
    );

    // attrs から mascot 変数への参照（injected 経由は上の連鎖と既存テストで担保）
    let mut vars2 = Variables::new();
    vars2.set_attrs(attr_map(&[("Pos", "${mascot.anchor.x+1}")]));
    assert_eq!(
        eval_attr_num(&mut vars2, &ctx, "Pos"),
        101.0,
        "attrs → mascot 参照が解決される"
    );
}

/// 契約 4: 深さ超過 Err の後でも同じ Variables が壊れない。
/// 深いチェーン（上限超過）を一度評価した直後、同じ Variables で別式を正常評価できる
/// ことを pin する（再帰深さカウンタが復元され、状態が壊れない）。
#[test]
fn variables_recover_after_depth_limit_error() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.inject("X", 1.0);

    // 200 段チェーン（上限 64 超過）。ガード実装後は Err、未実装なら Ok だが、
    // 結果自体は問わず「直後の評価が成功すること」が本テストの契約。
    let depth = 200usize;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for i in 0..depth - 1 {
        pairs.push((format!("D{i}"), format!("${{D{}+1}}", i + 1)));
    }
    pairs.push((format!("D{}", depth - 1), "${X}".to_string()));
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    vars.set_attrs(attr_map(&refs));

    let _ = vars.eval(&script_var("D0", true), &ctx);

    // カウンタが復元されていなければ、以降の評価も深さ超過で Err になる。
    assert_eq!(
        eval_attr_num(&mut vars, &ctx, "1+1"),
        2.0,
        "深さ超過 Err の後も別式を正常評価できる（カウンタ復元）"
    );
}

/// 契約 5: 過度に深いチェーン（500 段）は上限 64 超過として Err を返す
/// （スタックオーバーフローで abort しない）。
#[test]
fn attrs_overly_deep_chain_is_error_not_overflow() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();
    vars.inject("X", 1.0);

    let depth = 500usize;
    let mut pairs: Vec<(String, String)> = Vec::new();
    for i in 0..depth - 1 {
        pairs.push((format!("D{i}"), format!("${{D{}+1}}", i + 1)));
    }
    pairs.push((format!("D{}", depth - 1), "${X}".to_string()));
    let refs: Vec<(&str, &str)> = pairs
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    vars.set_attrs(attr_map(&refs));

    assert!(
        vars.eval(&script_var("D0", true), &ctx).is_err(),
        "500 段チェーンは深さ上限超過で Err（abort しない）"
    );
}

// =====================================================================
// #21: 式 AST 深さガード（評価・パースの再帰で abort しない）
//
// #16 の MAX_EVAL_DEPTH は attrs 相互再帰（eval_quiet）専用。式そのものの AST が
// 深い場合（長大な加算チェーンの左深 AST・深い括弧ネスト）は、評価器 Interp::eval
// やパーサ Parser の再帰がスタックオーバーフローを起こす。深さ上限超過は
// 既存の EvalError 経路で Err を返し、プロセスを落とさない（公開 API は不変）。
// 通常規模の式は従来どおり成功する。
// =====================================================================

/// 契約 1: 長大な加算チェーン（左深 AST）は abort せず Err。
/// pre-fix は評価の再帰が項数分だけ深くなり、スタックオーバーフローで abort する。
#[test]
fn long_additive_chain_is_error_not_stack_overflow() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();

    const TERMS: usize = 200_000;
    let mut source = String::with_capacity(TERMS * 2);
    source.push('1');
    for _ in 1..TERMS {
        source.push_str("+1");
    }

    let var = script_var(&source, true);
    assert!(
        vars.eval(&var, &ctx).is_err(),
        "20 万項の加算チェーンは深さ上限超過で Err（abort しない）"
    );
}

/// 契約 2: 深い括弧ネストはパース段の再帰で abort せず Err。
#[test]
fn deep_parenthesis_nesting_is_error_not_stack_overflow() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();

    const DEPTH: usize = 50_000;
    let source = format!("{}1{}", "(".repeat(DEPTH), ")".repeat(DEPTH));

    let var = script_var(&source, true);
    assert!(
        vars.eval(&var, &ctx).is_err(),
        "5 万段の括弧ネストはパース段の深さ上限超過で Err（abort しない）"
    );
}

/// 契約 3: 通常規模（数十ノード）の式は従来どおり Ok（ガードが誤検知しない回帰）。
#[test]
fn normal_sized_expression_still_evaluates() {
    let ctx = MockCtx::new();
    let mut vars = Variables::new();

    // 50 項の加算（数十ノード）。1+2+...+50 = 1275
    let source = (1..=50)
        .map(|n| n.to_string())
        .collect::<Vec<_>>()
        .join("+");
    let var = script_var(&source, true);
    match vars.eval(&var, &ctx) {
        Ok(EvalValue::Number(n)) => assert_eq!(n, 1275.0),
        Ok(other) => panic!("数値のはずが {}", describe_value(&other)),
        Err(e) => panic!("通常規模の式は Ok のはずが Err: {e:?}"),
    }
}

// =====================================================================
// attrs テスト用ヘルパ（このセクション専用）
// =====================================================================

/// (名前, 生の属性値) から VarMap を作る（Variable::parse で Script/Constant 化）。
fn attr_map(pairs: &[(&str, &str)]) -> VarMap {
    let mut map = BTreeMap::new();
    for (name, value) in pairs {
        map.insert((*name).to_string(), Variable::parse(value));
    }
    map
}

/// 永続 `Variables` で式を数値評価する（トップレベルは #{} として評価）。
fn eval_attr_num(vars: &mut Variables, ctx: &MockCtx, source: &str) -> f64 {
    match vars.eval(&script_var(source, true), ctx) {
        Ok(EvalValue::Number(n)) => n,
        Ok(EvalValue::Bool(_)) => panic!("式 {source:?} は数値のはずが Bool"),
        Err(e) => panic!("式 {source:?} の評価が Err: {e:?}"),
    }
}

/// 永続 `Variables` で式をブール評価する。
fn eval_attr_bool(vars: &mut Variables, ctx: &MockCtx, source: &str) -> bool {
    match vars.eval(&script_var(source, true), ctx) {
        Ok(EvalValue::Bool(b)) => b,
        Ok(EvalValue::Number(_)) => panic!("式 {source:?} はブールのはずが Number"),
        Err(e) => panic!("式 {source:?} の評価が Err: {e:?}"),
    }
}
