//! セル方式（`.tmp/design-draw-batching.md`）の幾何契約テスト。
//!
//! 窓は「最大フレーム + 2M」のセルで固定し、sprite はセル内ローカル座標に描く。
//! sprite が余白 `[0, 2M]` を外れたら再センターする。ここでは Win32 を伴わない
//! 純関数（`cell_local` / `needs_recenter` / `recentered_origin`）と
//! `ImageSet::max_frame_size` の契約を検証する。

use std::collections::BTreeMap;

use shimeji::render::imageset::{Frame, ImageSet};
use shimeji::render::{cell_local, is_unchanged, needs_recenter, recentered_origin, ImageKey};
use shimeji::win::window::CELL_MARGIN;

fn frame(width: u32, height: u32) -> Frame {
    Frame {
        width,
        height,
        argb: Vec::new(),
    }
}

fn set_with(frames: Vec<(&str, Frame)>) -> ImageSet {
    ImageSet {
        name: "test".to_string(),
        frames: frames
            .into_iter()
            .map(|(name, f)| (name.to_string(), f))
            .collect::<BTreeMap<_, _>>(),
        warnings: Vec::new(),
        scale: 1.0,
    }
}

// =====================================================================
// 契約: max_frame_size は最大フレーム寸法（空 set は (0,0)）
// =====================================================================

#[test]
fn max_frame_size_returns_largest_dimensions() {
    let set = set_with(vec![
        ("a.png", frame(128, 128)),
        ("b.png", frame(160, 96)),
        ("c.png", frame(64, 200)),
    ]);
    assert_eq!(set.max_frame_size(), (160, 200));
}

#[test]
fn max_frame_size_empty_set_is_zero() {
    let set = set_with(Vec::new());
    assert_eq!(
        set.max_frame_size(),
        (0, 0),
        "空 set は (0,0)（呼び出し側で clamp）"
    );
}

// =====================================================================
// 契約: セル内ローカル座標と再センター判定
// =====================================================================

#[test]
fn cell_local_is_sprite_minus_origin() {
    assert_eq!(cell_local((110, 60), (100, 50)), (10, 10));
    assert_eq!(cell_local((90, 40), (100, 50)), (-10, -10));
}

#[test]
fn needs_recenter_boundaries_are_inclusive() {
    let margin = CELL_MARGIN as i32;
    let hi = 2 * margin;
    assert!(!needs_recenter((0, 0), margin), "下端/左端は許容");
    assert!(!needs_recenter((hi, hi), margin), "上端/右端は許容");
    assert!(needs_recenter((-1, 0), margin));
    assert!(needs_recenter((0, -1), margin));
    assert!(needs_recenter((hi + 1, 0), margin));
    assert!(needs_recenter((0, hi + 1), margin));
}

#[test]
fn recentered_origin_puts_sprite_at_margin() {
    let margin = CELL_MARGIN as i32;
    let sprite = (1234, -567);
    let origin = recentered_origin(sprite, margin);
    assert_eq!(cell_local(sprite, origin), (margin, margin));
}

/// property: 任意の sprite 移動列（1px 歩行・flip/フレーム幅ジャンプ・高速移動・落下）
/// でも、セル方針適用後の local は常に `[0, 2M]` に収まる。
#[test]
fn cell_policy_keeps_local_within_margin() {
    let margin = CELL_MARGIN as i32;
    let hi = 2 * margin;

    let mut sprites: Vec<(i32, i32)> = Vec::new();
    let mut x = 100;
    let y = 50;
    for _ in 0..80 {
        x += 1; // 1px 歩行
        sprites.push((x, y));
    }
    x += 128; // flip によるオフセット反転相当のジャンプ
    sprites.push((x, y));
    x += 96; // フレーム幅変更相当
    sprites.push((x, y));
    for _ in 0..40 {
        x -= 3; // 逆向き高速移動
        sprites.push((x, y));
    }
    sprites.push((x, y + 400)); // 落下

    let mut origin: Option<(i32, i32)> = None;
    let mut recenters = 0usize;
    for sprite in sprites {
        let next = match origin {
            Some(o) if !needs_recenter(cell_local(sprite, o), margin) => o,
            _ => {
                recenters += 1;
                recentered_origin(sprite, margin)
            }
        };
        origin = Some(next);
        let local = cell_local(sprite, next);
        assert!(
            (0..=hi).contains(&local.0) && (0..=hi).contains(&local.1),
            "local={local:?} が [0,{hi}] を外れた（sprite={sprite:?}）"
        );
    }
    assert!(recenters >= 1, "初回は必ず再センターする");
}

// =====================================================================
// 契約: 再描画要否は窓位置も含める（高速移動で sprite が凍り付くバグの回帰）
// =====================================================================

/// 内容とセル内ローカル位置が同じでも、窓位置が違えば描画が必要。
/// 高速移動では毎 tick 再センターして local が同じ `M` に戻るため、位置を見ないと
/// 窓移動の ULW をスキップし、sprite がその場で凍り付く（実機で確認・2026-09-20）。
#[test]
fn is_unchanged_requires_same_window_origin() {
    let key = ImageKey {
        image_ref: "shime1.png".to_string(),
        flip: false,
        width: 128,
        height: 128,
    };
    let local = (CELL_MARGIN as i32, CELL_MARGIN as i32);

    // 窓位置だけ違う（再センター直後で local は同じ M）→ 描画必要
    assert!(
        !is_unchanged(
            Some(&key),
            Some(local),
            Some((984, 984)),
            &key,
            local,
            (1016, 984),
            false
        ),
        "窓位置が変われば描画必要（凍り付き防止）"
    );

    // 内容・ローカル・窓位置のすべてが同じ → 不要
    assert!(is_unchanged(
        Some(&key),
        Some(local),
        Some((984, 984)),
        &key,
        local,
        (984, 984),
        false
    ));

    // 寸法ドリフト修復時は常に描画必要
    assert!(
        !is_unchanged(
            Some(&key),
            Some(local),
            Some((984, 984)),
            &key,
            local,
            (984, 984),
            true
        ),
        "size_drift は描画必要"
    );

    // 初回（前回なし）は描画必要
    assert!(!is_unchanged(
        None,
        None,
        None,
        &key,
        local,
        (984, 984),
        false
    ));
}
