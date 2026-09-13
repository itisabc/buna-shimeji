//! [`OsSource`](crate::app::environment::OsSource) の実 Win32 実装（タスク #10a）。
//!
//! Java 正本: `.tmp/java-ref/platform/WindowsEnvironment.java` を仕様として逐語移植する:
//!
//! | メソッド | Java 対応行 |
//! |---|---|
//! | `monitors()` | `getWorkAreaRect` L247-256（false 経路）+ `updateScreenRect` L104-150 の Win32 直取得相当 = 既存 [`enumerate_workareas`] 再利用。**EventLoop 構築後（tao PMv2 設定済み）に呼ばれる前提** — 構築前は DPI 仮想化座標（handoff 実測 1536×864 vs 物理 1920×1080）が返る |
//! | `cursor_position()` | `AbstractEnvironment.tick` L177-182 の `MouseInfo.getPointerInfo()` 相当 = `GetCursorPos`。None = 取得失敗 |
//! | `active_window()` | `findActiveWindow` L178-194 逐語: Z 順 `EnumWindows` 走査・[`WindowStatus::Valid`] で採用+列挙停止・[`WindowStatus::Invalid`]（IsZoomed）で列挙中止（None）・[`WindowStatus::Ignored`] / [`WindowStatus::OutOfBounds`] は継続。状態判定は [`window_status`] = `getWindowStatus` L144-176 逐語 |
//! | `windows()` | `restoreWindows` L292-339 用の**選別済み集合**（#9b 契約）: `getWindowStatus` の選別部（visible / 非 cloaked / 非 IsZoomed / isInteractive / 非 IsIconic）を満たす全窓。Z 順・**交差判定なし・列挙中止なし**（OUT_OF_BOUNDS 判定は Environment 側が screen 交差で担う・orch 決定 4） |
//! | `move_window()` | `moveActiveWindow` L274-289 逐語（DPI 補正 + `SetWindowPos(SWP_NOSIZE)`・下記 coder 判断） |
//! | `raise_window()` | `restoreWindows` L327 の `BringWindowToTop` 相当 |
//!
//! orch 決定済みの意図的差異:
//! 1. **whitelist / blacklist は settings.toml `[interactive_windows]` 由来**:
//!    [`Win32OsSource::new`] が受け取った whitelist / blacklist を保持し、
//!    アクティブウィンドウ選別 [`is_interactive_by_title`] へ注入する。資産の
//!    settings.properties に記述が無い場合は既定 = 両方空で、実装体は空スライスを
//!    渡す（= 全窓非 interactive・Java 資産既定と同一挙動）。一般式
//!    [`is_interactive_by_title`] は Java `isInteractive` L87-142 逐語のまま。
//! 2. **interactiveCache / refreshCache 非実装**（L88-91 / L342-346）: 設定は起動時に
//!    確定し tick 中不変のため compute-once でキャッシュ不要
//! 3. **DWMWA_CLOAKED**: Java は Windows8+ チェック有り（L146-147）だが、Rust 版は
//!    [`DwmGetWindowAttribute`] 呼び出し + S_OK かつ flags != 0 のチェックのみで同等
//!    （Win8+ で S_OK が返る環境でのみ判定が効く）
//! 4. **windows() は交差判定をしない**: Environment 側が monitor index 順スロット管理と
//!    OUT_OF_BOUNDS 判定（screen 交差）を担う（#9b 契約）
//!
//! coder 判断（DPI 補正・`getWindowRect` L216-224 / `moveActiveWindow` L279-283）:
//! Java は AWT が system-DPI 論理空間で動くため `Toolkit.getScreenResolution() / 96.0`
//! で論理→物理へ変換してから `SetWindowPos` / `MoveWindow` に渡す。本実装は tao が
//! PMv2 を設定し OS 側 DPI 仮想化が存在しないため、Environment 座標（monitors() /
//! GetWindowRect / GetCursorPos いずれも物理ピクセル）と SetWindowPos 座標は同空間であり、
//! 係数は恒等 1.0。`GetDpiForSystem() / 96.0` を用いると 125% 環境で二重スケール
//! （例: ドラッグ先 800px → 1000px に跳ぶ）となり Java 相当挙動が崩れるため、
//! 変換なしを正とする（#8/#9b の Environment 座標系 = 物理座標契約と整合）。

use std::sync::atomic::{AtomicI64, Ordering};

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Threading::GetCurrentProcessId;
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetCursorPos, GetWindow, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindow, IsWindowVisible, IsZoomed, SetForegroundWindow, SetWindowPos, GWL_EXSTYLE,
    GW_HWNDPREV, HWND_NOTOPMOST, HWND_TOP, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    WS_EX_TOPMOST,
};

use crate::app::environment::OsSource;
use crate::mascot::Rect;
use crate::win::workarea::{enumerate_workareas, Rect as WorkAreaRect};

/// `isInteractive` L87-142 のタイトル判定部分（HWND キャッシュ L88-91 を除く逐語）。
///
/// - L96-100: 空タイトルは最短で false（`isEmpty` は正確な空。trim 後空ではない）
/// - L102-115: blacklist 先行。trim 後空の項目はスキップ（L108）し、
///   **生項目**に対する部分一致で false（L110）
/// - L117-131: whitelist。trim 後空の項目はスキップ（L123）し、生項目の部分一致で true（L126）
/// - L133-141: `whitelistInUse || !blacklistInUse` → 非 interactive、
///   それ以外（blacklist のみ使用中かつ不一致）→ interactive
/// - inUse = 「trim 後空でない項目が 1 つ以上存在」・contains は
///   Java `String.contains` 相当の部分一致・大文字小文字を区別する
pub fn is_interactive_by_title(title: &str, whitelist: &[&str], blacklist: &[&str]) -> bool {
    // L96-100: 空タイトル最短 false
    if title.is_empty() {
        return false;
    }

    // L102-115: blacklist takes precedence over whitelist
    let mut blacklist_in_use = false;
    for entry in blacklist {
        // L108: trim 後空の項目はスキップ（inUse に数えない）
        if entry.trim().is_empty() {
            continue;
        }
        blacklist_in_use = true;
        // L110: contains は生項目に対する部分一致
        if title.contains(entry) {
            return false;
        }
    }

    // L117-131: whitelist
    let mut whitelist_in_use = false;
    for entry in whitelist {
        // L123: trim 後空の項目はスキップ
        if entry.trim().is_empty() {
            continue;
        }
        whitelist_in_use = true;
        // L126: contains は生項目に対する部分一致
        if title.contains(entry) {
            return true;
        }
    }

    // L133-141: whitelistInUse || !blacklistInUse → 非 interactive・else → interactive
    !(whitelist_in_use || !blacklist_in_use)
}

/// `getWindowStatus` の判定結果（`WindowStatus.java` L51-63 相当）。
#[derive(Debug)]
enum WindowStatus {
    /// L166: 有効（prevent others from being valid）。
    Valid,
    /// L159: 最大化 = 無効（prevent others from being valid）。
    Invalid,
    /// L153 / L175: 無視（prevent しない）。
    Ignored,
    /// L169: 有効 interactive 窓だが screen 交差なし（rect 無しも含む）。
    OutOfBounds,
}

/// `getWindowStatus` L144-176 逐語。
///
/// `screen` は `getScreen()` 相当。`None` は [`OsSource::windows`] 用の
/// 交差判定省略モード（選別のみ・orch 決定 4）で、選別通過窓は全て
/// [`WindowStatus::OutOfBounds`] 変体として返る。
unsafe fn window_status(
    hwnd: HWND,
    screen: Option<&Rect>,
    whitelist: &[&str],
    blacklist: &[&str],
) -> WindowStatus {
    // L145: IsWindowVisible
    if !IsWindowVisible(hwnd).as_bool() {
        return WindowStatus::Ignored;
    }

    // L146-155: cloaked（orch 決定 3）
    if is_cloaked(hwnd) {
        return WindowStatus::Ignored;
    }

    // L157-160: 最大化 = INVALID（列挙中止対象）
    if IsZoomed(hwnd).as_bool() {
        return WindowStatus::Invalid;
    }

    // L162: isInteractive(hWnd) && !IsIconic(hWnd)
    if !is_interactive(hwnd, whitelist, blacklist) || IsIconic(hwnd).as_bool() {
        return WindowStatus::Ignored;
    }

    // L164: getWindowRect(hWnd, true)（物理座標・DPI 係数 1.0・モジュール doc）
    let rect = window_rect(hwnd).map(rect_from);

    match (screen, rect) {
        // L165-166: rect 有りかつ screen 交差 → VALID
        (Some(screen), Some(rect)) if intersects(&rect, screen) => WindowStatus::Valid,
        // L167-169: rect 無し / 交差しない / windows() 用（交差判定は Environment 側）
        _ => WindowStatus::OutOfBounds,
    }
}

/// `isInteractive` L87-142（実装体・設定由来の whitelist / blacklist を渡す）。
unsafe fn is_interactive(hwnd: HWND, whitelist: &[&str], blacklist: &[&str]) -> bool {
    // L94: WindowUtils.getWindowTitle(hWnd)
    let title = window_title(hwnd);
    is_interactive_by_title(&title, whitelist, blacklist)
}

/// JNA `WindowUtils.getWindowTitle`（W32WindowUtils）相当
/// （`GetWindowTextLengthW` + `GetWindowTextW` の全長取得）。
unsafe fn window_title(hwnd: HWND) -> String {
    // requiredLength = GetWindowTextLength + 1（null 終端込み）
    let mut buf = vec![0u16; GetWindowTextLengthW(hwnd) as usize + 1];
    // 戻り値 = コピー文字数（null を含まない）
    let copied = GetWindowTextW(hwnd, &mut buf) as usize;
    String::from_utf16_lossy(&buf[..copied.min(buf.len())])
}

/// L146-155 の cloaked 判定（orch 決定 3: S_OK かつ flags != 0 のみ）。
/// `DWMWA_CLOAKED` の値は DWORD（Java は cbattribute=8 の LongByReference だが
/// 値は DWORD なので u32 + 4 バイトで同等）。
unsafe fn is_cloaked(hwnd: HWND) -> bool {
    let mut cloaked: u32 = 0;
    let result = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut u32 as *mut core::ffi::c_void,
        core::mem::size_of::<u32>() as u32,
    );
    result.is_ok() && cloaked != 0
}

/// `getWindowRect(hWnd, dpiAware=true)` L201-226 相当。[`GetWindowRect`] 物理座標。
/// L216-224 の DPI 逆スケールは環境座標 = 物理座標のため恒等（モジュール doc）。
/// 取得失敗（Java は E_HANDLE のみ null・その他は rethrow）は一律 None
/// （列挙直後の有効窓で失敗する経路が実質無く、防御としての差異）。
unsafe fn window_rect(hwnd: HWND) -> Option<RECT> {
    let mut rect = RECT::default();
    GetWindowRect(hwnd, &mut rect).ok()?;
    Some(rect)
}

/// `getScreen()` 相当: 全モニタ矩形の union（`Rectangle.union` 逐語・Java 起点矩形
/// `(0,0,0,0)` 踏襲）。列挙失敗時は空矩形 = 交差しない（VALID 無し =
/// Java の空 screen と同一帰結）。
fn screen_union() -> Rect {
    enumerate_workareas().map_or(
        Rect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        |areas| {
            areas.iter().fold(
                Rect {
                    left: 0,
                    top: 0,
                    right: 0,
                    bottom: 0,
                },
                |acc, wa| Rect {
                    left: acc.left.min(wa.monitor.left),
                    top: acc.top.min(wa.monitor.top),
                    right: acc.right.max(wa.monitor.right),
                    bottom: acc.bottom.max(wa.monitor.bottom),
                },
            )
        },
    )
}

/// 空でない交差（Java `Rectangle.intersects` 相当・environment.rs `rects_intersect`
/// と同一式。退化矩形は実スクリーンでは発生しないため幅/高さの正値ガードは無し）。
fn intersects(a: &Rect, b: &Rect) -> bool {
    a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom
}

/// Win32 [`RECT`] → [`Rect`] 変換。
fn rect_from(rect: RECT) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

/// `workarea::Rect`（`MONITORINFO` 由来）→ [`Rect`] 変換。
fn rect_from_workarea(rect: WorkAreaRect) -> Rect {
    Rect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    }
}

/// id（[`OsSource`] の i64 = hwnd as isize）→ [`HWND`]。
fn hwnd_from_id(id: i64) -> HWND {
    HWND(id as isize as *mut core::ffi::c_void)
}

/// [`HWND`] → id（hwnd as isize）。
fn id_from_hwnd(hwnd: HWND) -> i64 {
    hwnd.0 as isize as i64
}

// ---- #30 item 5: panic 時の WS_EX_TOPMOST 復元（グローバル static の例外）----

/// panic 発生時に TOPMOST を剥がすべき窓 id（0 = なし）。
///
/// グローバル可変状態禁止（AGENTS.md §5-3）の対象はマスコット / アプリの状態であり、
/// プロセス終了時の best-effort 復元専用の小さな static は `mascot/rng.rs` の
/// `SEED_UNIQUIFIER` と同様にマスコット状態へ一切関与しないため例外とする
/// （判断記録: design.md §1.10(z)）。
static PANIC_UNPIN_WINDOW: AtomicI64 = AtomicI64::new(0);

/// panic 時に TOPMOST を剥がすべき窓 id を登録する（`None` = 解除）。
/// 「元から TOPMOST だった窓を対象外にする」判定は呼び出し側
/// （[`crate::app::manager::Manager`]）の責務。
pub fn set_panic_unpin_window(id: Option<i64>) {
    PANIC_UNPIN_WINDOW.store(id.unwrap_or(0), Ordering::Relaxed);
}

/// panic フックから呼ぶ best-effort 復元。保持中の id があれば `HWND_NOTOPMOST` で
/// `SetWindowPos` し、静かに失敗を許容する（戻り値不問・
/// [`Win32OsSource::set_window_topmost`] と同じ呼び出しスタイル）。呼び出し後は 0 に戻す。
pub fn restore_topmost_on_panic() {
    let id = PANIC_UNPIN_WINDOW.swap(0, Ordering::Relaxed);
    if id == 0 {
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd_from_id(id),
            Some(HWND_NOTOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

// ---- #30 item 6: 保持マスコット窓をピン対象窓より前面へ再アサート ----

/// 保持マスコット窓を最前面へ再アサートする（#30 item 6）。
///
/// pin 成立時にピン対象窓 W を `HWND_TOPMOST` にすると、同じ TOPMOST 帯で W が
/// マスコット窓（[`crate::win::window`] が `WS_EX_TOPMOST` で生成）より前面に出る。
/// マスコットは移動（tao の `set_outer_position`）のみで Z オーダーを変えないため、
/// 落下中 / 張り付き中のマスコットが W の裏に隠れる。本関数はピン保持中に毎 tick
/// 呼ばれ、マスコット窓を TOPMOST 帯の最前面へ押し戻す。
///
/// `insertAfter` に W ではなく `HWND_TOP` を渡す理由: マスコットは TOPMOST のため
/// `HWND_TOP` は「TOPMOST 帯の最前面」を意味し W の上に来る。W を指定すると W の
/// **直後**（= W の 1 つ下）になり、マスコットのさらに上に別の TOPMOST 窓があれば
/// 隠れ得るため、明示的に最前面を狙う `HWND_TOP` を採用する。
///
/// 上に窓が無い（既に最前面）なら `SetWindowPos` をスキップし、毎 tick の無駄な
/// Win32 呼び出しを避ける。`GW_HWNDPREV` は「Z 順で 1 つ上の窓」を返し、無ければ
/// `GetWindow` が `Err`（= NULL）になる。`SetWindowPos` の第 2 引数は windows-rs
/// では `Option<HWND>` で、`HWND_TOP` は 0（= NULL）のため `Some(HWND_TOP)` は
/// `None` と等価。`SetWindowPos` の戻り値は不問
/// （[`Win32OsSource::set_window_topmost`] と同じ呼び出しスタイル）。
pub fn ensure_window_above(mascot_hwnd: isize) {
    let mascot = HWND(mascot_hwnd as *mut std::ffi::c_void);
    if mascot.0.is_null() {
        return;
    }
    // GW_HWNDPREV = Z 順で 1 つ上の窓。Err（NULL 含む）なら既に最前面 → skip。
    unsafe {
        if GetWindow(mascot, GW_HWNDPREV).is_err() {
            return;
        }
    }
    unsafe {
        let _ = SetWindowPos(
            mascot,
            Some(HWND_TOP),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
    }
}

/// [`find_active_proc`] の走査コンテキスト（`findActiveWindow` 相当）。
struct ActiveWindowCtx<'a> {
    /// `getScreen()` 相当（L165 の交差判定用）。
    screen: Rect,
    /// 設定由来の whitelist（[`is_interactive_by_title`] へ渡す）。
    whitelist: &'a [&'a str],
    /// 設定由来の blacklist（[`is_interactive_by_title`] へ渡す）。
    blacklist: &'a [&'a str],
    /// 採用した hwnd。None = 窓無し / INVALID で中止。
    handle: Option<HWND>,
}

/// `findActiveWindow` L181-191 の WNDENUMPROC 逐語。
unsafe extern "system" fn find_active_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut ActiveWindowCtx<'_>);
    match window_status(hwnd, Some(&ctx.screen), ctx.whitelist, ctx.blacklist) {
        // L182-185: VALID → 採用して列挙停止
        WindowStatus::Valid => {
            ctx.handle = Some(hwnd);
            BOOL(0)
        }
        // L186: IGNORED / OUT_OF_BOUNDS → 走査継続
        WindowStatus::Ignored | WindowStatus::OutOfBounds => BOOL(1),
        // L187-189: INVALID → 探索中止（handle は None のまま）
        WindowStatus::Invalid => BOOL(0),
    }
}

/// [`collect_proc`] の収集コンテキスト（`restoreWindows` 用選別・交差判定なし）。
struct WindowsCtx<'a> {
    /// 設定由来の whitelist（[`is_interactive_by_title`] へ渡す）。
    whitelist: &'a [&'a str],
    /// 設定由来の blacklist（[`is_interactive_by_title`] へ渡す）。
    blacklist: &'a [&'a str],
    windows: Vec<(i64, RECT)>,
}

/// `restoreWindows` L293-338 用の EnumWindows コールバック。
/// 選別済み集合（`getWindowStatus` の選別部通過 = Java の VALID / OUT_OF_BOUNDS 群）を
/// 全て収集する。交差判定なし・列挙中止なし（#9b 契約・orch 決定 4）。
unsafe extern "system" fn collect_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut WindowsCtx<'_>);
    match window_status(hwnd, None, ctx.whitelist, ctx.blacklist) {
        // 選別通過。screen=None のため常に OutOfBounds 変体（交差判定は Environment 側）。
        // L305-315 相当: 矩形はここで取得（失敗は skip = 列挙継続）
        WindowStatus::Valid | WindowStatus::OutOfBounds => {
            if let Some(rect) = window_rect(hwnd) {
                ctx.windows.push((id_from_hwnd(hwnd), rect));
            }
        }
        WindowStatus::Invalid | WindowStatus::Ignored => {}
    }
    BOOL(1) // 列挙は常に続行
}

/// [`point_window_proc`] の走査コンテキスト（#30: `window_at_point`）。
struct PointWindowCtx {
    /// 判定対象の画面座標（物理座標）。
    point: (i32, i32),
    /// 自プロセス id（自マスコットのレイヤード窓を除外する）。
    pid: u32,
    /// 最初に点を含んだ窓（Z 順で最初 = 最前面）。None = 未発見。
    handle: Option<(i64, RECT)>,
}

/// #30 `window_at_point` の EnumWindows コールバック。
/// 可視・非最小化・自プロセス除外・シェル/デスクトップ窓除外を満たし、rect が点を
/// 含む最初のトップレベル窓を採用して列挙を停止する（Z 順 = 最前面優先）。
unsafe extern "system" fn point_window_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut PointWindowCtx);
    if ctx.handle.is_some() {
        return BOOL(0);
    }

    // 自プロセス（マスコット / トレイ等）を除外
    let mut pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == ctx.pid {
        return BOOL(1);
    }

    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return BOOL(1);
    }

    // タスクバー / デスクトップ / 壁紙（WorkerW）を除外
    if is_shell_window(hwnd) {
        return BOOL(1);
    }

    if let Some(rect) = window_rect(hwnd) {
        let (x, y) = ctx.point;
        // Rect は right/bottom を含まない（半開区間）。
        if rect.left <= x && x < rect.right && rect.top <= y && y < rect.bottom {
            ctx.handle = Some((id_from_hwnd(hwnd), rect));
            return BOOL(0);
        }
    }
    BOOL(1)
}

/// シェル / デスクトップ窓（クラス名完全一致）か。#30 `window_at_point` の除外用。
unsafe fn is_shell_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 256];
    let len = GetClassNameW(hwnd, &mut buf);
    if len <= 0 {
        return false;
    }
    let class = String::from_utf16_lossy(&buf[..len as usize]);
    matches!(class.as_str(), "Shell_TrayWnd" | "Progman" | "WorkerW")
}

/// [`OsSource`] の実 Win32 実装体（単一スレッド前提・Send 不要）。
///
/// whitelist / blacklist は settings.toml の `[interactive_windows]` 由来で、
/// [`Win32OsSource::new`] で注入する。未指定（空）の場合、どのアクティブウィンドウも
/// interactive にならない（Java `isInteractive` の意味論どおり）。
///
/// **`monitors()` / `active_window()` は EventLoop 構築後に呼ばれること**
/// （tao PMv2・モジュール doc 参照）。
#[derive(Debug, Clone, Default)]
pub struct Win32OsSource {
    /// 反応対象タイトルの部分一致リスト（空 = 非使用）。
    whitelist: Vec<String>,
    /// 反応除外タイトルの部分一致リスト（空 = 非使用）。
    blacklist: Vec<String>,
}

impl Win32OsSource {
    /// settings.toml `[interactive_windows]` の whitelist / blacklist を注入して構築する。
    pub fn new(whitelist: Vec<String>, blacklist: Vec<String>) -> Win32OsSource {
        Win32OsSource {
            whitelist,
            blacklist,
        }
    }

    /// `Vec<String>` 設定を [`is_interactive_by_title`] の `&[&str]` 契約へ
    /// 変換する参照スライス対（呼び出し中のみ有効）。
    fn interactive_lists(&self) -> (Vec<&str>, Vec<&str>) {
        (
            self.whitelist.iter().map(String::as_str).collect(),
            self.blacklist.iter().map(String::as_str).collect(),
        )
    }
}

impl OsSource for Win32OsSource {
    /// 既存 [`enumerate_workareas()`] を monitor index 順に
    /// `(モニタ矩形, WorkArea)` へ変換する（`updateScreenRect` L114-140 相当・
    /// doc 差異 1: Java は 5 秒タイマだが tick 毎取得）。
    /// 列挙失敗は warn ログ + 空配列（Java の画面 0 台相当・Environment 側は
    /// union 更新をスキップする）。
    fn monitors(&self) -> Vec<(Rect, Rect)> {
        match enumerate_workareas() {
            Ok(areas) => areas
                .into_iter()
                .map(|wa| (rect_from_workarea(wa.monitor), rect_from_workarea(wa.area)))
                .collect(),
            Err(err) => {
                log::warn!("failed to enumerate monitors: {err}");
                Vec::new()
            }
        }
    }

    /// `MouseInfo.getPointerInfo()` 相当（AbstractEnvironment L177-182）。
    /// `GetCursorPos` 物理座標・None = 取得失敗。
    fn cursor_position(&self) -> Option<(i32, i32)> {
        let mut point = POINT::default();
        unsafe { GetCursorPos(&mut point).ok()? };
        Some((point.x, point.y))
    }

    /// `findActiveWindow` L178-194 逐語（Z 順・VALID 即採用で列挙停止・
    /// INVALID で列挙中止・IGNORED / OUT_OF_BOUNDS は継続）+ Java tick L71 相当の
    /// 後段矩形取得。`EnumWindows` の戻り値は列挙停止（BOOL 0）でも FALSE に
    /// なるため無視（Java 同様）。
    fn active_window(&self) -> Option<(i64, Rect)> {
        let (whitelist, blacklist) = self.interactive_lists();
        let mut ctx = ActiveWindowCtx {
            screen: screen_union(),
            whitelist: &whitelist,
            blacklist: &blacklist,
            handle: None,
        };
        unsafe {
            let _ = EnumWindows(
                Some(find_active_proc),
                LPARAM(&mut ctx as *mut ActiveWindowCtx<'_> as isize),
            );
        }
        // L71: getWindowRect(findActiveWindow(), true)。getWindowStatus L164 でも
        // 取得済みのため、ここで失敗するのは窓が消滅する等のレース時のみ。
        // Java は rect null → setRect(-1,-1,0,0)（id は維持）だが、Rust の契約は
        // (id, rect) 同時なので窓無し（None）として返す（微差・実質到達不能）
        let hwnd = ctx.handle?;
        let rect = unsafe { window_rect(hwnd)? };
        Some((id_from_hwnd(hwnd), rect_from(rect)))
    }

    /// `moveActiveWindow` L274-289 逐語（窓無しは呼ばない契約は Environment 側）。
    fn move_window(&self, id: i64, x: i32, y: i32) {
        // L275: activeWindowHandle null 相当（id 0 = 窓無し）
        if id == 0 {
            return;
        }
        // L279-283: DPI 補正は恒等（coder 判断・モジュール doc）。
        // L285-288: SetWindowPos(SWP_NOSIZE)・戻り値不問（Java 同様）
        unsafe {
            let _ = SetWindowPos(hwnd_from_id(id), None, x, y, 0, 0, SWP_NOSIZE);
        }
    }

    /// `restoreWindows` L293-338 用の選別済み集合（visible / 非 cloaked / 非
    /// IsZoomed / isInteractive / 非 IsIconic）。交差判定なし・列挙中止なし
    /// （#9b 契約・orch 決定 4: OUT_OF_BOUNDS 判定は Environment 側）。
    fn windows(&self) -> Vec<(i64, Rect)> {
        let (whitelist, blacklist) = self.interactive_lists();
        let mut ctx = WindowsCtx {
            whitelist: &whitelist,
            blacklist: &blacklist,
            windows: Vec::new(),
        };
        unsafe {
            let _ = EnumWindows(
                Some(collect_proc),
                LPARAM(&mut ctx as *mut WindowsCtx<'_> as isize),
            );
        }
        ctx.windows
            .into_iter()
            .map(|(id, rect)| (id, rect_from(rect)))
            .collect()
    }

    /// `restoreWindows` L327 の `BringWindowToTop` 相当（#9b 契約）。
    fn raise_window(&self, id: i64) {
        unsafe {
            let _ = BringWindowToTop(hwnd_from_id(id));
        }
    }

    // ---- #30: ドロップした窓の最前面固定（§1.10(z) item 3）----

    /// 点を含む最前面のトップレベル窓 `(id, rect)` を返す（whitelist/blacklist 非適用・R12）。
    /// `WindowFromPoint` は自レイヤード窓を拾う恐れがあるため使わず、`EnumWindows` を
    /// Z 順に走査して可視・非最小化・自プロセス除外・シェル窓除外で選ぶ。
    fn window_at_point(&self, x: i32, y: i32) -> Option<(i64, Rect)> {
        let mut ctx = PointWindowCtx {
            point: (x, y),
            pid: unsafe { GetCurrentProcessId() },
            handle: None,
        };
        unsafe {
            let _ = EnumWindows(
                Some(point_window_proc),
                LPARAM(&mut ctx as *mut PointWindowCtx as isize),
            );
        }
        ctx.handle.map(|(id, rect)| (id, rect_from(rect)))
    }

    /// 窓の現在矩形（whitelist 非適用）。生存（IsWindow）・可視・非最小化・
    /// 非 DWM クロークのときのみ `Some`。pin 窓の毎 tick 追跡・自動解除判定に使う。
    fn window_frame(&self, id: i64) -> Option<Rect> {
        let hwnd = hwnd_from_id(id);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return None;
            }
            if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
                return None;
            }
            if is_cloaked(hwnd) {
                return None;
            }
            window_rect(hwnd).map(rect_from)
        }
    }

    /// `SetWindowPos(HWND_TOPMOST / HWND_NOTOPMOST, SWP_NOMOVE|SWP_NOSIZE|SWP_NOACTIVATE)`。
    /// 成功可否を返す（UIPI 保護窓は false）。
    fn set_window_topmost(&self, id: i64, topmost: bool) -> bool {
        let insert_after = if topmost {
            HWND_TOPMOST
        } else {
            HWND_NOTOPMOST
        };
        unsafe {
            SetWindowPos(
                hwnd_from_id(id),
                Some(insert_after),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
            .is_ok()
        }
    }

    /// `WS_EX_TOPMOST` の有無（pin 前の `was_topmost` 記録用）。
    fn is_window_topmost(&self, id: i64) -> bool {
        unsafe {
            let style = GetWindowLongPtrW(hwnd_from_id(id), GWL_EXSTYLE);
            (style & WS_EX_TOPMOST.0 as isize) != 0
        }
    }

    /// 窓をフォアグラウンド化する（pin 解除後の unpin-activate）。生存する窓のみ
    /// `SetForegroundWindow` を呼び、成否を返す。
    fn activate_window(&self, id: i64) -> bool {
        let hwnd = hwnd_from_id(id);
        unsafe {
            if !IsWindow(Some(hwnd)).as_bool() {
                return false;
            }
            SetForegroundWindow(hwnd).as_bool()
        }
    }
}
