//! タスク #5: src/render/mod.rs の描画補助関数の契約テスト。
//!
//! 検証対象は coder が実装する公開 API（シグネチャ固定）のみ:
//! - `flipped_offset_x(frame_width: u32, pose_dx: i32) -> i32`:
//!   ImagePairs.java L85 の右画像アンカー相当 `width - dx`（dx は負でもそのまま演算）
//!
//! タスク #31 で `compose_argb` は撤廃され、フレームはロード時にプレマルチプライド
//! ARGB へ 1 回変換される（`from_rgba` / `blit_argb` の契約テストは
//! tests/frame_argb_test.rs が担う）。MascotView（Win32 ウィンドウ制御）と
//! DrawAction 等の enum 判定はテストしない。

use shimeji::render::flipped_offset_x;

// =====================================================================
// 契約: flipped_offset_x は ImagePairs.java L85 相当 `width - dx`
// =====================================================================

/// 焼き込み値（ImagePairs.java L85: `rightImage.getWidth() - scaledAnchorX`）。
/// dx は負でもそのまま演算する（Java int 演算相当）。
#[test]
fn flipped_offset_x_is_width_minus_dx() {
    assert_eq!(flipped_offset_x(128, 64), 64);
    assert_eq!(flipped_offset_x(128, 96), 32);
    assert_eq!(flipped_offset_x(128, 0), 128);
    assert_eq!(flipped_offset_x(128, 128), 0);
    // 負の dx もそのまま width - dx
    assert_eq!(flipped_offset_x(100, -20), 120);
}
