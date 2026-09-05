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
use tao::event_loop::EventLoop;
use tao::platform::windows::{WindowBuilderExtWindows, WindowExtWindows};
use tao::window::{Window, WindowBuilder};
use thiserror::Error;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HANDLE, HWND, POINT,
};
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, BLENDFUNCTION, DIB_RGB_COLORS,
    HBITMAP, HDC, HGDIOBJ,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    GetWindowLongPtrW, SetWindowLongPtrW, UpdateLayeredWindow, GWL_EXSTYLE, ULW_ALPHA,
    WS_EX_LAYERED,
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

/// レイヤード表示用の tao ウィンドウを生成し、`WS_EX_LAYERED` を付与する
/// （[`LayeredWindow`] と softbuffer スパイクモードの共通土台）。
///
/// 設定（design.md §1.2 / Java 版踏襲）:
/// - 枠なし・リサイズ不可・常時最前面・タスクバーに表示しない
/// - `transparent` フラグは使わない（DWM blur-behind はレイヤード描画と不要に干渉する）
/// - フォーカスを取らない（マウス入力は受ける。Java 版の AWT Window 相当）
///
/// サイズは画像に合わせ動的変更できる（`Window::set_inner_size` / [`LayeredWindow::resize`]）。
pub fn build_layered_tao_window<T: 'static>(
    event_loop: &EventLoop<T>,
    width: u32,
    height: u32,
) -> Result<Window, WindowError> {
    let window = WindowBuilder::new()
        .with_decorations(false)
        .with_transparent(false)
        .with_always_on_top(true)
        .with_resizable(false)
        .with_focusable(false)
        .with_skip_taskbar(true)
        .with_visible(true)
        .with_inner_size(PhysicalSize::new(width, height))
        .build(event_loop)
        .map_err(|e| WindowError::CreateFailed(e.to_string()))?;

    let hwnd = hwnd_from_isize(window.hwnd());
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED.0 as isize);
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
    /// 透過ウィンドウを生成する。
    pub fn create<T: 'static>(
        event_loop: &EventLoop<T>,
        width: u32,
        height: u32,
    ) -> Result<Self, WindowError> {
        let window = build_layered_tao_window(event_loop, width, height)?;
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

    /// プレマルチプライド済み 0xAARRGGBB のピクセル列を
    /// `UpdateLayeredWindow(ULW_ALPHA)` でウィンドウに転送する。
    ///
    /// `pixels` の長さはバッファ（resize で設定した width × height）と一致すること。
    /// 位置・サイズはこの関数では変更しない（ULW に pptDst/psize を渡さない）。
    /// 位置変更は tao の `Window::set_outer_position`、サイズ変更は
    /// [`LayeredWindow::resize`] で行う（#5 の描画層が管理）。
    pub fn present(&mut self, pixels: &[u32]) -> Result<(), WindowError> {
        let buffer = self.buffer.as_mut().ok_or(WindowError::NoBuffer)?;
        if pixels.len() != buffer.len() {
            return Err(WindowError::SizeMismatch {
                expected: buffer.len(),
                actual: pixels.len(),
            });
        }
        buffer.pixels_mut().copy_from_slice(pixels);

        // AC_SRC_OVER + AC_SRC_ALPHA + 全体 α 255: DIB の per-pixel α をそのまま使う
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let src_point = POINT { x: 0, y: 0 };
        unsafe {
            UpdateLayeredWindow(
                self.hwnd,
                None, // hdcDst: 位置変更なし
                None, // pptDst: 位置変更なし
                None, // psize: サイズ変更なし（tao 側で管理）
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
}

/// 単一起動を保証するガード（design.md §3-7「多重起動防止」）。
///
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
    /// `name` は `Local\` プレフィックス付きの名前を想定（例: `Local\SimejiSingleInstance`）。
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
