//! モニタ列挙と WorkArea（タスクバー等を除く作業領域）の取得。
//!
//! design.md §1.2 / §2: WorkArea は仮想画面座標。全モニタを列挙し、各モニタに
//! 独立した WorkArea を持つ。所属判定（MonitorFromPoint 相当）は純粋関数に分離し、
//! Win32 API 呼び出しと切り離してテスト可能にする（タスク #2 の指示）。
//! モニタ構成変更は tick 毎の環境再判定で拾う（#8 の Environment の管轄）。

use std::mem;

use thiserror::Error;
use windows::core::BOOL;
use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORENUMPROC, MONITORINFO,
};

/// 仮想画面座標の矩形（左上原点。右辺・下辺は排他的: `right`/`bottom` は含まれない
/// ピクセルの座標。Win32 の `RECT` と同形）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// 幅（ピクセル）。
    pub fn width(&self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    /// 高さ（ピクセル）。
    pub fn height(&self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }

    /// 点が矩形内なら true（`left <= x < right` かつ `top <= y < bottom`）。
    pub fn contains(&self, x: i32, y: i32) -> bool {
        self.left <= x && x < self.right && self.top <= y && y < self.bottom
    }

    /// 点から矩形までの距離の二乗。矩形外の点は最も近い辺/隅までの距離、
    /// 矩形内なら 0。`MonitorFromPoint(MONITOR_DEFAULTTONEAREST)` 相当の
    /// 最近傍判定（[`monitor_for_point`]）で使う。
    fn distance_sq(&self, x: i32, y: i32) -> i64 {
        let dx = if x < self.left {
            (self.left - x) as i64
        } else if x >= self.right {
            (x - self.right + 1) as i64
        } else {
            0
        };
        let dy = if y < self.top {
            (self.top - y) as i64
        } else if y >= self.bottom {
            (y - self.bottom + 1) as i64
        } else {
            0
        };
        dx * dx + dy * dy
    }
}

impl From<RECT> for Rect {
    fn from(r: RECT) -> Self {
        Rect {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
        }
    }
}

/// 1 モニタ分の情報（仮想画面座標）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorkArea {
    /// モニタ全体の矩形（`MONITORINFO.rcMonitor`）。
    pub monitor: Rect,
    /// タスクバー等を除いた作業領域（`MONITORINFO.rcWork`）。
    pub area: Rect,
}

#[derive(Error, Debug)]
pub enum WorkAreaError {
    /// `GetMonitorInfoW` が失敗した（列挙を中断した）。
    #[error("GetMonitorInfoW failed for a monitor")]
    MonitorInfoFailed,
}

/// 全モニタを列挙し、各モニタの矩形と WorkArea を返す。
///
/// `EnumDisplayMonitors` + `GetMonitorInfoW`（rcMonitor / rcWork）を使用。
/// 戻り値の座標は仮想画面座標（プライマリモニタの左上が原点）。
pub fn enumerate_workareas() -> Result<Vec<WorkArea>, WorkAreaError> {
    let mut ctx = EnumCtx {
        areas: Vec::new(),
        failed: false,
    };
    let proc: MONITORENUMPROC = Some(enum_proc);
    // hdc = None（仮想画面全体）、lprcClip = None（クリップなし）。
    let enumerated = unsafe {
        EnumDisplayMonitors(None, None, proc, LPARAM(&mut ctx as *mut EnumCtx as isize)).as_bool()
    };
    // 戻り値 FALSE はコールバック（GetMonitorInfoW）が失敗した場合のみ返る。
    if ctx.failed || !enumerated {
        return Err(WorkAreaError::MonitorInfoFailed);
    }
    Ok(ctx.areas)
}

/// `EnumDisplayMonitors` のコールバックに渡すコンテキスト。
struct EnumCtx {
    areas: Vec<WorkArea>,
    failed: bool,
}

/// `EnumDisplayMonitors` の列挙コールバック。
unsafe extern "system" fn enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    let mut info = MONITORINFO {
        cbSize: mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    if !GetMonitorInfoW(hmonitor, &mut info).as_bool() {
        ctx.failed = true;
        return BOOL(0); // 列挙を中断
    }
    ctx.areas.push(WorkArea {
        monitor: Rect::from(info.rcMonitor),
        area: Rect::from(info.rcWork),
    });
    BOOL(1) // 列挙を続行
}

/// 点が属するモニタのインデックスを返す（純粋関数）。
///
/// `MonitorFromPoint`（`MONITOR_DEFAULTTONEAREST`）相当:
/// - 点を含むモニタがあればそれを返す
/// - なければ最も近いモニタを返す（距離が同点の場合は列挙順で最初）
///
/// 戻り値は `areas` のインデックス。`areas` が空なら `None`。
pub fn monitor_for_point(x: i32, y: i32, areas: &[WorkArea]) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut best_dist = i64::MAX;
    for (i, a) in areas.iter().enumerate() {
        let d = a.monitor.distance_sq(x, y);
        if d < best_dist {
            best = Some(i);
            best_dist = d;
        }
    }
    best
}
