//! 透過ウィンドウ（WS_EX_LAYERED + UpdateLayeredWindow）と単一起動 mutex。
//!
//! design.md §1.3: Java 版の `NativeFactory` / `TranslucentWindow` に相当する
//! `win::window::LayeredWindow`。描画は windows-rs 直描き経路
//! （CreateDIBSection + UpdateLayeredWindow）で行う。
//!
//! スパイク検証（design.md §6、タスク #2）により softbuffer 経路は不成立と確定した:
//! - softbuffer 0.4.8 の Windows バックエンド（`src/backends/win32.rs`）は
//!   `CreateDIBSection` + `BitBlt(SRCCOPY)` のみで、`UpdateLayeredWindow` を一切呼ばない
//! - GDI の BitBlt は α チャネルを転送しないため、WS_EX_LAYERED ウィンドウには
//!   ピクセル単位透過で描画できない
//! - tao 0.37 の `with_transparent` は DWM blur-behind 方式で、これもピクセル単位 α 不可
//!
//! 詳細はタスク #2 の報告に記録（`examples/transparency_spike.rs` で両経路を実証）。
//!
//! ヒットテスト: レイヤードウィンドウ（per-pixel α / ULW_ALPHA）は α=0 のピクセル領域を
//! OS が自動的にマウス透過する（Win32 の標準動作）。そのため design §2 の
//! 「WM_NCHITTEST → HTTRANSPARENT 相当」の要件はこの方式で OS レベルに達成され、
//! wndproc フック（tao 0.37 は提供しない）を追加しない。

use std::io;
use std::slice;

use tao::dpi::PhysicalSize;
// EventLoopWindowTarget で受ける（tao 0.37 の WindowBuilder::build が target を要求する。
// EventLoop 自体は EventLoopWindowTarget へ Deref するため main 側は &EventLoop を渡せる・#10b-2c）
use tao::event_loop::EventLoopWindowTarget;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HANDLE, HWND, POINT, SIZE,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, DeferWindowPos, EndDeferWindowPos, GetClientRect, GetWindowLongPtrW,
    SetWindowLongPtrW, SetWindowPos, UpdateLayeredWindow, GWL_EXSTYLE, GWL_STYLE, HDWP,
    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, ULW_ALPHA, WS_CAPTION,
    WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_GROUP, WS_MAXIMIZEBOX,
    WS_MINIMIZEBOX, WS_POPUP, WS_SYSMENU, WS_THICKFRAME, WS_VISIBLE,
};

#[derive(Error, Debug)]
pub enum WindowError {
    #[error("tao window creation failed: {0}")]
    CreateFailed(String),
    #[error("GetDC failed")]
    GetDcFailed,
    #[error("CreateCompatibleDC failed")]
    CreateCompatibleDcFailed(#[source] io::Error),
    #[error("CreateDIBSection failed")]
    CreateDibSectionFailed(#[source] io::Error),
    #[error("no buffer; call resize first")]
    NoBuffer,
    #[error("pixel buffer size mismatch: expected {expected}, got {actual}")]
    SizeMismatch { expected: usize, actual: usize },
    #[error("UpdateLayeredWindow failed")]
    UpdateLayeredWindowFailed(#[source] io::Error),
}

/// Win32 `HANDLE` の生の値（tao の `hwnd()` は `isize` を返す）。
fn hwnd_from_isize(hwnd: isize) -> HWND {
    HWND(hwnd as *mut core::ffi::c_void)
}

/// RGBA8（1 ピクセル 4 バイト）を、レイヤードウィンドウ用の
/// プレマルチプライド済み 0xAARRGGBB（u32、リトルエンディアンでメモリ上は B,G,R,A）に変換する。
///
/// # 返り値の形式
/// 上位 8 ビットが α、下位から R, G, B。α=0 のピクセルは完全透過（値は 0）。
///
/// Java 版（`IntegerRaster` / `BufferedImage TYPE_INT_ARGB_PRE`）と同じく
/// 切り捨て `(v * a) / 255` でプレマルチプライする（#4/#5 で Java 一致を再検証）。
pub fn premultiply_rgba_to_argb(rgba: &[u8]) -> Vec<u32> {
    rgba.chunks_exact(4)
        .map(|c| {
            let r = c[0] as u32;
            let g = c[1] as u32;
            let b = c[2] as u32;
            let a = c[3] as u32;
            (a << 24) | ((r * a / 255) << 16) | ((g * a / 255) << 8) | (b * a / 255)
        })
        .collect()
}

/// プレマルチプライド ARGB の sprite を `dst` の `at` に flip 付きで配置する。
///
/// `dst` 全体を透明(0)でクリアしてから書く（セル内のゴースト防止）。`at` は負や
/// はみ出しを許容し、`dst` の外へ出た画素はクリップする。`src` が空なら no-op。
///
/// `flip == true` は sprite を水平反転して置く（`dst` 上のピクセル位置は変えず、
/// 対応する `src` の列を反転する）。転送コピーに融合するため、描画ごとのフレーム
/// 全体コピーを避けられる。
///
/// 契約: `dst.len() == dst_w * dst_h`・`src.len() == src_w * src_h`（違反は assert）。
pub fn blit_argb_at(
    dst: &mut [u32],
    dst_size: (u32, u32),
    src: &[u32],
    src_size: (u32, u32),
    at: (i32, i32),
    flip: bool,
) {
    let (dst_w, dst_h) = dst_size;
    let (src_w, src_h) = src_size;
    assert_eq!(
        dst.len(),
        dst_w as usize * dst_h as usize,
        "blit_argb_at: dst size mismatch"
    );
    if src.is_empty() || dst_w == 0 || dst_h == 0 {
        return;
    }
    assert_eq!(
        src.len(),
        src_w as usize * src_h as usize,
        "blit_argb_at: src size mismatch"
    );
    assert!(
        src_w > 0 && src_h > 0,
        "blit_argb_at: src dims must be non-zero"
    );

    dst.fill(0);

    let dst_w = dst_w as i64;
    let dst_h = dst_h as i64;
    let src_w = src_w as i64;
    let src_h = src_h as i64;
    for sy in 0..src_h {
        let dy = at.1 as i64 + sy;
        if dy < 0 || dy >= dst_h {
            continue;
        }
        let dst_row = dy as usize * dst_w as usize;
        let src_row = sy as usize * src_w as usize;
        for sx in 0..src_w {
            let dx = at.0 as i64 + sx;
            if dx < 0 || dx >= dst_w {
                continue;
            }
            let s = if flip {
                src[src_row + (src_w - 1 - sx) as usize]
            } else {
                src[src_row + sx as usize]
            };
            dst[dst_row + dx as usize] = s;
        }
    }
}

/// style を真の枠なし窓（WS_POPUP）に矯正する純関数。
///
/// 装飾系 6 ビット（WS_CAPTION / WS_SYSMENU / WS_MAXIMIZEBOX / WS_MINIMIZEBOX /
/// WS_THICKFRAME / WS_GROUP。MINIMIZEBOX と GROUP は同値）を除去し、
/// WS_POPUP | WS_VISIBLE | WS_CLIPSIBLINGS を付与する。それ以外のビットは保持する。
///
/// tao 0.37 は `with_decorations(false)` でも装飾系スタイルを無条件に残す
/// （`to_window_styles`）ための対処。UpdateLayeredWindow の per-pixel α 合成は
/// 真の枠なし窓を前提とする（スパイク検証 2026-09-05）。
pub fn correct_style(style: isize) -> isize {
    (style
        & !(WS_CAPTION.0
            | WS_SYSMENU.0
            | WS_MAXIMIZEBOX.0
            | WS_MINIMIZEBOX.0
            | WS_THICKFRAME.0
            | WS_GROUP.0) as isize)
        | (WS_POPUP.0 | WS_VISIBLE.0 | WS_CLIPSIBLINGS.0) as isize
}

/// exstyle をタスクバー/Alt-Tab に出ない形に矯正する純関数。
///
/// WS_EX_APPWINDOW を除去し WS_EX_TOOLWINDOW を付与する。他のビットは保持し、
/// WS_EX_LAYERED の付与は含めない（呼び出し側で先に OR 済みの値を渡す）。
///
/// tao 0.37 の `with_skip_taskbar(true)` は APPWINDOW を除去しないため、
/// レイヤード描画用の exstyle 設定後に追加適用する（タスク #9e・実測対処）。
pub fn correct_exstyle(exstyle: isize) -> isize {
    (exstyle & !(WS_EX_APPWINDOW.0 as isize)) | WS_EX_TOOLWINDOW.0 as isize
}

/// レイヤード表示用の tao ウィンドウを生成し、`WS_EX_LAYERED` を付与する
/// （[`LayeredWindow`] と softbuffer スパイクモードの共通土台）。
///
/// 設定（design.md §1.2 / Java 版踏襲）:
/// - 枠なし・リサイズ不可・常時最前面・タスクバーに表示しない
/// - `transparent` フラグは使わない（DWM blur-behind はレイヤード描画と不要に干渉する）
/// - フォーカスを取らない（マウス入力は受ける。Java 版の AWT Window 相当）
///
/// サイズは画像に合わせ動的変更できる（`Window::set_inner_size` / [`LayeredWindow::resize`]）。
///
/// `window_target` は [`tao::event_loop::EventLoopWindowTarget`]。tao 0.37 では
/// イベントハンドラ内（`run` クロージャの第 2 引数）でも新規窓を生成する必要がある
/// （#10b-2c の view 補充経路）ため、`EventLoop` ではなく target で受ける
/// （`EventLoop` は Deref するため main 側の `&event_loop` 直渡しも引き続き動く）。
pub fn build_layered_tao_window<T: 'static>(
    window_target: &EventLoopWindowTarget<T>,
    width: u32,
    height: u32,
) -> Result<Window, WindowError> {
    let window = WindowBuilder::new()
        .with_decorations(false)
        // tao 0.37 は既定で非装飾窓にも DWM 影を残す(decoration_shadow=true):
        // WS_CAPTION がスタイルに残り WM_NCCALCSIZE が client を枠分だけ縮めるため、
        // 窓矩形 ≠ client 矩形となり UpdateLayeredWindow の per-pixel α が
        // 窓全体に適用されない(スパイク検証で白い client として表示される不具合)。
        // 影を無効化し client = 窓全体 = 要求サイズにする。
        .with_undecorated_shadow(false)
        .with_transparent(false)
        .with_always_on_top(true)
        .with_resizable(false)
        .with_focusable(false)
        .with_skip_taskbar(true)
        .with_visible(true)
        .with_inner_size(PhysicalSize::new(width, height))
        .build(window_target)
        .map_err(|e| WindowError::CreateFailed(e.to_string()))?;

    let hwnd = hwnd_from_isize(window.hwnd());
    unsafe {
        // 既存の exstyle に WS_EX_LAYERED を追加し、タスクバーに出ないよう
        // APPWINDOW を除去して TOOLWINDOW に矯正する（skip_taskbar(true) は
        // APPWINDOW を除去しない実測への対処・タスク #9e）。
        let exstyle = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            correct_exstyle(exstyle | WS_EX_LAYERED.0 as isize),
        );

        // tao 0.37 は with_decorations(false) でも WS_CAPTION|WS_SYSMENU|WS_MAX(MIN)IMIZEBOX
        // を無条件に残す(to_window_styles)。UpdateLayeredWindow の per-pixel α 合成は
        // 真の枠なし窓(WS_POPUP)を前提とするため、装飾系スタイルを剥がして
        // WS_POPUP に矯正する。これがないと ULW が TRUE を返しても内容が
        // スクリーンに合成されない(スパイク検証 2026-09-05)。
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(hwnd, GWL_STYLE, correct_style(style));
        // スタイル変更を非クライアント領域に反映し、outer = client = 要求サイズに矯正する
        // (CreateWindowEx 時に AdjustWindowRect 相当で幅が膨張するための是正)。
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            width as i32,
            height as i32,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    Ok(window)
}

/// レイヤードウィンドウの DIB バッファ（サイズ変更時のみ再確保する。design.md §2）。
struct LayeredBuffer {
    dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    pixels: *mut u32,
    width: u32,
    height: u32,
}

impl LayeredBuffer {
    /// 32bpp top-down DIB を作成し、メモリ DC に選択する。
    unsafe fn new(width: u32, height: u32) -> Result<Self, WindowError> {
        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err(WindowError::GetDcFailed);
        }
        let dc = CreateCompatibleDC(Some(screen_dc));
        if dc.is_invalid() {
            let err = io::Error::last_os_error();
            ReleaseDC(None, screen_dc);
            return Err(WindowError::CreateCompatibleDcFailed(err));
        }
        ReleaseDC(None, screen_dc);

        let bmi = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: core::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                // 負の高さ = top-down（PNG の走査順と一致。pixels[y * width + x]）
                biHeight: -(height as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let bitmap =
            CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0).map_err(|e| {
                let _ = DeleteDC(dc);
                WindowError::CreateDibSectionFailed(io::Error::from(e))
            })?;
        let old_bitmap = SelectObject(dc, bitmap.into());

        Ok(LayeredBuffer {
            dc,
            bitmap,
            old_bitmap,
            pixels: bits as *mut u32,
            width,
            height,
        })
    }

    fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    fn pixels(&self) -> &[u32] {
        unsafe { slice::from_raw_parts(self.pixels, self.len()) }
    }

    fn pixels_mut(&mut self) -> &mut [u32] {
        unsafe { slice::from_raw_parts_mut(self.pixels, self.len()) }
    }
}

impl Drop for LayeredBuffer {
    fn drop(&mut self) {
        unsafe {
            // 選択解除 → ビットマップ削除 → DC 削除の順（GDI の規範的な解放順）
            let _ = SelectObject(self.dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

/// 透過ウィンドウ本体（Java 版 `TranslucentWindow` 相当）。
///
/// tao ウィンドウ（[`build_layered_tao_window`] で WS_EX_LAYERED 付き）+ DIB バッファ +
/// `UpdateLayeredWindow` による per-pixel α 描画。
pub struct LayeredWindow {
    window: Window,
    hwnd: HWND,
    buffer: Option<LayeredBuffer>,
}

impl LayeredWindow {
    /// 透過ウィンドウを生成する。`window_target` は
    /// [`tao::event_loop::EventLoopWindowTarget`]（イベントハンドラ内での生成に対応・
    /// [`build_layered_tao_window`] doc 参照）。
    pub fn create<T: 'static>(
        window_target: &EventLoopWindowTarget<T>,
        width: u32,
        height: u32,
    ) -> Result<Self, WindowError> {
        let window = build_layered_tao_window(window_target, width, height)?;
        let hwnd = hwnd_from_isize(window.hwnd());
        Ok(LayeredWindow {
            window,
            hwnd,
            buffer: None,
        })
    }

    /// 内包する tao ウィンドウ（位置変更・イベント照合などに使用）。
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// 内包する tao ウィンドウ（softbuffer スパイクモードが move するための所有権移転）。
    pub fn into_window(self) -> Window {
        self.window
    }

    /// 生ウィンドウハンドル。
    pub fn hwnd(&self) -> HWND {
        self.hwnd
    }

    /// 描画バッファを (width, height) に再確保する。
    /// バッファはサイズ変更時のみ再確保する（design.md §2「描画バッファ再確保」）。
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), WindowError> {
        if let Some(b) = &self.buffer {
            if b.width == width && b.height == height {
                return Ok(());
            }
        }
        if width == 0 || height == 0 {
            self.buffer = None;
            return Ok(());
        }
        self.buffer = Some(unsafe { LayeredBuffer::new(width, height)? });
        Ok(())
    }

    /// sprite を DIB のローカル座標 `at` に描く（`flip` 付き）。
    ///
    /// `pixels` は `width * height` 長のプレマルチプライド 0xAARRGGBB。
    /// DIB 全体はクリアされる（[`blit_argb_at`]）。窓や画面へはまだ反映しない
    /// （反映は [`LayeredWindow::present`]）。
    pub fn blit(
        &mut self,
        pixels: &[u32],
        width: u32,
        height: u32,
        at: (i32, i32),
        flip: bool,
    ) -> Result<(), WindowError> {
        let buffer = self.buffer.as_mut().ok_or(WindowError::NoBuffer)?;
        let (buf_w, buf_h) = (buffer.width, buffer.height);
        if pixels.len() != width as usize * height as usize {
            return Err(WindowError::SizeMismatch {
                expected: width as usize * height as usize,
                actual: pixels.len(),
            });
        }
        blit_argb_at(
            buffer.pixels_mut(),
            (buf_w, buf_h),
            pixels,
            (width, height),
            at,
            flip,
        );
        Ok(())
    }

    /// バッファ内容を `UpdateLayeredWindow(ULW_ALPHA)` でウィンドウへ転送し、
    /// あわせて窓位置を `dst` にする。
    ///
    /// **目標位置 `dst` とバッファサイズを毎回明示的に渡す**。pptDst / psize を
    /// 省略する「内容のみ更新」形式は、この検証環境（Windows 11 / スパイク検証
    /// 2026-09-05）では TRUE を返しながら画面に一切合成されないため、明示渡しが必須。
    ///
    /// **位置は呼び出し前に [`MoveBatch`]（`DeferWindowPos`）で確定させること**
    /// （`dst` はその確定位置と一致させる）。毎 tick 動く `pptDst` を ULW で与えると
    /// 旧内容が画面に残り、前ポーズの残像として見える（2026-09-20 実測・cellcap 117/181 異常）。
    /// 内容が位置非依存なので、移動は窓 API 側で行い、ULW は内容更新専用にする。
    pub fn present(&mut self, dst: (i32, i32)) -> Result<(), WindowError> {
        let buffer = self.buffer.as_ref().ok_or(WindowError::NoBuffer)?;

        // AC_SRC_OVER + AC_SRC_ALPHA + 全体 α 255: DIB の per-pixel α をそのまま使う
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let src_point = POINT { x: 0, y: 0 };
        let dst_point = POINT { x: dst.0, y: dst.1 };
        let size = SIZE {
            cx: buffer.width as i32,
            cy: buffer.height as i32,
        };
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None, // hdcDst: 既定のスクリーン DC を使用
                Some(&dst_point),
                Some(&size),
                Some(buffer.dc),
                Some(&src_point),
                COLORREF(0), // ULW_ALPHA では不使用
                Some(&blend),
                ULW_ALPHA,
            )
            .map_err(|e| WindowError::UpdateLayeredWindowFailed(io::Error::from(e)))?;
        }
        Ok(())
    }

    /// DIB に書き込まれたピクセルを読み戻す（スパイク検証・テスト用）。
    pub fn read_pixels(&self) -> Result<Vec<u32>, WindowError> {
        let buffer = self.buffer.as_ref().ok_or(WindowError::NoBuffer)?;
        Ok(buffer.pixels().to_vec())
    }

    /// バッファの (width, height)。
    pub fn buffer_size(&self) -> Option<(u32, u32)> {
        self.buffer.as_ref().map(|b| (b.width, b.height))
    }

    /// クライアント領域の物理寸法（live の `GetClientRect`）。
    ///
    /// tao の `inner_size()` は DPI 遷移後に stale な scale を経由するため物理値を
    /// ずらすことがある（遷移直後の 128px クリップ・2026-09-20 実測）。窓サイズの
    /// 判定は Win32 の生値で行う。
    pub fn client_size(&self) -> (u32, u32) {
        let mut rect = windows::Win32::Foundation::RECT::default();
        if unsafe { GetClientRect(self.hwnd, &mut rect) }.is_err() {
            return (0, 0);
        }
        (
            (rect.right - rect.left).max(0) as u32,
            (rect.bottom - rect.top).max(0) as u32,
        )
    }

    /// 窓の物理寸法を `(width, height)` に変更する（tao を経由しない Win32 直呼び）。
    ///
    /// tao の `set_inner_size` は stale な scale で論理→物理変換するため、DPI 遷移直後に
    /// 128×128 へ縮む（[`LayeredWindow::client_size`] の doc 参照）。位置は動かさない。
    pub fn set_size(&self, width: u32, height: u32) {
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                width as i32,
                height as i32,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// 1 tick 分の窓移動を `DeferWindowPos` に積んで一括適用する。
///
/// 窓ごとに `SetWindowPos` を呼ぶと N に超線形に伸びる（実測 100 窓で 33.2 ms/tick =
/// 332 µs/窓）が、`BeginDeferWindowPos` / `DeferWindowPos` / `EndDeferWindowPos` の
/// 1 バッチなら 100 窓で 3.0 ms/tick（30 µs/窓）に収まる（2026-09-20 movespike 実測）。
///
/// 使い方: 各 view が [`MoveBatch::add`] で位置を積み、tick の描画ループ末尾で
/// [`MoveBatch::flush`] する。**内容の ULW（[`LayeredWindow::present`]）は flush 後に
/// 呼ぶこと**（ULW に位置を動かさせない = 残像の原因を避ける）。
///
/// 失敗時は失われた移動を作らないことを優先する:
/// - `BeginDeferWindowPos` 失敗 → 以降の `add` は即時 `SetWindowPos`
/// - `DeferWindowPos` 失敗（この時点で HDWP は使用不可になる）→ 積み済みを即時
///   `SetWindowPos` で適用してから、以降は即時 `SetWindowPos` に切り替える
/// - `EndDeferWindowPos` 失敗 → [`MoveBatch::flush`] が `false` を返す（呼び出し側は
///   「窓位置が未反映」として状態更新を見送り、次 tick にやり直す）
pub struct MoveBatch {
    hdwp: Option<HDWP>,
    /// 積んだ移動（HDWP が途中で使えなくなったときの再適用用）。
    pending: Vec<(HWND, i32, i32)>,
}

/// 窓を 1 枚動かす（サイズ・Z 順・アクティブ化は不変）。
fn move_window(hwnd: HWND, x: i32, y: i32) {
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

impl MoveBatch {
    /// `capacity` は 1 tick に動かす見込みの窓数（`DeferWindowPos` の初期容量ヒント）。
    pub fn new(capacity: usize) -> Self {
        let hdwp = unsafe { BeginDeferWindowPos(capacity.max(1) as i32) }.ok();
        let pending = if hdwp.is_some() {
            Vec::with_capacity(capacity)
        } else {
            Vec::new()
        };
        MoveBatch { hdwp, pending }
    }

    /// 窓 `hwnd` の位置を `(x, y)` にする予約を積む。
    pub fn add(&mut self, hwnd: HWND, x: i32, y: i32) {
        if let Some(hdwp) = self.hdwp {
            match unsafe {
                DeferWindowPos(
                    hdwp,
                    hwnd,
                    None,
                    x,
                    y,
                    0,
                    0,
                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                )
            } {
                Ok(next) => {
                    self.hdwp = Some(next);
                    self.pending.push((hwnd, x, y));
                    return;
                }
                Err(_) => {
                    // MSDN: 失敗した HDWP は使用不可。積み済みを即時適用してから切替える。
                    self.hdwp = None;
                    self.apply_pending();
                }
            }
        }
        move_window(hwnd, x, y);
    }

    /// 積んだ移動をまとめて適用する。`EndDeferWindowPos` が失敗したときだけ `false`
    /// （その場合も積み済みの移動を `SetWindowPos` で適用し、呼び出し側には
    /// 「窓位置が確定していない」ことを伝える）。何も積んでいなければ `true`。
    pub fn flush(&mut self) -> bool {
        let Some(hdwp) = self.hdwp.take() else {
            self.apply_pending();
            return true;
        };
        match unsafe { EndDeferWindowPos(hdwp) } {
            Ok(()) => {
                self.pending.clear();
                true
            }
            Err(_) => {
                self.apply_pending();
                false
            }
        }
    }

    /// 積んだ移動を 1 枚ずつ `SetWindowPos` で適用する（HDWP を使えない・失敗したとき）。
    fn apply_pending(&mut self) {
        for (hwnd, x, y) in self.pending.drain(..) {
            move_window(hwnd, x, y);
        }
    }
}

impl Drop for MoveBatch {
    fn drop(&mut self) {
        // flush 忘れで移動が消えないように（EndDeferWindowPos は失敗しても無害）。
        self.flush();
    }
}

/// 単一起動を保証するガード（design.md §3-7「多重起動防止」）。
/// 名前付き mutex（`Local\` 名前空間 = ユーザーセッション内で単一）を取得し、
/// 既に取得済みなら [`SingleInstanceError::AlreadyRunning`] を返す。
/// Java 版には無い改善。ドロップ時に mutex を解放する。
pub struct SingleInstance {
    _handle: HANDLE,
}

#[derive(Error, Debug)]
pub enum SingleInstanceError {
    /// 既に同一インスタンスが起動している（第 2 起動を検出）。
    #[error("another instance is already running")]
    AlreadyRunning,
    #[error("CreateMutexW failed")]
    CreateFailed(#[source] io::Error),
}

impl SingleInstance {
    /// 名前付き mutex で単一起動を保証する。
    ///
    /// `name` は `Local\` プレフィックス付きの名前を想定（例: `Local\ShimejiSingleInstance`）。
    /// `CreateMutexW` は既存 mutex があっても有効なハンドルを返し（成功扱い）、
    /// その直後の `GetLastError() == ERROR_ALREADY_EXISTS` で既起動を判別する。
    pub fn acquire(name: &str) -> Result<Self, SingleInstanceError> {
        let name_wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe { CreateMutexW(None, false, PCWSTR::from_raw(name_wide.as_ptr())) }
            .map_err(|e| SingleInstanceError::CreateFailed(io::Error::from(e)))?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            let _ = unsafe { CloseHandle(handle) };
            return Err(SingleInstanceError::AlreadyRunning);
        }
        Ok(SingleInstance { _handle: handle })
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self._handle);
        }
    }
}
