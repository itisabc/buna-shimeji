//! 起動シーケンス・結線（タスク #10b-2c・design.md §1.4 / §1.9 / §1.10(c)）。
//!
//! 決定済みの起動手順（orch wiring 設定）を以下の順に実施する:
//! 1. `env_logger`（既定 info・`RUST_LOG` 尊重）
//! 2. exe ディレクトリ解決 → [`resolve_assets`]（Err は欠落パス入りで終了）
//! 3. `Settings::load(conf/settings.toml)`（handoff ⑪・Err は表示して終了）
//! 4. [`SingleInstance::acquire`]（`let _guard` 束縛でドロップ防止・
//!    AlreadyRunning は「既に起動しています」で終了）
//! 5. tao `EventLoop` 構築（UserEvent 型 = `()`）
//! 6. **EventLoop 構築後に** `load_materials`（1 回・XML 破損系は行番号付き表示で終了）
//! 7. [`App`]{resolver map / factory / rng / per-set tables} 組立。per-set tables は
//!    [`Manager::reload`]（materials 非空経路 = base 置換 + set_tables 全消し再登録・
//!    mascot 0 体のため behavior 再構築ループは no-op）で一括登録する
//! 8. settings 初期適用（allowed 6 種 → passthrough・disabled_behaviors →
//!    [`Manager::set_disabled_behaviors`]）
//! 9. 起動時 1 体 [`Manager::request_spawn_random`]
//! 10. トレイ（[`TrayMenuModel::build_tray`] + `TrayIconBuilder`）
//! 11. `EventLoop::run` glue（[`App`] の 3 経路: NewEvents / WindowEvent / LoopDestroyed）
//!
//! tao 0.37 実装の対応（ソース実読・event_loop.rs / platform_impl/windows/event_loop.rs
//! + event_loop/runner.rs）:
//! - **`AboutToWait` 変体は存在しない**（tao 0.37 は winit 由来でない独自変体:
//!   NewEvents / MainEventsCleared / RedrawEventsCleared / LoopDestroyed 等）。
//!   tick スケジュールは [`ControlFlow::WaitUntil`] 発火を起点とする:
//!   handler は各イベントバッチの先頭 `Event::NewEvents(StartCause)` で
//!   [`Manager::tick_due`] 判定 → 到来していれば tick 実行 →
//!   `WaitUntil(last_tick + 40ms)` を再設定する（タイマー約束の回復）。
//!   アイドル中は wait スレッドが時刻まで待ち `NewEvents(ResumeTimeReached)` を発火する
//!   （platform_impl L2360-2412 PROCESS_NEW_EVENTS_MSG / runner.rs `call_new_events`
//!   L373-420・`call_redraw_events_cleared` L421-424 実読）。各行 NewEvents で
//!   WaitUntil を設定し直すため、マウスイベント等でタイマーが中断されても回復する
//! - `EventLoop::run` は内部で `std::process::exit(exit_code)` を呼ぶ
//!   （event_loop.rs L220-233・platform_impl L264-292 実読）。よって handler 側では
//!   [`ControlFlow::Exit`] を設定するのみで終了する（design §1.9: exit flag は
//!   handler で process::exit をしない）。`LoopDestroyed` ではトレイの cleanup
//!   （`TrayIcon` drop = Shell_NotifyIcon(NIM_DELETE)）を tao 内部 exit の前に行い
//!   ゴーストアイコンを防ぐ
//! - `WindowEvent::MouseInput` は押下位置を持たないため、ポイントは
//!   [`EventLoopWindowTarget::cursor_position()`]（= `GetCursorPos` 物理 global・
//!   PMv2。Dragged の差分計算と同一空間 = スクリーン座標契約）で取得する
//!   （tao util.rs L216-218 実読）
//! - `EventLoopWindowTarget` 上で新しい窓が作れる
//!   （`WindowBuilder::build(&Target)` window.rs L610 実読）。spawn drain 直後の
//!   view 補充は handler 内 [`MascotView::create`]（target 受け・本タスクの
//!   小改修で win/render 側を target 受けへ変更）
//! - `WindowEvent::Resized` は完全無視（draw glue の `set_inner_size` 由来を含めて
//!   draw 経路以外で反応しない・⑥(N)）
//!
//! muda / tray-icon 実物 API（Cargo.lock 実物照合: tray-icon 0.24.2 / muda 0.19.3・
//! tray-icon は `pub mod menu { pub use muda::*; }` / `pub use muda::dpi` を re-export）:
//! - マスコット右クリック popup:
//!   [`tray_icon::menu::ContextMenu::show_context_menu_for_hwnd`]
//!   （position `None` = カーソル位置。Windows 版は `TrackPopupMenu(TPM_RETURNCMD)`
//!   同期追跡 → 選択時に MenuEvent 発行。platform_impl/windows/mod.rs L960 +
//!   L1039-1041 + menu_selected 経路 実読）。**同期的**
//!   （メニュー追跡中は handler をブロックする。モーダルループ中もメッセージポンプは回る
//!   ため wndproc / tao runner は動き続ける。挙動は手動確認対象）
//! - [`tray_icon::menu::MenuEvent::receiver`] の `try_recv()` で毎 tick drain
//! - トレイ本体: `TrayIconBuilder::with_menu / with_icon / with_tooltip / build`
//!   （icon 無しは Shell_NotifyIcon が NIF_ICON 無しで通知領域に表示されないため、
//!   アイコンは [`load_tray_icon_rgba`] で `img/icon.png` 優先 → 埋め込み既定。
//!   Java `Main.getIcon()` L764-792 準拠）
//!
//! Reload 結線（tray.rs `apply_tray_command` の Reload 分岐は lib API として残し・
//! wiring 側で自前処理）:
//! `load_materials` → Err なら log + 現状維持 / Ok なら resolver map 差し替え +
//! [`Manager::reload`] + 全 view [`MascotView::reset`] + 全 mascot
//! [`Mascot::set_needs_repaint(true)`]（新資産で同一 pose なら set_image が同値
//! no-op により needs_repaint が立たない経路の遮断・本タスクの lib 小改修）。

// release は GUI サブシステム（コンソール非表示）。debug（cargo run）は
// コンソール表示のまま（既定 false の show_console と整合）。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::cell::RefCell;
use std::collections::HashMap;
use std::os::windows::io::AsRawHandle;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{bail, Context};
use tao::event::{ElementState, Event, MouseButton, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop, EventLoopWindowTarget};
use tao::platform::windows::WindowExtWindows;
use tao::window::WindowId;
use tray_icon::menu::{ContextMenu, MenuEvent};
use tray_icon::TrayIcon;
use windows::core::HSTRING;
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Console::{
    AllocConsole, AttachConsole, GetConsoleWindow, SetConsoleOutputCP, SetStdHandle,
    ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use shimeji::app::assets::{resolve_assets, AssetDirs};
use shimeji::app::environment::Environment;
use shimeji::app::manager::Manager;
use shimeji::app::reload::load_materials;
use shimeji::i18n::{Lang, UiKey};
use shimeji::mascot::action::factory::XmlBehaviorFactory;
use shimeji::mascot::rng::JavaRandom;
use shimeji::render::imageset::ImageSet;
use shimeji::render::{MascotView, SpriteDraw};
use shimeji::tray::{
    apply_tray_command, load_tray_icon_rgba, Settings, TrayCommand, TrayContext, TrayMenuModel,
};
use shimeji::win::os_source::{ensure_window_above, restore_topmost_on_panic, Win32OsSource};
use shimeji::win::window::{MoveBatch, SingleInstance, SingleInstanceError};

/// 単一起動 mutex 名（ユーザーセッション内単一・`Local\` 名前空間）。
const SINGLE_INSTANCE_MUTEX: &str = "Local\\ShimejiSingleInstance";

/// [`Settings`] の走査 scale map を [`load_materials`] 入力の `HashMap` に変換する
/// ([`Settings::scales`] BTreeMap 契約 → HashMap 化・tray.rs Reload 分岐と同一変換)。
fn scales_of(settings: &Settings) -> HashMap<String, f64> {
    settings
        .scales()
        .iter()
        .map(|(set, scale)| (set.clone(), *scale))
        .collect()
}

/// exe 同場所の conf/img パス + トレイコンテキスト（起動後は不変）。
struct AppDirs {
    conf_dir: PathBuf,
    img_dir: PathBuf,
    tray_context: TrayContext,
}

impl AppDirs {
    fn new(conf_dir: PathBuf, img_dir: PathBuf, image_sets: Vec<String>) -> AppDirs {
        let tray_context = TrayContext {
            conf_dir: conf_dir.clone(),
            img_dir: img_dir.clone(),
            image_sets,
        };
        AppDirs {
            conf_dir,
            img_dir,
            tray_context,
        }
    }
}

/// イベントループ glue の状態（本文。
/// [`Manager`]（mascot 集合 + Environment + 行動表）/ settings / 视图群 /
/// トレイモデル + アイコン / popup 保持 / resolver map / draw warn 抑止备忘 の所有者）。
struct App {
    dirs: AppDirs,
    resolver_map: Rc<RefCell<HashMap<String, Arc<ImageSet>>>>,
    manager: Manager,
    settings: Settings,
    tray_model: TrayMenuModel,
    /// LoopDestroyed で明示 drop（Shell_NotifyIcon(NIM_DELETE) ＝ ゴーストアイコン防止）
    tray: Option<TrayIcon>,
    views: Vec<MascotView>,
    // mascot 右クリック popup のモデル（選択時 / drain 時に command_of 走査）
    menu_popups: Vec<TrayMenuModel>,
    /// UI 文言辞書（起動時 1 回ロード・popup 構築が参照・design §4.2）。
    lang: Lang,
    last_tick: Instant,
    // draw 失敗 / frame 取得失敗の連続 warn 抑止（index → 最後の warn 文字列）
    last_draw_warns: HashMap<usize, String>,
}

impl App {
    /// `Event::NewEvents`（イベントバッチ先頭・WinUntil 发火を含む）。
    fn on_new_events(
        &mut self,
        target: &EventLoopWindowTarget<()>,
        control_flow: &mut ControlFlow,
    ) {
        let elapsed = self.last_tick.elapsed();
        if Manager::tick_due(elapsed) {
            // ① tick（environment 更新 / spawn drain / remove / 全員 tick /
            // exit flag 設定がこの 1 呼び出しに全て含まれる）
            self.manager.tick(Instant::now());
            self.last_tick = Instant::now();

            // ② view 同期（不足分 create → ocos 仮寸法 1×1・次 draw で正寸化。
            // create 失敗は log + 次回再試行）
            self.sync_views(target);
            // 除去同期（tick retain で消えた mascot index → 降順 remove で対応維持）
            for index in self.manager.take_removed().into_iter().rev() {
                if index < self.views.len() {
                    self.views.remove(index);
                }
            }

            // ③ draw glue（draw + clear_needs_repaint）
            // ③a 効果音（Java `Mascot.apply` L699-707 は draw の前に同期実行される）。
            //     needs_repaint を消す前に判定するため handle_draws より先に呼ぶ。
            self.manager.play_pending_sounds();
            handle_draws(
                &mut self.views,
                &mut self.manager,
                &mut self.last_draw_warns,
            );

            // ③b #30 item 6: ピン保持中は保持マスコット窓をピン対象窓 W より前面へ
            // 再アサートする（W を TOPMOST にした際にマスコットが裏へ隠れるのを防ぐ）。
            // ピンが無い間は pinned_holder() が None のため新規コストなし。
            if let Some(holder) = self.manager.pinned_holder() {
                if let Some(view) = self.views.get(holder) {
                    ensure_window_above(view.window().hwnd());
                }
            }

            // ④ トレイ / popup コマンド drain・適用
            self.drain_menu_events();

            // ⑤ 終了判定（design §1.9: handler で process::exit はしない）
            if self.manager.should_exit() {
                *control_flow = ControlFlow::Exit;
                return;
            }
        }
        let elapsed = self.last_tick.elapsed();
        // タイマー約束の回復（elapsed < 間隔 = 残り時間 / >= 間隔 = 40ms クランプ）
        *control_flow = ControlFlow::WaitUntil(self.last_tick + Manager::next_delay(elapsed));
    }

    /// マスコット窓の不足分補充（1×1 → draw glue で正寸化）。
    fn sync_views(&mut self, target: &EventLoopWindowTarget<()>) {
        while self.views.len() < self.manager.count() {
            match MascotView::create(target, 1, 1) {
                Ok(view) => self.views.push(view),
                Err(err) => {
                    log::error!("failed to create mascot window (will retry on next tick): {err}");
                    break;
                }
            }
        }
    }

    /// マウス入力（window id → view index）。
    fn on_window_event(
        &mut self,
        target: &EventLoopWindowTarget<()>,
        window_id: WindowId,
        event: WindowEvent<'_>,
    ) {
        match event {
            // 完全無視（draw glue の set_inner_size 由来等・⑥(N)）
            WindowEvent::Resized(_) => {}
            // マスコット窓は閉じさせない（Java 版同様）
            WindowEvent::CloseRequested => {}
            WindowEvent::MouseInput { state, button, .. } => {
                let Some(index) = view_index_of(&self.views, window_id) else {
                    return;
                };
                match (state, button) {
                    (ElementState::Pressed, MouseButton::Left) => {
                        // スクリーン座標契約（GetCursorPos 物理・PMv2）
                        let point = target
                            .cursor_position()
                            .map_or((0, 0), |pos| (pos.x as i32, pos.y as i32));
                        if let Err(err) = self.manager.mouse_pressed_at(index, point) {
                            log::error!("failed to handle mouse press: {err}");
                            self.manager.dismiss_at(index);
                        }
                    }
                    (ElementState::Released, MouseButton::Left) => {
                        // #30 item 4/5: 解放点（スクリーン座標）を渡し、トグル ON なら
                        // 直下の窓を pin する。解除フック（トグル OFF / RestoreWindows /
                        // DismissAll / Reload / LoopDestroyed / panic）は 30-5 で配線済み。
                        let point = target
                            .cursor_position()
                            .map_or((0, 0), |pos| (pos.x as i32, pos.y as i32));
                        if let Err(err) = self.manager.mouse_released_at(index, point) {
                            log::error!("failed to handle mouse release: {err}");
                            self.manager.dismiss_at(index);
                        }
                    }
                    // Java `isPopupTrigger()` 準拠: Windows はボタンを離した時に開く
                    (ElementState::Released, MouseButton::Right) => {
                        self.open_popup(index);
                    }
                    _ => {}
                }
            }
            // ドラッグ追従用（Java Mascot.setCursorPosition 相当・スクリーン座標）
            WindowEvent::CursorMoved { position, .. } => {
                let Some(index) = view_index_of(&self.views, window_id) else {
                    return;
                };
                if let Ok(outer) = self.views[index].window().outer_position() {
                    let point = (outer.x + position.x as i32, outer.y + position.y as i32);
                    self.manager.set_cursor_position_at(index, Some(point));
                }
            }
            _ => {}
        }
    }

    /// マスコット右クリック（Java `Mascot` popup 相当）:
    /// 構築（[`Manager::behavior_menu_items`]）→ muda context menu 表示（同期追跡）。
    /// 選択された時のみモデルを保持する（未選択で閉じたモデルは回収者がいないため破棄）。
    fn open_popup(&mut self, index: usize) {
        let Some(set_name) = self.manager.image_set_name_at(index) else {
            return;
        };
        let Some(view) = self.views.get(index) else {
            return;
        };
        let menu_items = self.manager.behavior_menu_items(&set_name);
        let is_paused = self.manager.is_paused_at(index).unwrap_or(false);
        let model = TrayMenuModel::build_popup(
            index,
            &self.dirs.tray_context.image_sets,
            &menu_items,
            is_paused,
            &self.lang,
        );
        // muda / tray-icon 実物 API: position None = カーソル位置（doc 参照）。
        // 戻り値 = 項目選択の有無（platform_impl/windows/mod.rs show_context_menu_for_hwnd
        // L960-973 実読）: true の時点で MenuEvent は menu_selected 経由で同期送信済み
        // （同 L1238-1243）。
        let hwnd = view.window().hwnd();
        let selected = unsafe { model.menu().show_context_menu_for_hwnd(hwnd, None) };
        if selected {
            // 同期送信済みの MenuEvent を次ループ `drain_menu_events` が command_of で
            // 回収するため保持する（選択時のみ）。
            self.menu_popups.push(model);
        }
        // 未選択で閉じた場合（Esc / メニュー外クリック）は MenuEvent が発生しないため
        // 回収者がおらず、ここで model を破棄する（リーク根絶・A-4）。
    }

    /// トレイ / popup メニューコマンドの drain・適用（⑨）。
    /// - tray コマンド → [`apply_tray_command`（Reload は自前）] + `sync_allowed`
    /// - popup コマンド → [`apply_tray_command`]（SetAllowed 無し・sync 不要・
    ///   選択された popup は保持から除去）
    /// - 未知 id は warn で無視
    fn drain_menu_events(&mut self) {
        while let Ok(menu_event) = MenuEvent::receiver().try_recv() {
            let id = menu_event.id().clone();
            if let Some(command) = self.tray_model.command_of(&id) {
                self.apply_command(command);
                // Allowed トグル適用後の UI 整合（必ず・#9c 契約）
                self.tray_model.sync_allowed(&self.settings.allowed);
                continue;
            }
            if let Some(position) = self
                .menu_popups
                .iter()
                .position(|model| model.command_of(&id).is_some())
            {
                let popup = self.menu_popups.swap_remove(position);
                if let Some(command) = popup.command_of(&id) {
                    self.apply_command(command);
                }
                continue;
            }
            log::warn!("ignoring unknown menu id: {id:?}");
        }
    }

    /// コマンド適用（Reload は wiring 自前処理・それ以外は tray.rs 既存経路）。
    fn apply_command(&mut self, command: TrayCommand) {
        match command {
            TrayCommand::Reload => self.reload(),
            other => {
                apply_tray_command(
                    &mut self.manager,
                    &mut self.settings,
                    other,
                    &self.dirs.tray_context,
                );
            }
        }
    }

    /// Reload（wiring 自前処理・tray.rs `apply_tray_command` の Reload 分岐は
    /// resolver map を触れないためここで行う）。
    fn reload(&mut self) {
        let scales = scales_of(&self.settings);
        match load_materials(&self.dirs.conf_dir, &self.dirs.img_dir, &scales) {
            Ok(materials) => {
                // resolver map 差し替え（次構築から新 ImageSet を返す）
                *self.resolver_map.borrow_mut() = materials
                    .iter()
                    .map(|material| (material.name.clone(), Arc::clone(&material.image_set)))
                    .collect();
                // #32: per-set 定義集合も素材の一部なのでファクトリを作り直す
                //（reload より先に差し替える = 再構築は新定義集合で行われる）
                self.manager
                    .set_factory(Box::new(XmlBehaviorFactory::from_sets(
                        materials
                            .iter()
                            .map(|material| (material.name.clone(), Arc::clone(&material.actions))),
                    )));
                // 参照付け替え（ImageSet Arc / 行動表 / behavior 再構築）
                self.manager.reload(materials);
                // 全 view reset（ImageKey に set 名を含まないため必須・design §1.10(c)）
                self.views.iter_mut().for_each(MascotView::reset);
                // 全マスコットへ再描画要求（rebind は builds needs_repaint を立てない・
                // set_image 同値 no-op 経路の遮断）
                self.manager
                    .apply_all(|mascot| mascot.set_needs_repaint(true));
            }
            Err(err) => {
                log::error!("reload failed; keeping current state: {err}");
            }
        }
    }
}

// =====================================================================
// 結線ヘルパ群（動的3経路 = NewEvents クローズ / WindowEvent / LoopDestroyed）
// =====================================================================

/// `window_id` → view index 特定（マスコット窓の MouseInput / CursorMoved の送付先）。
fn view_index_of(views: &[MascotView], window_id: WindowId) -> Option<usize> {
    views
        .iter()
        .position(|view| view.window().id() == window_id)
}

/// 「同一内容の連続 warn を出さない」スパム抑止（index 毎に最後の warn 文字列を記憶）。
fn warn_once(memo: &mut HashMap<usize, String>, index: usize, message: impl FnOnce() -> String) {
    let message = message();
    if memo.get(&index) == Some(&message) {
        return;
    }
    memo.insert(index, message.clone());
    log::warn!("skipped drawing mascot #{index} due to a problem: {message}");
}

/// draw glue（各マスコットの描画）。
///
/// 2 段階で行う（[`MascotView`] の doc 参照）:
/// 1. [`MascotView::stage`] で位置を [`MoveBatch`] に積み、内容が変わった個体は DIB を更新して反映を予約
/// 2. [`MoveBatch::flush`] で全員の移動を 1 バッチ適用 → 予約分を [`MascotView::commit`]（ULW）
///
/// 移動を `SetWindowPos` の1バッチにまとめるのは、窓ごとの `SetWindowPos` が N に超線形で
/// 伸びるため（100 窓 33.2 ms/tick → 3.0 ms/tick・2026-09-20 movespike 実測）。
/// 成功した個体だけ `needs_repaint` を落とす（失敗は次 tick 再試行）。
///
/// manager と views は別所有物のため、[`Manager::apply_all`] のクロージャ内で
/// views[view_index] を借用できる（mascots 順 = views 順契約・実読確認済み）。
fn handle_draws(
    views: &mut [MascotView],
    manager: &mut Manager,
    last_draw_warns: &mut HashMap<usize, String>,
) {
    let mut moves = MoveBatch::new(views.len());
    // stage に成功した index（commit を試す対象）。
    let mut staged = vec![false; views.len()];

    let mut view_index = 0usize;
    manager.apply_all(|mascot| {
        let index = view_index;
        view_index += 1;
        let Some(view) = views.get_mut(index) else {
            // create 失敗で view 未補充 → 次回再試行
            return;
        };
        if !mascot.needs_repaint() {
            return;
        }
        // 画像状態（image_ref / center（flip 調整済み）/ 寸法）
        let Some(image_state) = mascot.image().cloned() else {
            // frame 取得失敗（None）→ log（連続抑止）+ clear しない（次 tick 再試行）
            warn_once(last_draw_warns, index, || {
                "image pose unresolved (mascot holds no image)".to_string()
            });
            return;
        };
        let image_set = mascot.image_set_arc();
        let Some(frame) = image_set.frames.get(&image_state.image_ref) else {
            warn_once(last_draw_warns, index, || {
                format!("image {} not found in image set", image_state.image_ref)
            });
            return;
        };

        // `pose_anchor` は「flip 前」のポーズアンカー（dx/dy）を渡す:
        // - [`MascotView::stage`] は flip=true 時に [`shimeji::render::flipped_offset_x`]
        //   = `width - pose_anchor.0`（Java `ImagePairs.getImage(right)` L85-91 の
        //   `rightImage.getWidth() - scaledAnchorX` 相当）をオフセットに使用する
        // - [`ImageState::center`] は flip 調整済み（look_right 時 width - dx・
        //   animation.rs L114-118）のため、flip=true 時は
        //   `width - center.0` で flip 前値（dx）に復元する（center をそのまま渡すと
        //   二重反転となり反転画像内アンカー位置が anchor からずれる）
        let flip = mascot.look_right();
        let pose_anchor = if flip {
            (
                i32::try_from(image_state.width).unwrap_or(i32::MAX) - image_state.center.0,
                image_state.center.1,
            )
        } else {
            image_state.center
        };

        match view.stage(
            SpriteDraw {
                image_ref: &image_state.image_ref,
                frame,
                flip,
                pose_anchor,
                anchor_pos: mascot.anchor(),
                tint: mascot.tint_rgb(),
                glow: mascot.tint_glow(),
            },
            &mut moves,
        ) {
            Ok(()) => staged[index] = true,
            Err(err) => {
                warn_once(last_draw_warns, index, || format!("draw failed: {err}"));
            }
        }
    });

    // 移動をまとめて適用してから内容（ULW）を反映する。ここで失敗した個体は
    // needs_repaint を残し、次 tick に再試行する。
    let moves_applied = moves.flush();

    let mut view_index = 0usize;
    manager.apply_all(|mascot| {
        let index = view_index;
        view_index += 1;
        if !staged.get(index).copied().unwrap_or(false) {
            return;
        }
        let Some(view) = views.get_mut(index) else {
            return;
        };
        match view.commit(moves_applied) {
            Ok(true) => {
                mascot.clear_needs_repaint();
                last_draw_warns.remove(&index);
            }
            // 移動が未反映（HDWP 失敗）→ needs_repaint を残して次 tick にやり直す。
            Ok(false) => {}
            Err(err) => {
                warn_once(last_draw_warns, index, || format!("draw failed: {err}"));
            }
        }
    });
}

// =====================================================================
// main
// =====================================================================

/// エントリポイント（エラー捕捉のみ・本体は [`try_main`]）。
/// Err は [`report_fatal`] でコンソールまたは MessageBox に表示して非ゼロ終了する。
fn main() {
    if let Err(err) = try_main() {
        report_fatal(&err);
        std::process::exit(1);
    }
}

fn try_main() -> anyhow::Result<()> {
    // 1. exe ディレクトリ（欠落パス入りエラーで終了）
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(|dir| dir.to_path_buf()))
        .context("could not determine the executable directory")?;

    // 2. settings.toml（パースエラー等はファイルパスを添えて終了・handoff ⑪。
    //    不在時は既定値が返る）
    let settings_path = exe_dir.join("conf").join("settings.toml");
    let settings = Settings::load(&settings_path)
        .with_context(|| format!("failed to load {}", settings_path.display()))?;

    // 3. show_console = true のときだけコンソールを確保（既定 false = 非表示）
    if settings.general.show_console {
        attach_console();
    }

    // 4. ログ（既定 info・RUST_LOG 尊重）
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    // 4b. panic フック: プロセス異常終了時に pin 中の WS_EX_TOPMOST を best-effort で
    //     剥がしてから既存フック（既定の panic 表示）へ委譲する（#30 item 5）。
    //     pin が無ければ `restore_topmost_on_panic` は no-op。
    {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            restore_topmost_on_panic();
            previous(info);
        }));
    }

    // 5. 資産ディレクトリ（欠落パス入りエラーで終了）
    let AssetDirs { conf_dir, img_dir } = resolve_assets(&exe_dir)?;

    // 5b. UI 文言辞書（env_logger 初期化後 = warn を消さない・design §4.2）
    let lang = Lang::load(&conf_dir.join("lang"), &settings.general.language);

    // 6. settings.toml が無ければ既定設定で初回生成する（失敗しても起動は止めない）
    match Settings::create_default_if_missing(&settings_path) {
        Ok(true) => log::info!("created {} with default settings", settings_path.display()),
        Ok(false) => {}
        Err(err) => log::warn!(
            "failed to create {} (continuing with default settings): {err}",
            settings_path.display()
        ),
    }

    // 7. 単一起動（ドロップ防止のため _guard 束縛）
    let _guard = match SingleInstance::acquire(SINGLE_INSTANCE_MUTEX) {
        Ok(guard) => guard,
        Err(SingleInstanceError::AlreadyRunning) => {
            bail!("another instance is already running (single instance only)");
        }
        Err(err @ SingleInstanceError::CreateFailed(_)) => return Err(err.into()),
    };

    // 8. tao EventLoop（UserEvent 型 = ()。構築は失敗時に panics・tao 仕様）
    let event_loop = EventLoop::<()>::new();

    // 9. 素材ロード（EventLoop 構築後・XML 破損等は行番号付きで終了）
    let scales = scales_of(&settings);
    let materials = match load_materials(&conf_dir, &img_dir, &scales) {
        Ok(materials) => materials,
        Err(err) => bail!("failed to load materials at startup: {err}"),
    };
    if materials.is_empty() {
        bail!(
            "no valid image sets found: expected image set folders (containing PNGs) under {}",
            img_dir.display()
        );
    }

    // 10. resolver map（main 所有・Reload で差し替え・resolver は Rc/RefCell 参照）
    let resolver_map: Rc<RefCell<HashMap<String, Arc<ImageSet>>>> = Rc::new(RefCell::new(
        materials
            .iter()
            .map(|material| (material.name.clone(), Arc::clone(&material.image_set)))
            .collect(),
    ));

    // set 一覧（トレイ「呼ぶ」選択元・spawn ランダム選択元・辞書順 = 既定 set が先頭）
    let image_sets: Vec<String> = materials
        .iter()
        .map(|material| material.name.clone())
        .collect();

    // factory 用定義集合（#32）: per-set の actions を Reload 素材から受け取り、
    // 構築時に set 名で選択する（BehaviorFactory::set_image_set が
    // マスコットの image set を構築直前に注入する）。scale は構築の都度
    // BehaviorTable::build_behavior_direct がマスコットの ImageSet.scale を
    // factory.set_scale で注入する（per-set scale を行動へ反映）。
    // disabled アニメの除去は load_materials が set 毎に済ませている。
    let factory = XmlBehaviorFactory::from_sets(
        materials
            .iter()
            .map(|material| (material.name.clone(), Arc::clone(&material.actions))),
    );

    let mut manager = Manager::new(
        Environment::new(Win32OsSource::new(
            settings.interactive_windows.whitelist.clone(),
            settings.interactive_windows.blacklist.clone(),
        )),
        materials[0].table.clone(),
        Box::new(factory),
        Box::new(JavaRandom::from_os()),
    );
    // per-set tables 登録（materials 非空経路 = base 置換 + set_tables 全消し再登録。
    // mascot 0 体なので behavior 再構築ループは no-op・rng 消費 0）
    manager.reload(materials);

    manager.set_image_set_resolver({
        let resolver_map = Rc::clone(&resolver_map);
        move |image_set_name| resolver_map.borrow().get(image_set_name).cloned()
    });

    // 11. settings 初期適用（#36: Sounds も実配線）
    manager.set_breeding_allowed(settings.allowed.breeding);
    manager.set_transients_enabled(settings.allowed.transients);
    manager.set_transformation_allowed(settings.allowed.transformation);
    manager.set_throwing_allowed(settings.allowed.throwing);
    manager.set_multiscreen(settings.allowed.multiscreen);
    manager.set_sounds_enabled(settings.allowed.sounds);

    // 11b. 効果音バックエンド（#36: Win32 PlaySound の最小実体。`img/` の探索順は
    //      Java Main.getSoundFilePath L446-461 準拠）。
    manager.set_sound_player(Box::new(shimeji::win::sound::WinSoundPlayer::new(
        img_dir.clone(),
    )));
    // #30 item 1: pin トグルの永続値を起動時に読み戻す（settings.toml で ON 保存済みの
    // 場合でもトグル OFF 配線漏れがないよう、他 Allowed と同じ経路で適用する）。
    manager.set_pin_dropped_window_allowed(settings.allowed.pin_dropped_window);
    // 無効 Behavior map（Manager passthrough・全体置換）
    manager.set_disabled_behaviors(settings.disabled_behaviors.clone());

    // 12. 起動時 1 体
    manager.request_spawn_random(&image_sets);

    // 13. トレイ（アイコンは img/icon.png 優先 → 埋め込み既定・Java Main.getIcon L764-792 準拠）
    let tray_model = TrayMenuModel::build_tray(&image_sets, &settings.allowed, &lang);
    let (icon_rgba, icon_width, icon_height) = load_tray_icon_rgba(&img_dir.join("icon.png"));
    let tray_icon = tray_icon::TrayIconBuilder::new()
        .with_menu(Box::new(tray_model.menu().clone()))
        .with_icon(
            tray_icon::Icon::from_rgba(icon_rgba, icon_width, icon_height)
                .context("failed to create tray icon")?,
        )
        .with_tooltip(lang.text(UiKey::Shimeji))
        .build()
        .context("failed to create tray icon")?;
    // 「LoopDestroyed で明示 drop」のため Option 化
    let tray = Some(tray_icon);

    let mut app = App {
        dirs: AppDirs::new(conf_dir, img_dir, image_sets),
        resolver_map,
        manager,
        settings,
        tray_model,
        tray,
        views: Vec::new(),
        menu_popups: Vec::new(),
        last_tick: Instant::now(),
        last_draw_warns: HashMap::new(),
        lang,
    };

    // 14. tao イベントループ（tick 1 本・描画/入力/トレイ受信は同じループ）
    event_loop.run(move |event, target, control_flow| {
        match event {
            Event::NewEvents(_) => app.on_new_events(target, control_flow),
            Event::WindowEvent {
                window_id, event, ..
            } => {
                app.on_window_event(target, window_id, event);
            }
            // 終了前 cleanup（tao 0.37 内部で process::exit(exit_code) が走るため
            // handler 側の明示 exit は不要。ゴーストトレイアイコン防止の drop のみ）
            Event::LoopDestroyed => {
                let app = &mut app;
                drop(app.tray.take());
                app.menu_popups.clear();
                // #30 item 5: pin 中の TOPMOST を終了時に剥がす（best-effort）。
                app.manager.unpin_pinned_window();
            }
            _ => {}
        }
    });
}

// =====================================================================
// コンソール制御（起動時の表示制御・致命的エラー表示）
// =====================================================================

/// 起動時にコンソールを確保し、標準出力 / 標準エラーを繋ぐ（settings.toml の
/// `general.show_console = true` 時のみ呼ぶ）。いずれの失敗も無視して続行する。
///
/// - 既にコンソールがある場合（debug ビルド・親コンソール継承時）は確保不要
/// - 無い場合: 親プロセスのコンソールへ [`AttachConsole`] を試し、失敗したら
///   [`AllocConsole`] で新規確保
/// - `CONOUT$` を読み書きモードで開き、そのハンドルを [`SetStdHandle`] で
///   `STD_OUTPUT_HANDLE` / `STD_ERROR_HANDLE` に設定する。ハンドルはプロセス
///   終了まで有効に保つ（[`std::mem::forget`] で File を drop させない）
/// - [`SetConsoleOutputCP`]`(65001)`（UTF-8）で日本語ログの文字化けを防ぐ
fn attach_console() {
    // 既存コンソール判定（GetConsoleWindow は無コンソール時 NULL）
    let has_console = unsafe { !GetConsoleWindow().is_invalid() };
    if !has_console {
        // 親コンソールへ attach（GUI サブシステムからの起動時）。無ければ新規確保。
        if unsafe { AttachConsole(ATTACH_PARENT_PROCESS) }.is_err() {
            let _ = unsafe { AllocConsole() };
        }
    }

    // CONOUT$ を読み書きで開いて std ハンドルへ接続する（開けなければ無視）
    if let Ok(file) = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("CONOUT$")
    {
        let handle = HANDLE(file.as_raw_handle());
        unsafe {
            let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
            let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
        }
        // ここで drop するとハンドルが閉じ std 出力が壊れる。プロセス終了まで保持する。
        std::mem::forget(file);
    }

    // 出力コードページを UTF-8 に（日本語ログの文字化け防止）
    let _ = unsafe { SetConsoleOutputCP(65001) };
}

/// 致命的起動エラーを表示する（[`main`] の Err 経路）。パニックしない。
///
/// - コンソールがある場合（[`GetConsoleWindow`] 非 null）: `eprintln!` で
///   原因チェーン付き（`{:#}`）を表示
/// - 無い場合（release 通常起動）: [`MessageBoxW`]（本文 = エラー、タイトル
///   "Shimeji"（英語固定・design §4.2）、`MB_OK | MB_ICONERROR`）
fn report_fatal(err: &anyhow::Error) {
    let has_console = unsafe { !GetConsoleWindow().is_invalid() };
    if has_console {
        eprintln!("{err:#}");
    } else {
        let text = HSTRING::from(format!("{err:#}"));
        let caption = HSTRING::from("Shimeji");
        unsafe {
            MessageBoxW(None, &text, &caption, MB_OK | MB_ICONERROR);
        }
    }
}
