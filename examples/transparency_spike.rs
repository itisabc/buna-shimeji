//! 透過スパイク検証（タスク #2）。**タスク #5 完了後に削除予定の example。**
//!
//! design.md §6 の手順に従い、WS_EX_LAYERED ウィンドウへの 2 つの描画経路を検証する:
//!
//! - 既定（ULW 経路）: windows-rs 直描き（CreateDIBSection + UpdateLayeredWindow）
//! - `--softbuffer`: softbuffer × WS_EX_LAYERED（不成立の視覚確認用対照実験）
//!
//! どちらも実物資産 `img/Shimeji/shime1.png`（128×128 RGBA）をモニタの作業領域中央に描画し、
//! 5 秒後に自動終了する。単一起動 mutex と WorkArea 列挙の実行確認を兼ねる。
//!
//! ## 実行
//!
//! ```text
//! cargo run --example transparency_spike              # ULW 経路（成立確認）
//! cargo run --example transparency_spike -- --softbuffer  # softbuffer（不成立確認）
//! ```
//!
//! ## 単一起動の確認
//!
//! 起動中（5 秒以内）にもう 1 つ起動すると、2 つ目は
//! 「another instance is already running → exit」を出して即終了する。

use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use tao::dpi::PhysicalPosition;
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};

use simeji::win::window::{premultiply_rgba_to_argb, LayeredWindow, SingleInstance};
use simeji::win::workarea::{enumerate_workareas, monitor_for_point};

const RUN_DURATION: Duration = Duration::from_secs(5);
const MUTEX_NAME: &str = "Local\\SimejiTransparencySpike";
const PNG_PATH: &str = "img/Shimeji/shime1.png";

fn main() -> Result<()> {
    let softbuffer_mode = std::env::args().any(|a| a == "--softbuffer");
    println!(
        "[mode] {}",
        if softbuffer_mode {
            "softbuffer × WS_EX_LAYERED (expected NOT to work)"
        } else {
            "CreateDIBSection + UpdateLayeredWindow (expected to work)"
        }
    );

    // --- 単一起動 mutex（Done criteria 確認を兼ねる） ---
    // guard はプロセス生存中保持する（match アーム内に束縛すると即ドロップされ、
    // mutex が解放されて単一起動が効かなくなる点に注意）。
    let _guard = match SingleInstance::acquire(MUTEX_NAME) {
        Ok(g) => {
            println!("[single-instance] mutex acquired (first instance)");
            g
        }
        Err(e) => {
            println!("[single-instance] {e} → exit now (this is the 2nd instance)");
            return Ok(());
        }
    };

    // --- WorkArea 列挙（Done criteria 確認を兼ねる） ---
    let areas = enumerate_workareas().context("enumerate_workareas")?;
    for (i, a) in areas.iter().enumerate() {
        println!(
            "[workarea] monitor #{i}: monitor={:?} work={:?}",
            (
                a.monitor.left,
                a.monitor.top,
                a.monitor.width(),
                a.monitor.height()
            ),
            (a.area.left, a.area.top, a.area.width(), a.area.height())
        );
    }
    if let Some(i) = monitor_for_point(0, 0, &areas) {
        println!("[workarea] point (0,0) belongs to monitor #{i}");
    }

    // --- 実物資産の PNG を読み込む（img/Shimeji/shime1.png: 128×128 RGBA） ---
    let img = image::ImageReader::open(PNG_PATH)
        .with_context(|| format!("open {PNG_PATH}"))?
        .decode()
        .with_context(|| format!("decode {PNG_PATH}"))?
        .into_rgba8();
    let (w, h) = (img.width(), img.height());
    let argb = premultiply_rgba_to_argb(img.as_raw());
    println!("[png] {PNG_PATH}: {w}x{h} RGBA → premultiplied 0xAARRGGBB");

    // 作業領域中央に配置
    let area = areas[0].area;
    let px = area.left + (area.width() - w as i32) / 2;
    let py = area.top + (area.height() - h as i32) / 2;

    let event_loop = EventLoopBuilder::<()>::with_user_event().build();
    let mut layered = LayeredWindow::create(&event_loop, w, h)?;
    layered
        .window()
        .set_outer_position(PhysicalPosition::new(px, py));
    println!("[window] created (layered, always-on-top): {w}x{h} at ({px},{py})");

    if !softbuffer_mode {
        // --- ULW 経路（成立期待） ---
        layered.resize(w, h)?;
        layered.present(&argb)?;
        println!("[ULW] UpdateLayeredWindow(ULW_ALPHA) succeeded");
        // 自動検証: DIB への書き込みが保持されていること（α 含む）
        let readback = layered.read_pixels()?;
        if readback[..] == argb[..] {
            println!("[ULW] DIB read-back matches (premultiplied α preserved in memory)");
        } else {
            println!("[ULW] DIB read-back MISMATCH");
        }
    } else {
        // --- softbuffer × WS_EX_LAYERED（不成立確認用の対照実験） ---
        // LayeredWindow は WS_EX_LAYERED 付きで作られている。softbuffer の
        // Windows バックエンドは BitBlt(SRCCOPY) のみで UpdateLayeredWindow を
        // 呼ばないため、レイヤードウィンドウには内容が表示されない。
        // 視覚確認: ウィンドウは見えない（または均一な黒/無描画）。
        println!("[softbuffer] presenting (expected: nothing becomes visible)");
        let context = softbuffer::Context::new(&event_loop).map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut surface = softbuffer::Surface::new(&context, layered.into_window())
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        surface
            .resize(w.max(1).try_into()?, h.max(1).try_into()?)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let mut buffer = surface.buffer_mut().map_err(|e| anyhow::anyhow!("{e}"))?;
        buffer[..].copy_from_slice(&argb);
        buffer.present().map_err(|e| anyhow::anyhow!("{e}"))?;
        println!("[softbuffer] present returned Ok (but no UpdateLayeredWindow was called)");
    }

    // --- 5 秒後に自動終了 ---
    // tao 0.37 の `run` は never を返すため、終了は LoopDestroyed で `process::exit` する。
    let deadline = Instant::now() + RUN_DURATION;
    event_loop.run(move |event, _, control_flow| match event {
        Event::NewEvents(StartCause::Init) => {
            *control_flow = ControlFlow::WaitUntil(deadline);
        }
        Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
            *control_flow = ControlFlow::Exit;
        }
        Event::LoopDestroyed => {
            std::process::exit(0);
        }
        _ => {}
    })
}
