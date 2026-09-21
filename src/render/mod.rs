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
//!（[`crate::render::imageset::Frame::argb`]）。描画時の flip は DIB への
//! 転送コピー（[`crate::win::window::blit_argb_at`]）に融合する。
//!
//! # 窓 = フレーム寸法（セル方式の撤去・2026-09-20）
//!
//! 窓は sprite のフレームちょうど（DIB も同寸）とし、sprite は常にローカル
//! (0,0) へ描く。sprite の移動は窓位置（[`crate::win::window::MoveBatch`]）だけが運び、
//! **内容は位置に依存しない**。内容の反映（[`LayeredWindow::present`]）は
//! 内容が変わったとき（ポーズ変更・初回・サイズ修復）だけ行う。
//!
//! これは `docs/plans/done/design-draw-batching.md` のセル方式（窓 = 最大フレーム + 2M・
//! sprite をセル内ローカル座標で動かし、余白を割ったら再センター）の撤去。
//! セル方式は再センターのたびに `origin` と `local` が同時に変わるため、
//! 位置と内容が別フレームへ合成されると「旧窓位置 + 新内容」が 1 フレーム露出し、
//! ±M px の揺れ（P0 ジッタ）を起こした。順序付けでは回避できない構造的欠陥
//! （docs/plans/done/report-p0-jitter.md §2・§7）。
//!
//! セル撤去後も「移動を毎 tick ULW の `pptDst` で行う」実装は実機で失敗した:
//! 旧内容が画面に残り、前ポーズの残像として見える（cellcap が 117/181 サンプルを
//! 異常と判定・2026-09-20）。移動は窓 API（[`crate::win::window::MoveBatch`] =
//! `DeferWindowPos` の 1 バッチ）、内容更新は ULW と役割を分けている
//! （移動も窓ごとの `SetWindowPos` だと N に超線形で 100 窓 33 ms/tick かかるが、
//! バッチなら 3 ms/tick）。

pub mod imageset;

use tao::event_loop::EventLoopWindowTarget;
use tao::window::Window;

use crate::render::imageset::Frame;
use crate::win::window::{GlowLayer, LayeredWindow, MoveBatch, WindowError};

/// 最後に描画した画像を識別するキー（image_ref + flip + 寸法 + tint + グローの同一性のみ。
/// ピクセル内容は比較しない — 同一 image_ref のフレーム内容は実行中に不変で、
/// 色づけとグローは描画のたびに掛けて作る）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageKey {
    pub image_ref: String,
    pub flip: bool,
    pub width: u32,
    pub height: u32,
    /// 色づけの乗算係数（`None` = 無変化）。色が変われば内容も変わるので同一性に含める。
    pub tint: Option<[u8; 3]>,
    /// グローの α 倍率（`0` = グローなし）。強度が変われば内容も変わる。
    pub glow: u8,
}

/// [`MascotView::stage`] へ渡す sprite の描画パラメータ（引数過多を避ける束ね）。
#[derive(Debug, Clone, Copy)]
pub struct SpriteDraw<'a> {
    pub image_ref: &'a str,
    pub frame: &'a Frame,
    pub flip: bool,
    /// flip 前のポーズアンカー dx/dy。
    pub pose_anchor: (i32, i32),
    /// アンカー点のスクリーン座標。
    pub anchor_pos: (i32, i32),
    /// 色づけの乗算係数（`None` = 無変化＝元画像のまま）。
    pub tint: Option<[u8; 3]>,
    /// グローの α 倍率（`0` = グローなし）。色は `tint` と同じ色を使う。
    pub glow: u8,
}

/// flip 時の水平描画オフセット（Java ImagePairs.java L85:
/// `rightImage.getWidth() - scaledAnchorX` 相当）。
///
/// dx は負でも `width - dx` のまま i32 演算する（u32 減算・飽和はしない）。
pub fn flipped_offset_x(frame_width: u32, pose_dx: i32) -> i32 {
    frame_width as i32 - pose_dx
}

/// 内容・窓位置が前回と同じで、寸法ドリフトも無ければ描画不要。
///
/// 窓位置（`origin`）を含めるのが必須: 移動では内容（`ImageKey`）が変わらないため、
/// 位置を見ないと「内容が同じ = 不要」と誤判定して窓移動（[`crate::win::window::MoveBatch`]）を
/// スキップし、sprite がその場に凍り付く（実機で確認・2026-09-20）。
pub fn is_unchanged(
    last_image: Option<&ImageKey>,
    last_origin: Option<(i32, i32)>,
    key: &ImageKey,
    origin: (i32, i32),
    size_drift: bool,
) -> bool {
    !size_drift && last_image == Some(key) && last_origin == Some(origin)
}

/// [`MascotView::stage`] が積んだ「この tick で反映する内容」。
#[derive(Debug, Clone)]
struct PendingDraw {
    key: ImageKey,
    /// 内容を ULW で送る位置（= この tick の窓位置）。
    origin: (i32, i32),
    /// DIB を更新済みで ULW 送信が必要か（移動だけの tick は `false`）。
    present: bool,
}

/// 透過ウィンドウ 1 枚と「最後に描画した内容」の状態を保持するビュー。
///
/// 窓は sprite のフレームちょうどで、sprite は常にローカル (0,0) に描く。
/// 1 tick の流れは 2 段階:
///
/// 1. [`MascotView::stage`]: 位置を [`MoveBatch`] に積み、内容が変われば DIB へ blit して
///    反映を予約する（まだ画面には出ない）。
/// 2. 呼び出し側が [`MoveBatch::flush`] した後、[`MascotView::commit`]: 予約した内容を
///    ULW で反映する（窓は既に確定位置にあるため ULW は位置を動かさない）。
///
/// `last_image` / `last_origin` は **画面に反映済みの状態**を表す。更新は
/// [`MascotView::commit`] の成功時のみ行う（失敗時に進めると、次 tick に
/// 「変化なし」と誤判定して再送されず、次のポーズ変化まで画面が凍る）。
///
/// 移動を窓 API に、内容更新を ULW に分けた理由は `LayeredWindow::present` の doc 参照。
pub struct MascotView {
    window: LayeredWindow,
    last_image: Option<ImageKey>,
    /// 最後に反映した窓位置 = sprite 左上のスクリーン座標。
    last_origin: Option<(i32, i32)>,
    /// [`MascotView::commit`] で反映する予約（内容が変わった tick のみ `Some`）。
    pending: Option<PendingDraw>,
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
            last_origin: None,
            pending: None,
        })
    }

    /// 内包する tao ウィンドウ（位置取得・イベント照合などに使用）。
    pub fn window(&self) -> &Window {
        self.window.window()
    }

    /// アンカー位置のフレームを用意する（窓 = フレーム寸法・内容は位置非依存）。
    ///
    /// - `sprite.pose_anchor`: ポーズのアンカー dx/dy（flip 前）。`sprite.anchor_pos` は
    ///   アンカー点のスクリーン座標。sprite 左上 = `anchor_pos - offset`。
    ///
    /// この呼び出しでは画面を変えない: 位置は `moves` に積み、内容が変わったときだけ
    /// DIB へ blit して [`MascotView::commit`] 用に予約する（[`MascotView`] の doc 参照）。
    ///
    /// 窓物理寸法がフレームとずれたら（初回・フレーム変更・WM_DPICHANGED の自己修復）
    /// 窓と DIB をフレーム寸法へ戻す。内容・位置とも不変なら何もしない。
    pub fn stage(
        &mut self,
        sprite: SpriteDraw<'_>,
        moves: &mut MoveBatch,
    ) -> Result<(), WindowError> {
        let frame = sprite.frame;
        let size = (frame.width, frame.height);

        let offset_x = if sprite.flip {
            flipped_offset_x(frame.width, sprite.pose_anchor.0)
        } else {
            sprite.pose_anchor.0
        };
        // sprite 左上のスクリーン座標 = 窓位置。内容は常に local (0,0)。
        let origin = (
            sprite.anchor_pos.0 - offset_x,
            sprite.anchor_pos.1 - sprite.pose_anchor.1,
        );

        // 窓サイズのドリフト検知（初回・フレーム変更・DPI 遷移の自己修復）。
        // tao の inner_size は stale な scale を経由するため Win32 の生値で見て、
        // 復帰も SetWindowPos 直呼びで行う（set_inner_size だと 128px に縮む・
        // `LayeredWindow::client_size` の doc 参照）。
        let size_drift = self.window.client_size() != size;
        if size_drift {
            self.window.set_size(size.0, size.1);
        }
        if self.window.buffer_size() != Some(size) {
            self.window.resize(size.0, size.1)?;
        }

        let key = ImageKey {
            image_ref: sprite.image_ref.to_string(),
            flip: sprite.flip,
            width: frame.width,
            height: frame.height,
            tint: sprite.tint,
            glow: sprite.glow,
        };
        // 内容キーの比較（位置は見ない）。移動だけの tick で ULW を省く判定に使う。
        let content_changed = self.last_image.as_ref() != Some(&key);
        let unchanged = is_unchanged(
            self.last_image.as_ref(),
            self.last_origin,
            &key,
            origin,
            size_drift,
        );
        if unchanged {
            return Ok(());
        }

        moves.add(self.window.hwnd(), origin.0, origin.1);
        if content_changed || size_drift {
            // 内容を DIB に用意し、ULW は flush 後にまとめて送る（位置は窓 API が担う）。
            self.window.blit(
                &frame.argb,
                frame.width,
                frame.height,
                (0, 0),
                sprite.flip,
                sprite.tint,
            )?;
            // グローは α ブラー層を blit の後に加算合成する（色と強度はここで掛ける）。
            // 色づけなしの個体は色を持たないため光らせない。
            if sprite.glow > 0 {
                if let Some(color) = sprite.tint {
                    self.window.blit_add(
                        GlowLayer {
                            blur: &frame.glow,
                            size: (frame.width, frame.height),
                            color,
                            strength: sprite.glow,
                        },
                        (0, 0),
                        sprite.flip,
                    )?;
                }
            }
        }
        // 予約と状態更新（`last_*`）は [`MascotView::commit`] の成功時に確定する。
        self.pending = Some(PendingDraw {
            key,
            origin,
            present: content_changed || size_drift,
        });
        Ok(())
    }

    /// [`MascotView::stage`] が予約した内容反映（ULW）を送り、反映済み状態を更新する。
    ///
    /// - **呼び出し側が [`MoveBatch::flush`] した後に呼ぶこと**（窓が確定位置に無い状態で
    ///   ULW に位置を渡すと、ULW が窓を動かして残像の原因になる）。
    /// - `moves_applied` は `flush` の結果。移動だけの tick では、これが `true` のときだけ
    ///   窓位置を反映済みとして記録する（`false` なら次 tick にやり直す）。
    /// - 戻り値 `Ok(true)` = この tick の予約がすべて反映された（呼び出し側は `needs_repaint`
    ///   を落としてよい）。`Ok(false)` = 移動が未反映のまま残った（再試行が必要）。
    /// - `present` 失敗時は `Err`（`last_*` を進めない = 次 tick に同じ内容を再送する）。
    pub fn commit(&mut self, moves_applied: bool) -> Result<bool, WindowError> {
        let Some(draw) = self.pending.take() else {
            return Ok(true);
        };
        if draw.present {
            // ULW は位置も同時に渡すため、失敗しなければ移動も反映済みになる。
            self.window.present(draw.origin)?;
            self.last_image = Some(draw.key);
            self.last_origin = Some(draw.origin);
            return Ok(true);
        }
        if moves_applied {
            self.last_origin = Some(draw.origin);
        }
        Ok(moves_applied)
    }

    /// 最後に描画した状態をクリアする（Reload 対応。次 draw は全再描画になる）。
    pub fn reset(&mut self) {
        self.last_image = None;
        self.last_origin = None;
        self.pending = None;
    }
}
