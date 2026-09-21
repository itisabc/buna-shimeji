//! タスク #5: src/render/mod.rs の描画補助関数の契約テスト。
//!
//! 検証対象は coder が実装する公開 API（シグネチャ固定）のみ:
//! - `flipped_offset_x(frame_width: u32, pose_dx: i32) -> i32`:
//!   ImagePairs.java L85 の右画像アンカー相当 `width - dx`（dx は負でもそのまま演算）
//! - `is_unchanged(...)`: 再描画要否（窓位置を含む・「投げると凍る」の回帰）
//!
//! タスク #31 で `compose_argb` は撤廃され、フレームはロード時にプレマルチプライド
//! ARGB へ 1 回変換される（`from_rgba` / `blit_argb` の契約テストは
//! tests/frame_argb_test.rs が担う）。MascotView（Win32 ウィンドウ制御）はテストしない。

use shimeji::render::{flipped_offset_x, is_unchanged, ImageKey};

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

// =====================================================================
// 契約: 再描画要否は窓位置も含める（移動で sprite が凍り付くバグの回帰）
// =====================================================================

fn key() -> ImageKey {
    ImageKey {
        image_ref: "shime1.png".to_string(),
        flip: false,
        width: 128,
        height: 128,
        tint: None,
        glow: 0,
    }
}

/// tint が違えば内容が違う＝描画が必要（色を毎 tick 変えるモードの前提）。
#[test]
fn is_unchanged_distinguishes_tint() {
    let key = key();
    let tinted = ImageKey {
        tint: Some([255, 128, 0]),
        ..key.clone()
    };
    assert!(
        !is_unchanged(Some(&key), Some((984, 984)), &tinted, (984, 984), false),
        "tint が変われば描画必要"
    );
    assert!(
        !is_unchanged(Some(&tinted), Some((984, 984)), &key, (984, 984), false),
        "tint が外れたときも描画必要"
    );
    assert!(
        is_unchanged(Some(&key), Some((984, 984)), &key, (984, 984), false),
        "tint が同じなら不要"
    );
}

/// グロー強度が違えば内容が違う＝描画が必要。同じなら再送しない
/// （ゲーミングの「毎 tick 回る」再描画は tint 側だけで起きる）。
#[test]
fn is_unchanged_distinguishes_glow() {
    let key = key();
    let glowing = ImageKey {
        tint: Some([255, 128, 0]),
        glow: 143,
        ..key.clone()
    };
    let same = ImageKey {
        tint: Some([255, 128, 0]),
        glow: 143,
        ..key.clone()
    };
    let dimmer = ImageKey {
        tint: Some([255, 128, 0]),
        glow: 51,
        ..key.clone()
    };

    assert!(
        !is_unchanged(Some(&glowing), Some((984, 984)), &dimmer, (984, 984), false),
        "グロー強度が変われば描画必要"
    );
    assert!(
        is_unchanged(Some(&glowing), Some((984, 984)), &same, (984, 984), false),
        "グローが同じなら不要"
    );
}

#[test]
fn is_unchanged_requires_same_window_origin() {
    let key = key();

    // 内容が同じでも窓位置が違う（投げ = 毎 tick 移動）→ 描画必要
    assert!(
        !is_unchanged(Some(&key), Some((984, 984)), &key, (1016, 984), false),
        "窓位置が変われば描画必要（凍り付き防止）"
    );

    // 内容・窓位置とも同じ → 不要
    assert!(is_unchanged(
        Some(&key),
        Some((984, 984)),
        &key,
        (984, 984),
        false
    ));

    // 内容が変わった → 必要
    let other = ImageKey {
        image_ref: "shime2.png".to_string(),
        ..key.clone()
    };
    assert!(!is_unchanged(
        Some(&key),
        Some((984, 984)),
        &other,
        (984, 984),
        false
    ));

    // 寸法ドリフト修復時は常に描画必要
    assert!(
        !is_unchanged(Some(&key), Some((984, 984)), &key, (984, 984), true),
        "size_drift は描画必要"
    );

    // 初回（前回なし）は描画必要
    assert!(!is_unchanged(None, None, &key, (984, 984), false));
}
