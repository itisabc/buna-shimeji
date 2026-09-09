//! 描画レイヤ。タスク #5: 描画本体（compose + MascotView）。
//!
//! design.md §3-9 の needsRepaint 管理: 画像変化時のみ UpdateLayeredWindow
//! で再描画し、位置不変なら移動のみ（同一内容の移動はピクセル同等）。両者
//! 不変なら何もしない。Java 正本は `MascotImage`（center = アンカー Point）と
//! `WindowsTranslucentWindow`（setImage = フィールド代入・updateImage =
//! validate + repaint 相当）。
//!
//! - [`compose_argb`]: straight RGBA8 をプレマルチプライした 0xAARRGGBB
//!   （行優先・flip は行内水平反転）に変換する純粋関数。
//!   式は [`crate::win::window::premultiply_rgba_to_argb`] をそのまま再利用
//!   （切り捨て `(v * a) / 255`）。flip とプレマルチプライはピクセル独立のため順序不変
//! - [`flipped_offset_x`]: Java ImagePairs.java L85-91 の右画像アンカー相当
//!   `width - dx`（dx は負でもそのまま int 演算）
//! - [`MascotView`]: 透過ウィンドウ 1 枚の再描画・移動の状態機械

pub mod imageset;

use tao::dpi::{PhysicalPosition, PhysicalSize};
// EventLoopWindowTarget で受ける（tao 0.37 の WindowBuilder::build が target を要求。
// イベントハンドラ内での窓生成（#10b-2c の view 補充）に対応。EventLoop は Deref するため
// main 側の &EventLoop 直渡しも引き続き動く）
use tao::event_loop::EventLoopWindowTarget;
use tao::window::Window;

use crate::render::imageset::Frame;
use crate::win::window::{premultiply_rgba_to_argb, LayeredWindow, WindowError};

/// 最後に描画した画像を識別するキー（image_ref + flip + 寸法の同一性のみ。
/// ピクセル内容は比較しない — 同一 image_ref のフレーム内容は実行中に不変）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageKey {
    pub image_ref: String,
    pub flip: bool,
    pub width: u32,
    pub height: u32,
}

/// [`MascotView::draw`] の結果（needsRepaint 最適化の実効区分）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawAction {
    /// 画像・位置とも不変。ULW も位置設定も行わない。
    Skipped,
    /// 同一内容の移動のみ（ULW なし）。
    MovedOnly,
    /// 同一位置での再描画のみ（位置設定なし）。
    Redrawn,
    /// 再描画 + 移動。
    MovedAndRedrawn,
}

/// straight RGBA8 をプレマルチプライした 0xAARRGGBB（行優先）に変換する。
///
/// flip=true は各行内で水平反転する（行単位。バッファ全体反転ではない）。
/// 変換式は [`premultiply_rgba_to_argb`] を行スライスに適用するだけなので
/// 全体一括と同一結果（flip とプレマルチプライはピクセル独立のため順序不変）。
pub fn compose_argb(frame: &Frame, flip: bool) -> Vec<u32> {
    let mut out = Vec::with_capacity((frame.width * frame.height) as usize);
    let row_bytes = frame.width as usize * 4;
    let mut row_rgba: Vec<u8> = Vec::with_capacity(row_bytes);
    for row in frame.rgba.chunks_exact(row_bytes) {
        row_rgba.clear();
        if flip {
            for pixel in row.rchunks_exact(4) {
                row_rgba.extend_from_slice(pixel);
            }
        } else {
            row_rgba.extend_from_slice(row);
        }
        out.extend(premultiply_rgba_to_argb(&row_rgba));
    }
    out
}

/// flip 時の水平描画オフセット（Java ImagePairs.java L85:
/// `rightImage.getWidth() - scaledAnchorX` 相当）。
///
/// dx は負でも `width - dx` のまま int 演算する（u32 減算・飽和はしない）。
pub fn flipped_offset_x(frame_width: u32, pose_dx: i32) -> i32 {
    frame_width as i32 - pose_dx
}

/// 透過ウィンドウ 1 枚と「最後に描画した内容」の状態を保持し、
/// design.md §3-9 の needsRepaint 管理（画像変化時のみ再描画・
/// 位置不変なら移動のみ・両者不変なら何もしない）を行うビュー。
pub struct MascotView {
    window: LayeredWindow,
    last_image: Option<ImageKey>,
    last_pos: Option<(i32, i32)>,
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
            last_image: None,
            last_pos: None,
        })
    }

    /// 内包する tao ウィンドウ（位置変更・イベント照合などに使用）。
    pub fn window(&self) -> &Window {
        self.window.window()
    }

    /// アンカー位置に画像を描画する。
    ///
    /// - `pose_anchor`: ポーズのアンカー dx/dy（画像左上からアンカー点へのオフセット）
    /// - `anchor_pos`: アンカー点のスクリーン座標
    ///
    /// ウィンドウ左上 = (anchor_pos - offset)。flip 時は水平オフセットが
    /// [`flipped_offset_x`] に反転する。画像変化時のみ ULW（再描画）し、
    /// 位置変化時のみ移動する。状態の更新はすべての Win32 操作が成功した後
    /// （`?` でエラー伝播時は前回状態を維持する）。
    /// 処理順は「位置変更→present」（plan.md L93: present() は GetWindowRect の
    /// 現在位置を ULW に渡すため、位置を先に確定させて ULW を 1 呼び出しで確定させる）。
    ///
    /// # 窓寸法 drift の自己修復（タスク #12）
    ///
    /// 描画冒頭で窓物理寸法（tao `inner_size()` = GetClientRect）と frame 寸法を
    /// 比較し、不一致なら窓サイズを frame に復帰して ULW を再送する。
    ///
    /// 因果モデル（ユーザー実機の混在 DPI 環境で実測確定）:
    /// 1. 窓は create 時に主モニタ DPI（例: 125%）コンテキストで論理 102.4
    ///    （=128/1.25）初期化され、窓物理は 128px
    /// 2. モニタ間移動（例: 100% の副モニタへ落下）で WM_DPICHANGED が来ると
    ///    tao 0.37 は通常窓（fullscreen なし && 非最大化 = allow_resize）に対し
    ///    OS suggested_rect を無条件適用する（無効化オプションなし）ため、
    ///    窓物理が 102px に再スケールされる
    /// 3. 落下中は画像固定（shime4 等）のため image_changed=false → ULW 未呼出し
    ///    → 窓 102px が持続し、128px DIB バッファの右/下 26px がクリップされる
    ///    （マスコットの体の角が消える症状）
    /// 4. 窓が縮小しても DIB バッファは 128px 不変のため buffer_size == frame であり、
    ///    既存の「buffer_size 不一致時のみ set_inner_size」経路では窓が戻らない
    ///    （縮むのは窓だけで、バッファは縮んでいない）
    ///
    /// 対処として size_drift を image_changed と同一の present 経路の発火条件に
    /// 加え、窓物理寸法を frame に復帰させる。落下中は anchor 変化で毎 tick
    /// draw が呼ばれる（#11 修正後 needs_repaint が立つ）ため、修復は次 tick
    /// （≤40ms・1 フレーム以内）に完了しクリップは知覚不能。
    /// 単一 DPI 環境では size_drift=false が常に成立するため挙動は不変。
    /// コストは draw あたり inner_size()（GetClientRect）1 回のみ。
    pub fn draw(
        &mut self,
        image_ref: &str,
        frame: &Frame,
        flip: bool,
        pose_anchor: (i32, i32),
        anchor_pos: (i32, i32),
    ) -> Result<DrawAction, WindowError> {
        let offset_x = if flip {
            flipped_offset_x(frame.width, pose_anchor.0)
        } else {
            pose_anchor.0
        };
        let next_pos = (anchor_pos.0 - offset_x, anchor_pos.1 - pose_anchor.1);
        let key = ImageKey {
            image_ref: image_ref.to_string(),
            flip,
            width: frame.width,
            height: frame.height,
        };
        let image_changed = self.last_image.as_ref() != Some(&key);
        let pos_changed = self.last_pos != Some(next_pos);
        // 窓物理寸法の drift 検知（自己修復・doc「窓寸法 drift の自己修復」参照）:
        // WM_DPICHANGED → tao suggested resize で窓だけが frame からずれた状態を
        // 検出する。DIB バッファは縮んでいないため buffer_size では検出できない。
        let win_size = self.window.window().inner_size();
        let size_drift = win_size.width != frame.width || win_size.height != frame.height;

        if pos_changed {
            // 位置変更を先に行う（plan.md L93: window.rs の present() は
            // GetWindowRect の現在位置を ULW に明示渡しするため、位置を先に
            // 確定させれば ULW が新位置で 1 呼び出し確定する）。
            // ULW 呼び出しなし。同一内容の移動はピクセル同等（design.md §3-9）。
            self.window
                .window()
                .set_outer_position(PhysicalPosition::new(next_pos.0, next_pos.1));
        }
        if image_changed || size_drift {
            // 窓物理寸法を frame に同期する。size_drift 時はバッファ寸法が一致して
            // いても（窓だけ縮小している状態）tao 側サイズの復帰が必要なため、
            // buffer_size 不一致チェックと独立条件として set_inner_size する。
            if size_drift || self.window.buffer_size() != Some((frame.width, frame.height)) {
                self.window
                    .window()
                    .set_inner_size(PhysicalSize::new(frame.width, frame.height));
            }
            // バッファ寸法が変わる場合のみ DIB を再確保する（不変条件
            // outer = client = DIB 寸法を維持。resize は DIB 再確保のみ）。
            if self.window.buffer_size() != Some((frame.width, frame.height)) {
                self.window.resize(frame.width, frame.height)?;
            }
            let pixels = compose_argb(frame, flip);
            self.window.present(&pixels)?;
        }

        if image_changed || pos_changed {
            self.last_image = Some(key);
            self.last_pos = Some(next_pos);
        }

        // present を実行した（image_changed または size_drift 修復）かどうかで区分。
        let redrawn = image_changed || size_drift;
        Ok(match (redrawn, pos_changed) {
            (true, true) => DrawAction::MovedAndRedrawn,
            (true, false) => DrawAction::Redrawn,
            (false, true) => DrawAction::MovedOnly,
            (false, false) => DrawAction::Skipped,
        })
    }

    /// 最後の描画状態をクリアする（Reload 対応。次 draw は全再描画になる）。
    pub fn reset(&mut self) {
        self.last_image = None;
        self.last_pos = None;
    }
}
