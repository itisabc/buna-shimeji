//! タスク #9e: src/win/window.rs のスタイル矯正純粋関数の契約テスト。
//!
//! 検証対象は coder が実装する公開 API（シグネチャ固定）のみ:
//! - `correct_style(style: isize) -> isize`:
//!   入力 style から装飾系 6 ビット（WS_CAPTION / WS_SYSMENU / WS_MAXIMIZEBOX /
//!   WS_MINIMIZEBOX / WS_THICKFRAME / WS_GROUP）を除去し、
//!   WS_POPUP / WS_VISIBLE / WS_CLIPSIBLINGS を付与する。
//!   装飾マスク外のビットは極力保持する。
//!   挙動 pin: src/win/window.rs の既存 unsafe ブロック内 popup 矯正
//!   （build_layered_tao_window・popup_style 計算部）と同一の演算結果。
//! - `correct_exstyle(exstyle: isize) -> isize`:
//!   WS_EX_APPWINDOW（0x00040000）を除去し WS_EX_TOOLWINDOW（0x00000080）を
//!   付与する。他のビットは保持し、WS_EX_LAYERED の付与は含めない
//!   （build 側で先に OR 済みの値に適用される前提）。
//!
//! ビット定数は windows クレートの型変換を避けるため数値リテラルで pin する
//! （Win32: WS_POPUP=0x80000000, WS_VISIBLE=0x10000000, WS_CLIPSIBLINGS=0x04000000,
//! WS_CAPTION=0x00C00000, WS_SYSMENU=0x00080000, WS_THICKFRAME=0x00040000,
//! WS_MINIMIZEBOX=WS_GROUP=0x00020000（同値）, WS_MAXIMIZEBOX=0x00010000）。
//! 既存インライン矯正式との同一性は、同式を windows 定数で再構成した
//! 参照式との照合で直接 pin する。
//!
//! TDD RED: correct_style / correct_exstyle は未実装のため
//! 「解決できない名前」のコンパイルエラーになることが正常。

use simeji::win::window::{correct_exstyle, correct_style};

use windows::Win32::UI::WindowsAndMessaging::{
    WS_CAPTION, WS_CLIPSIBLINGS, WS_GROUP, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU,
    WS_THICKFRAME, WS_VISIBLE,
};

// =====================================================================
// 契約定数（テスト側の対照表・数値リテラル pin）
// =====================================================================

/// 除去すべき装飾系 6 ビットの合成マスク
///（CAPTION|SYSMENU|MAXIMIZEBOX|MINIMIZEBOX|THICKFRAME|GROUP。MINIMIZEBOX と
/// GROUP は同一値 0x00020000 のため実質 5 ビット幅・6 定義名）。
const DECOR_MASK: isize = 0x00CF_0000;

/// 付与すべき 3 ビットの合成マスク（POPUP|VISIBLE|CLIPSIBLINGS）。
const ADD_MASK: isize = 0x9400_0000;

// =====================================================================
// 契約 1: correct_style は装飾系 6 ビットを除去する
// =====================================================================

/// 装飾系 6 ビットを含む入力から、6 ビットすべてが除去される。
#[test]
fn correct_style_strips_all_six_decoration_bits() {
    // CAPTION|SYSMENU|THICKFRAME|GROUP(MINIMIZEBOX)|MAXIMIZEBOX の合成値のみの入力
    let out = correct_style(DECOR_MASK);
    assert_eq!(out & DECOR_MASK, 0, "装飾系 6 ビットはすべて除去される");

    // 装飾 + ユーザービット混在入力でも装飾のみ落ちる
    let mixed = DECOR_MASK | 0x4B00_000F; // + CHILD|DISABLED|MAXIMIZE|CLIPCHILDREN|下位 4 ビット
    assert_eq!(
        correct_style(mixed) & DECOR_MASK,
        0,
        "混在入力でも装飾系 6 ビットのみ除去される"
    );
}

// =====================================================================
// 契約 2: correct_style は POPUP / VISIBLE / CLIPSIBLINGS を付与する
// =====================================================================

/// 装飾を含まない入力 0 には POPUP|VISIBLE|CLIPSIBLINGS が「ちょうど」付く
///（これ以外のビットを新たに付け加えないことの pin）。
#[test]
fn correct_style_on_zero_adds_exactly_popup_visible_clipsiblings() {
    assert_eq!(correct_style(0), ADD_MASK);
}

// =====================================================================
// 契約 3: 装飾マスク外のビットは保持される
// =====================================================================

/// 装飾を含まない入力に対しては 出力 = 入力 | 付与マスク
///（WS_CHILD / WS_MAXIMIZE / WS_CLIPCHILDREN / WS_DISABLED / 下位汎用ビットの保持）。
#[test]
fn correct_style_preserves_non_decoration_bits() {
    let user_bits = 0x4B00_000F; // CHILD|DISABLED|MAXIMIZE|CLIPCHILDREN|下位 4 ビット
    assert_eq!(correct_style(user_bits), user_bits | ADD_MASK);
}

// =====================================================================
// 契約 4: 既存インライン popup 矯正（window.rs L133-140 相当）と同一演算
// =====================================================================

/// 既存 unsafe ブロック内の popup 矯正式を windows 定数でそのまま再構成した
/// 参照式（src/win/window.rs build_layered_tao_window の popup_style 計算と同一）。
fn reference_correct_style(style: isize) -> isize {
    (style
        & !(WS_CAPTION.0
            | WS_SYSMENU.0
            | WS_MAXIMIZEBOX.0
            | WS_MINIMIZEBOX.0
            | WS_THICKFRAME.0
            | WS_GROUP.0) as isize)
        | (WS_POPUP.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0) as isize
}

/// correct_style は参照式と全入力で一致する（インライン式からの抽出で
/// 演算が変わっていないことの直接 pin）。
#[test]
fn correct_style_equals_existing_inline_correction_expression() {
    let inputs: [isize; 5] = [0, DECOR_MASK, 0x15CF_0000, 0x4B00_000F, 0x7FFF_FFFF];
    for input in inputs {
        assert_eq!(
            correct_style(input),
            reference_correct_style(input),
            "input={input:#x}"
        );
    }
}

/// tao の非装飾窓が残しがちなスタイル（装飾 6 種 + VISIBLE + CLIPSIBLINGS +
/// MAXIMIZE）を模した焼き込み値:
/// 入力 0x15CF_0000 = MAXIMIZE(0x0100_0000) | VISIBLE(0x1000_0000) |
/// CLIPSIBLINGS(0x0400_0000) | CAPTION(0x00C0_0000) | SYSMENU(0x0008_0000) |
/// THICKFRAME(0x0004_0000) | GROUP/MINIMIZEBOX(0x0002_0000) | MAXIMIZEBOX(0x0001_0000)
/// → 期待 0x9500_0000 = POPUP | VISIBLE | CLIPSIBLINGS | MAXIMIZE。
#[test]
fn correct_style_burned_in_tao_like_value() {
    assert_eq!(correct_style(0x15CF_0000), 0x9500_0000);
}

// =====================================================================
// 契約 5: correct_style の冪等性
// =====================================================================

/// 2 回適用しても 1 回適用と同じ結果（既に矯正済みの style は不変）。
#[test]
fn correct_style_is_idempotent() {
    let inputs: [isize; 7] = [
        0,
        DECOR_MASK,
        0x15CF_0000,
        0x9500_0000,
        0xDF00_000F,
        0x4B00_000F,
        0x8000_0000,
    ];
    for input in inputs {
        let once = correct_style(input);
        assert_eq!(correct_style(once), once, "input={input:#x}");
    }
}

// =====================================================================
// 契約 6: correct_exstyle は APPWINDOW を除去し TOOLWINDOW を付与する
// =====================================================================

/// 実測値（NOACTIVATE(0x08000000) | LAYERED(0x00080000) | APPWINDOW(0x00040000) |
/// 0x10 | TOPMOST(0x8) の合成値 0x080C_0018）→ APPWINDOW が除去され
/// TOOLWINDOW が付与され 0x0808_0098。
#[test]
fn correct_exstyle_real_measured_value_replaces_appwindow_with_toolwindow() {
    assert_eq!(correct_exstyle(0x080C_0018), 0x0808_0098);
}

/// APPWINDOW を含まない入力には TOOLWINDOW だけが追加される。
/// 入力 0 → ちょうど 0x80（WS_EX_LAYERED=0x00080000 の付与は含めないことの pin）。
#[test]
fn correct_exstyle_on_zero_adds_only_toolwindow() {
    assert_eq!(correct_exstyle(0), 0x80);
    assert_eq!(correct_exstyle(0x0004_0000), 0x80, "APPWINDOW のみの入力");
}

/// APPWINDOW を含まない既存 exstyle は TOOLWINDOW 付与以外は完全に保持される
///（出力 = 入力 | 0x80）。
#[test]
fn correct_exstyle_preserves_other_bits() {
    // NOACTIVATE|LAYERED|0x10|TOPMOST
    assert_eq!(correct_exstyle(0x0808_0018), 0x0808_0098);
    // LAYERED|TOPMOST
    assert_eq!(correct_exstyle(0x0008_0008), 0x0008_0088);
}

// =====================================================================
// 契約 7: correct_exstyle の冪等性
// =====================================================================

/// 2 回適用しても 1 回適用と同じ結果（既に矯正済みの exstyle は不変）。
#[test]
fn correct_exstyle_is_idempotent() {
    let inputs: [isize; 6] = [0, 0x80, 0x0004_0000, 0x080C_0018, 0x0808_0098, 0x0008_0000];
    for input in inputs {
        let once = correct_exstyle(input);
        assert_eq!(correct_exstyle(once), once, "input={input:#x}");
    }
}
