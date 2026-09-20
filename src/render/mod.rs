//! 描画レイヤ。タスク #5: 描画本体（MascotView）。
//!
//! design.md §3-9 の needsRepaint 管理: 画像変化時のみ UpdateLayeredWindow
//! で再描画し、位置不変なら移動のみ（同一内容の移動はピクセル同等）。両者
//! 不変なら何もしない。Java 正本は `MascotImage`（center = アンカー Point）と
//! `WindowsTranslucentWindow`（setImage = フィールド代入・updateImage =
//! validate + repaint 相当）。
//!
//! - [`flipped_offset_x`]: Java ImagePairs.java L85-91 の右画像アンカー相当
//!   `width - dx`（dx は負でもそのまま i32 演算）
//! - [`MascotView`]: 透過ウィンドウ 1 枚の再描画・移動の状態機械
//!
//! フレームはロード時にプレマルチプライド 0xAARRGGBB へ変換済み
//!（[`crate::render::imageset::Frame::argb`]）。描画時の flip はセル DIB への
//! 転送コピー（[`crate::win::window::blit_argb_at`]）に融合する。
//!
//! # セル方式（描画コスト削減・`.tmp/design-draw-batching.md`）
//!
//! 窓は sprite ちょうどではなく「sprite がセル内を動ける余白 `CELL_MARGIN` を
//! 加えたセル寸法」（最大フレーム + 2M）で固定する。sprite の移動はセル内の
//! ローカル座標（blit 位置）で表現し、sprite が余白を割ったときだけ窓を再センター
//! する（`SetWindowPos` を毎 tick 呼ばない）。
//!
//! **位置と内容は 1 回の ULW で原子的に反映する**（[`LayeredWindow::present`] に
//! 目標座標を渡す）。窓を先に移動すると「旧内容のまま新位置」が 1 フレーム見え、
//! 再センターのたびに位置がズレて戻る（実機で確認・2026-09-20）。DIB への blit は
//! 表示に影響しないため、blit → 位置指定 ULW の順で glitch なく更新できる。
//!
//! 不変条件: `local = sprite_origin - cell_origin` が `[0, 2M]`。

pub mod imageset;

use tao::dpi::PhysicalSize;
// EventLoopWindowTarget で受ける（tao 0.37 の WindowBuilder::build が target を要求。
// イベントハンドラ内での窓生成（#10b-2c の view 補充）に対応。EventLoop は Deref するため
// main 側の &EventLoop 直渡しも引き続き動く）
use tao::event_loop::EventLoopWindowTarget;
use tao::window::Window;

use crate::render::imageset::Frame;
use crate::win::window::{LayeredWindow, WindowError, CELL_MARGIN};

/// 最後に描画した画像を識別するキー（image_ref + flip + 寸法の同一性のみ。
/// ピクセル内容は比較しない — 同一 image_ref のフレーム内容は実行中に不変）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageKey {
    pub image_ref: String,
    pub flip: bool,
    pub width: u32,
    pub height: u32,
}

/// [`MascotView::draw`] へ渡す sprite の描画パラメータ（引数過多を避ける束ね）。
#[derive(Debug, Clone, Copy)]
pub struct SpriteDraw<'a> {
    pub image_ref: &'a str,
    pub frame: &'a Frame,
    pub flip: bool,
    /// flip 前のポーズアンカー dx/dy。
    pub pose_anchor: (i32, i32),
    /// アンカー点のスクリーン座標。
    pub anchor_pos: (i32, i32),
    /// セル寸法の基礎（`ImageSet::max_frame_size`）。
    pub max_frame: (u32, u32),
}

/// flip 時の水平描画オフセット（Java ImagePairs.java L85:
/// `rightImage.getWidth() - scaledAnchorX` 相当）。
///
/// dx は負でも `width - dx` のまま i32 演算する（u32 減算・飽和はしない）。
pub fn flipped_offset_x(frame_width: u32, pose_dx: i32) -> i32 {
    frame_width as i32 - pose_dx
}

/// sprite 左上のスクリーン座標から、セル原点に対するローカル座標を求める。
pub fn cell_local(sprite_origin: (i32, i32), cell_origin: (i32, i32)) -> (i32, i32) {
    (
        sprite_origin.0 - cell_origin.0,
        sprite_origin.1 - cell_origin.1,
    )
}

/// 再センターが必要か（ローカル座標がセルの余白域 `[0, 2M]` を外れた）。
pub fn needs_recenter(local: (i32, i32), margin: i32) -> bool {
    let hi = 2 * margin;
    local.0 < 0 || local.0 > hi || local.1 < 0 || local.1 > hi
}

/// 再センター後のセル原点（sprite を余白中央 `M` に置く）。
pub fn recentered_origin(sprite_origin: (i32, i32), margin: i32) -> (i32, i32) {
    (sprite_origin.0 - margin, sprite_origin.1 - margin)
}

/// 内容・セル内位置・窓位置がすべて前回と同じで、寸法ドリフトも無ければ描画不要。
///
/// 窓位置（`origin`）を含めるのが必須: 高速移動では毎 tick 再センターして
/// `local` が同じ `M` に戻るため、位置を見ないと「内容が同じ = 不要」と誤判定し、
/// 窓を動かす ULW をスキップして sprite がその場に凍り付く（実機で確認・2026-09-20）。
pub fn is_unchanged(
    last_image: Option<&ImageKey>,
    last_local: Option<(i32, i32)>,
    last_origin: Option<(i32, i32)>,
    key: &ImageKey,
    local: (i32, i32),
    origin: (i32, i32),
    size_drift: bool,
) -> bool {
    !size_drift
        && last_image == Some(key)
        && last_local == Some(local)
        && last_origin == Some(origin)
}

/// 透過ウィンドウ 1 枚と「最後に描画した内容」の状態を保持するビュー。
///
/// 窓はセル寸法（最大フレーム + 2M）で固定し、sprite はセル内ローカル座標に
/// blit する。sprite がセルの余白を割ったときだけ窓を再センターする。
pub struct MascotView {
    window: LayeredWindow,
    /// 現在のセル寸法（0 = 未確定）。
    cell_size: (u32, u32),
    /// 現在のセル原点（ULW の pptDst。None = 未確定 → 次 `draw` で再センター）。
    cell_origin: Option<(i32, i32)>,
    last_image: Option<ImageKey>,
    last_local: Option<(i32, i32)>,
    /// 最後に ULW で反映した窓位置（`origin`）。窓は ULW でしか動かないため、
    /// `cell_origin`（目標）と一致していない間は必ず present する。
    last_origin: Option<(i32, i32)>,
}

impl MascotView {
    /// 透過ウィンドウを生成する（tao ウィンドウ生成・WS_EX_LAYERED 付与・
    /// WS_POPUP 矯正は `LayeredWindow::create` 内部で実施済み）。
    /// `window_target` は [`tao::event_loop::EventLoopWindowTarget`]
    /// （イベントハンドラ内での生成に対応・win/window.rs doc 参照）。
    pub fn create<T: 'static>(
        window_target: &EventLoopWindowTarget<T>,
        width: u32,
        height: u32,
    ) -> Result<Self, WindowError> {
        let window = LayeredWindow::create(window_target, width, height)?;
        Ok(MascotView {
            window,
            cell_size: (0, 0),
            cell_origin: None,
            last_image: None,
            last_local: None,
            last_origin: None,
        })
    }

    /// 内包する tao ウィンドウ（位置取得・イベント照合などに使用）。
    pub fn window(&self) -> &Window {
        self.window.window()
    }

    /// セル幾何を決め、必要なら再センターし、セル DIB へ blit したうえで
    /// **位置と内容を 1 回の ULW で原子的に**反映する。
    ///
    /// - `sprite.max_frame`: `ImageSet::max_frame_size()`（セル寸法の基礎）。
    /// - `sprite.pose_anchor`: ポーズのアンカー dx/dy（flip 前）。`sprite.anchor_pos` は
    ///   アンカー点のスクリーン座標。sprite 左上 = `anchor_pos - offset`。
    ///
    /// 窓物理寸法がセルとずれたら（初回・set 変更・WM_DPICHANGED の自己修復）窓と
    /// DIB をセル寸法へ戻し、内容を再描画する。内容・位置とも不変なら何もしない。
    pub fn draw(&mut self, sprite: SpriteDraw<'_>) -> Result<(), WindowError> {
        let max_frame = sprite.max_frame;
        let frame = sprite.frame;
        let cell = (
            max_frame.0.max(1) + 2 * CELL_MARGIN,
            max_frame.1.max(1) + 2 * CELL_MARGIN,
        );

        let offset_x = if sprite.flip {
            flipped_offset_x(frame.width, sprite.pose_anchor.0)
        } else {
            sprite.pose_anchor.0
        };
        let sprite_origin = (
            sprite.anchor_pos.0 - offset_x,
            sprite.anchor_pos.1 - sprite.pose_anchor.1,
        );

        // セル / 窓の寸法合わせ（初回・set 変更・DPI drift の自己修復）。
        // 「窓サイズ == セルサイズ」が不変条件（frame サイズではない）。
        let win_size = self.window.window().inner_size();
        let size_drift = win_size.width != cell.0 || win_size.height != cell.1;
        if size_drift {
            self.window
                .window()
                .set_inner_size(PhysicalSize::new(cell.0, cell.1));
            self.cell_origin = None;
            self.last_local = None;
        }
        if self.cell_size != cell || self.window.buffer_size() != Some(cell) {
            self.window.resize(cell.0, cell.1)?;
            self.cell_size = cell;
            self.cell_origin = None;
            self.last_local = None;
        }

        // 再センター（未確定 or 余白外れ）。位置は ULW の pptDst として渡す。
        let margin = CELL_MARGIN as i32;
        let origin = match self.cell_origin {
            Some(origin) if !needs_recenter(cell_local(sprite_origin, origin), margin) => origin,
            _ => recentered_origin(sprite_origin, margin),
        };
        self.cell_origin = Some(origin);
        let local = cell_local(sprite_origin, origin);

        // 内容・セル内位置・窓位置のいずれかが変われば描画する。位置（origin）を
        // 含めないと、高速移動で「毎 tick 再センター → local が同じ M」のときに
        // 窓移動の ULW をスキップして sprite が凍り付く（`is_unchanged` の doc 参照）。
        let key = ImageKey {
            image_ref: sprite.image_ref.to_string(),
            flip: sprite.flip,
            width: frame.width,
            height: frame.height,
        };
        if is_unchanged(
            self.last_image.as_ref(),
            self.last_local,
            self.last_origin,
            &key,
            local,
            origin,
            size_drift,
        ) {
            return Ok(());
        }

        // blit は表示に影響しない（表示は ULW でのみ更新）。
        self.window
            .blit(&frame.argb, frame.width, frame.height, local, sprite.flip)?;
        // 位置と内容を原子的に反映する（窓を先に動かすと 1 フレーム位置ズレが出る）。
        self.window.present(origin)?;
        self.last_image = Some(key);
        self.last_local = Some(local);
        self.last_origin = Some(origin);
        Ok(())
    }

    /// 最後に描画した状態をクリアする（Reload 対応。次 draw は全再描画になる）。
    pub fn reset(&mut self) {
        self.last_image = None;
        self.last_local = None;
    }
}
