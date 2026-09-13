//! タスク #10b-2b: glue API 3 点（`Mascot::image_set_arc` / `Manager::take_removed` /
//! `app::assets` 資産パス純関数）の契約テスト（RED）。
//!
//! 設計正本: .tmp/design.md L89-96（起動は exe と同じ場所の conf/ と img/ を使う）+
//! AGENTS §9。実装（#10b-2 分）未着手のため cargo test は compile error = RED が正常
//! （E0432: `shimeji::app::assets` 未存在 / E0599: `Mascot::image_set_arc`・
//! `Manager::take_removed` の未存在メソッド）。
//!
//! pin する API 契約（coder への指示・シグネチャは tests が固定する）:
//!
//! ```text
//! // ---- src/mascot/mod.rs ----
//! impl Mascot {
//!     // Arc clone を返す（不変 ImageSet の共有・bin/glue 側からの観測点）。
//!     // 取得後も ImageSet の公開 API（frame 等）で参照できることが本契約で、
//!     // Arc clone 性（strong count 等）の直接観察はしない
//!     pub fn image_set_arc(&self) -> Arc<ImageSet>
//! }
//!
//! // ---- src/app/manager.rs ----
//! impl Manager {
//!     // tick の retain（現 L320）で除去した mascot の「除去前 index」を昇順で
//!     // 蓄積し、呼び出しで drain する（呼ぶまで蓄積・呼んだら空になる）。
//!     // tick（現 L230-342）の逐語構造（環境更新→added→spawn drain→retain→
//!     // no_mascots→全員 tick→exit flag）は変えない
//!     pub fn take_removed(&mut self) -> Vec<usize>
//! }
//!
//! // ---- src/app/assets.rs（新規）+ src/app/mod.rs へ pub mod assets; 追加 ----
//! pub struct AssetDirs {
//!     pub conf_dir: PathBuf,
//!     pub img_dir: PathBuf,
//! }
//!
//! #[derive(Debug, Error)]          // thiserror 系・欠落種別で区別（Display 文言は不問）
//! pub enum AssetError {
//!     #[error("...")]
//!     ConfDirNotFound(PathBuf),
//!     #[error("...")]
//!     ImgDirNotFound(PathBuf),
//! }
//!
//! // 起動は exe と同じ場所の conf/ と img/ を使う（design L89-96・AGENTS §9）。
//! // img/ 配下の各ディレクトリ = 1 セット。XML 破損検証は load_materials 側の
//! // ため、ここはパス解決 + ディレクトリ存在検証のみ。
//! pub fn resolve_assets(exe_dir: &Path) -> Result<AssetDirs, AssetError>
//! // - conf/・img/ 両方存在 → Ok（conf_dir = exe_dir.join("conf") /
//! //   img_dir = exe_dir.join("img")）
//! // - conf 欠落 → Err(ConfDirNotFound(exe_dir.join("conf")))
//! // - img 欠落 → Err(ImgDirNotFound(exe_dir.join("img")))
//! // - 両方欠落 → Err（Err と種別判別のみ pin・どちらの種別でもよい = 検査順は不pin）
//! ```
//!
//! 自己完結（tests/common 不使用）。契約 1 は tests/mascot_test.rs の合成 ImageSet
//! fixture、契約 2 は tests/app_manager_ext_test.rs の FakeSource / ScriptedFactory /
//! deterministic Rng パターン、契約 3 は tests/app_reload_test.rs / tests/tray_test.rs
//! の temp dir（Windows temp・Drop で再帰削除）パターンを踏襲する。

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use shimeji::app::assets::{resolve_assets, AssetDirs, AssetError};
use shimeji::app::environment::{Environment, OsSource};
use shimeji::app::manager::Manager;
use shimeji::config::{BehaviorDef, BehaviorEntry, BehaviorsConfig, SequenceChild, VarMap};
use shimeji::mascot::behavior::{
    Action, ActionError, BehaviorError, BehaviorFactory, BehaviorTable,
};
use shimeji::mascot::{EnvironmentView, Mascot, Rect, Rng};
use shimeji::render::imageset::{Frame, ImageSet};

// =====================================================================
// 合成データヘルパ（自己完結・app_manager_ext_test.rs / mascot_test.rs 踏襲）
// =====================================================================

/// 矩形ヘルパ。
fn rect(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
    Rect {
        left,
        top,
        right,
        bottom,
    }
}

/// フレーム無し ImageSet（mascot 初期生成用・既存流儀踏襲）。
fn empty_image_set(name: &str) -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

/// 指定フレームだけを持つ ImageSet（契約 1・app_reload_test.rs の
/// image_set_with と同形。寸法で参照結果を判別する）。
fn image_set_with(name: &str, frames: &[(&str, u32, u32)]) -> Arc<ImageSet> {
    let mut map = BTreeMap::new();
    for (file, width, height) in frames {
        map.insert(
            file.to_string(),
            Frame {
                width: *width,
                height: *height,
                rgba: vec![0; *width as usize * *height as usize * 4],
            },
        );
    }
    Arc::new(ImageSet {
        name: name.to_string(),
        frames: map,
        warnings: Vec::new(),
        scale: 1.0,
    })
}

// =====================================================================
// Environment — fake source（app_manager_ext_test.rs 踏襲）
// =====================================================================

/// OS 供給の状態。
#[derive(Default)]
struct FakeState {
    monitors: Vec<(Rect, Rect)>,
    cursor: Option<(i32, i32)>,
    active_window: Option<(i64, Rect)>,
    windows: Vec<(i64, Rect)>,
    moved: Vec<(i64, i32, i32)>,
    raised: Vec<i64>,
}

struct FakeSource {
    state: Rc<RefCell<FakeState>>,
}

impl OsSource for FakeSource {
    fn monitors(&self) -> Vec<(Rect, Rect)> {
        self.state.borrow().monitors.clone()
    }

    fn cursor_position(&self) -> Option<(i32, i32)> {
        self.state.borrow().cursor
    }

    fn active_window(&self) -> Option<(i64, Rect)> {
        self.state.borrow().active_window
    }

    fn move_window(&self, id: i64, x: i32, y: i32) {
        self.state.borrow_mut().moved.push((id, x, y));
    }

    fn windows(&self) -> Vec<(i64, Rect)> {
        self.state.borrow().windows.clone()
    }

    fn raise_window(&self, id: i64) {
        self.state.borrow_mut().raised.push(id);
    }
}

/// 単一モニタ構成の Environment（monitor (0,0,1920,1080) / work area (0,0,1920,1040)）。
fn single_monitor_env() -> Environment {
    let state = Rc::new(RefCell::new(FakeState {
        monitors: vec![(rect(0, 0, 1920, 1080), rect(0, 0, 1920, 1040))],
        cursor: None,
        active_window: None,
        windows: Vec::new(),
        moved: Vec::new(),
        raised: Vec::new(),
    }));
    Environment::new(FakeSource {
        state: state.clone(),
    })
}

// =====================================================================
// Manager fixture（scripted action + 合成 config・app_manager_ext_test.rs 踏襲）
// =====================================================================

/// Java Math.random 相当の [0,1) 乱数。キューを順に返し、枯渇 = 過剰消費として
/// panic する。
struct BoxedRng {
    values: Vec<f64>,
    consumed: usize,
}

impl Rng for BoxedRng {
    fn unit(&mut self) -> f64 {
        let v = *self
            .values
            .get(self.consumed)
            .unwrap_or_else(|| panic!("BoxedRng 枯渇（{} 回要求）", self.consumed + 1));
        self.consumed += 1;
        v
    }
}

/// 既定乱数（すべて 0.5・64 回分）。
fn unit_rng() -> Box<BoxedRng> {
    Box::new(BoxedRng {
        values: vec![0.5; 64],
        consumed: 0,
    })
}

/// 常時継続・正常・rng を消費しない action（安定動作）。
struct ScriptedAction {
    has_next_calls: usize,
}

impl Action for ScriptedAction {
    fn init(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }

    fn has_next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<bool, ActionError> {
        self.has_next_calls += 1;
        Ok(true)
    }

    fn next(
        &mut self,
        _mascot: &mut Mascot,
        _env: &dyn EnvironmentView,
        _rng: &mut dyn Rng,
    ) -> Result<(), ActionError> {
        Ok(())
    }
}

/// 参照名に応じた action を返すファクトリ（本テストは遷移させない）。
struct ScriptedFactory;

impl BehaviorFactory for ScriptedFactory {
    fn build_action(&mut self, child: &SequenceChild) -> Result<Box<dyn Action>, BehaviorError> {
        match child {
            SequenceChild::Ref { .. } => Ok(Box::new(ScriptedAction { has_next_calls: 0 })),
            SequenceChild::Inline(_) => Err(BehaviorError::UnknownBehavior(
                "(inline は本テストで未使用)".to_string(),
            )),
        }
    }
}

fn row_entry(name: &str, frequency: i32, hidden: bool, toggleable: bool) -> BehaviorEntry {
    BehaviorEntry::Single(BehaviorDef {
        name: name.to_string(),
        frequency,
        hidden,
        toggleable,
        action: SequenceChild::Ref {
            name: name.to_string(),
            attrs: VarMap::new(),
        },
        next: None,
    })
}

/// 非 toggleable・非 hidden の行 1 つ。
fn row(name: &str, frequency: i32) -> BehaviorEntry {
    row_entry(name, frequency, false, false)
}

fn table(entries: Vec<BehaviorEntry>) -> BehaviorTable {
    BehaviorTable::new(&BehaviorsConfig { entries })
}

/// no_mascots で exit_flag を立てない構成（既存流儀踏襲）。
fn make_manager(env: Environment, factory: ScriptedFactory) -> Manager {
    let mut manager = Manager::new(
        env,
        table(vec![row("Walk", 100)]),
        Box::new(factory),
        unit_rng(),
    );
    manager.set_exit_on_last_removed(false);
    manager
}

/// 指定 set の新規 Mascot（fresh・behavior 無し）。
fn mascot_of_set(image_set: &str, anchor: (i32, i32)) -> Mascot {
    Mascot::new(image_set, empty_image_set(image_set), anchor)
}

/// マスコット集合の観測スナップショット（apply_all 経由の公開 API のみ）。
#[derive(Debug, Clone, PartialEq)]
struct MascotView {
    image_set: String,
    anchor: (i32, i32),
    remove_pending: bool,
}

fn snapshot(manager: &mut Manager) -> Vec<MascotView> {
    let mut out: Vec<MascotView> = Vec::new();
    manager.apply_all(|m| {
        out.push(MascotView {
            image_set: m.image_set_name().to_string(),
            anchor: m.anchor(),
            remove_pending: m.remove_pending(),
        });
    });
    out
}

// =====================================================================
// temp dir fixture（契約 3・app_reload_test.rs の TempAssets 踏襲）
// =====================================================================

/// exe 起動ディレクトリの使い捨て代替（Drop で再帰削除・conf/img は必要時生成）。
struct TempExe {
    root: PathBuf,
}

impl TempExe {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("shimeji_t10b2b_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // 前回残留の掃除
        std::fs::create_dir_all(&root).expect("ルートディレクトリを作れる");
        TempExe { root }
    }

    fn conf_dir(&self) -> PathBuf {
        self.root.join("conf")
    }

    fn img_dir(&self) -> PathBuf {
        self.root.join("img")
    }

    fn ensure_conf(&self) {
        std::fs::create_dir_all(self.conf_dir()).expect("conf ディレクトリを作れる");
    }

    fn ensure_img(&self) {
        std::fs::create_dir_all(self.img_dir()).expect("img ディレクトリを作れる");
    }
}

impl Drop for TempExe {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// =====================================================================
// 契約 1: Mascot::image_set_arc（src/mascot/mod.rs 追加）
// =====================================================================

/// 取得した Arc<ImageSet> が ImageSet の公開 API（frame）で参照でき、
/// mascot の image_set_name() と整合する画像を引ける。
#[test]
fn image_set_arc_exposes_frames_consistent_with_image_set_name() {
    let set = image_set_with("TestSet", &[("shime1.png", 64, 32), ("shime2.png", 48, 48)]);
    let mascot = Mascot::new("TestSet", set, (100, 200));
    assert_eq!(mascot.image_set_name(), "TestSet");

    let arc = mascot.image_set_arc();
    assert_eq!(
        arc.name, "TestSet",
        "取得した Arc は mascot の image_set_name と整合する set"
    );
    let frame = arc
        .frame("shime1.png")
        .expect("Arc 経由で shime1.png を参照できる");
    assert_eq!((frame.width, frame.height), (64, 32));
    assert_eq!(
        arc.frame("shime2.png").map(|f| (f.width, f.height)),
        Some((48, 48))
    );
    assert!(arc.frame("missing.png").is_none(), "無い参照は None");
}

/// 返り値は所有する Arc（共有データ）である: mascot を drop した後も
/// Arc 経由で同じ画像を参照できる（&ImageSet でなく Arc<ImageSet> を返す pin）。
#[test]
fn image_set_arc_is_owned_and_outlives_the_mascot() {
    let mascot = Mascot::new(
        "TestSet",
        image_set_with("TestSet", &[("shime1.png", 64, 32)]),
        (0, 0),
    );
    let arc = mascot.image_set_arc();
    drop(mascot);
    let frame = arc
        .frame("shime1.png")
        .expect("mascot 解放後も Arc 経由で同一フレームを参照できる");
    assert_eq!((frame.width, frame.height), (64, 32));
}

// =====================================================================
// 契約 2: Manager::take_removed（src/app/manager.rs・tick L230-342 逐語維持）
// =====================================================================

/// dispose → tick → take_removed で除去 index が返る。
/// dismiss_at(1) で立てた remove_pending が tick の retain で除去され、
/// 除去された個体（SetB・除去前 index 1）の位置が返る。
#[test]
fn take_removed_reports_index_after_dispose_and_tick() {
    let mut manager = make_manager(single_monitor_env(), ScriptedFactory);
    manager.add(mascot_of_set("SetA", (0, 500)));
    manager.add(mascot_of_set("SetB", (100, 500)));
    manager.add(mascot_of_set("SetA", (200, 500)));
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 3);
    assert_eq!(
        manager.take_removed(),
        Vec::<usize>::new(),
        "この tick では除去していない"
    );

    manager.dismiss_at(1); // 2 番目（SetB）を削除予定に
    let snap = snapshot(&mut manager);
    assert!(
        snap[1].remove_pending && snap[1].image_set == "SetB",
        "dismiss_at は remove_pending を立てるのみ（tick で除去）"
    );

    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2, "SetB が除去される");
    assert_eq!(manager.count_of("SetB"), 0);
    assert_eq!(
        manager.take_removed(),
        vec![1],
        "除去前 index（SetB の位置）が返る"
    );
}

/// 複数同時除去 → 昇順で全部返る。
#[test]
fn take_removed_returns_all_indices_ascending_for_simultaneous_removals() {
    let mut manager = make_manager(single_monitor_env(), ScriptedFactory);
    for index in 0..5 {
        manager.add(mascot_of_set("SetA", (index * 100, 500)));
    }
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 5);

    manager.dismiss_at(4);
    manager.dismiss_at(2);
    manager.dismiss_at(0);
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 2);
    assert_eq!(
        manager.take_removed(),
        vec![0, 2, 4],
        "同時除去は昇順で全部返る"
    );
}

/// 除去なし tick → 空 Vec（起動直後の take も空）。
#[test]
fn take_removed_is_empty_when_nothing_was_removed() {
    let mut manager = make_manager(single_monitor_env(), ScriptedFactory);
    assert_eq!(manager.take_removed(), Vec::<usize>::new(), "tick 前でも空");

    manager.add(mascot_of_set("SetA", (0, 500)));
    manager.add(mascot_of_set("SetA", (100, 500)));
    manager.tick(Instant::now());
    manager.tick(Instant::now());
    assert_eq!(
        manager.take_removed(),
        Vec::<usize>::new(),
        "除去なし tick では空"
    );
}

/// take_removed は drain 型: 1 回目で返したら 2 回目は空。
#[test]
fn take_removed_is_drained_by_each_call() {
    let mut manager = make_manager(single_monitor_env(), ScriptedFactory);
    for index in 0..3 {
        manager.add(mascot_of_set("SetA", (index * 100, 500)));
    }
    manager.tick(Instant::now());

    manager.dismiss_at(2);
    manager.tick(Instant::now());
    assert_eq!(manager.take_removed(), vec![2], "1 回目で返す");
    assert_eq!(
        manager.take_removed(),
        Vec::<usize>::new(),
        "2 回目は空（drain 型）"
    );

    // drain 後・除去なし tick でも空のまま
    manager.tick(Instant::now());
    assert_eq!(manager.take_removed(), Vec::<usize>::new());
}

/// take を呼ばずに複数 tick すると蓄積される（毎 tick リセットでない = drain 型 pin）。
#[test]
fn take_removed_accumulates_across_ticks_until_taken() {
    let mut manager = make_manager(single_monitor_env(), ScriptedFactory);
    manager.add(mascot_of_set("SetA", (0, 500)));
    manager.add(mascot_of_set("SetB", (100, 500)));
    manager.add(mascot_of_set("SetC", (200, 500)));
    manager.tick(Instant::now());

    // tick 1: SetC（当時 index 2）を除去・take は呼ばない
    manager.dismiss_at(2);
    manager.tick(Instant::now());
    // tick 2: 残り 2 体の SetA（当時 index 0）を除去・take は呼ばない
    manager.dismiss_at(0);
    manager.tick(Instant::now());

    let removed = manager.take_removed();
    assert_eq!(removed.len(), 2, "2 tick 分の除去が take まで蓄積される");
    assert!(removed.contains(&2), "tick 1 の除去（当時 index 2）が残る");
    assert!(removed.contains(&0), "tick 2 の除去（当時 index 0）が残る");
    assert_eq!(
        manager.take_removed(),
        Vec::<usize>::new(),
        "take 後は空（drain）"
    );
}

/// 返る index は「除去前の位置」: 3 体中 2・3 番目を除去 → [1, 2]。
/// tick 後に残る先頭個体（旧 index 0）の新位置 0 とは別物。
#[test]
fn take_removed_indices_refer_to_pre_removal_positions() {
    let mut manager = make_manager(single_monitor_env(), ScriptedFactory);
    manager.add(mascot_of_set("SetA", (0, 500)));
    manager.add(mascot_of_set("SetB", (100, 500)));
    manager.add(mascot_of_set("SetC", (200, 500)));
    manager.tick(Instant::now());

    // 除去前の並びを pin（index 1 = SetB / index 2 = SetC）
    let snap = snapshot(&mut manager);
    assert_eq!(snap[1].image_set, "SetB");
    assert_eq!(snap[2].image_set, "SetC");

    manager.dismiss_at(1);
    manager.dismiss_at(2);
    manager.tick(Instant::now());
    assert_eq!(manager.count(), 1);

    let removed = manager.take_removed();
    assert_eq!(
        removed,
        vec![1, 2],
        "除去前位置の昇順（残存個体の新位置 0 や renumber した [0] / [0, 1] でない）"
    );

    // 残ったのは旧 index 0 の個体（SetA・anchor (0,500)）であることの確認
    let snap = snapshot(&mut manager);
    assert_eq!(
        snap,
        vec![MascotView {
            image_set: "SetA".to_string(),
            anchor: (0, 500),
            remove_pending: false,
        }],
        "残存個体は旧 index 0 の SetA"
    );
}

// =====================================================================
// 契約 3: 資産パス純関数 resolve_assets（src/app/assets.rs 新規）
// =====================================================================

/// conf/・img/ 両方存在 → Ok で exe_dir.join("conf") / exe_dir.join("img")。
#[test]
fn resolve_assets_returns_conf_and_img_under_exe_dir() {
    let exe = TempExe::new("both_ok");
    exe.ensure_conf();
    exe.ensure_img();

    let AssetDirs { conf_dir, img_dir } =
        resolve_assets(&exe.root).expect("両ディレクトリが揃っていれば Ok");
    assert_eq!(
        conf_dir,
        exe.conf_dir(),
        "conf_dir = exe_dir.join(\"conf\")"
    );
    assert_eq!(img_dir, exe.img_dir(), "img_dir = exe_dir.join(\"img\")");
}

/// conf 欠落 → Err・欠落種別は conf 側で判別でき、パスも conf 側を指す。
#[test]
fn resolve_assets_reports_conf_dir_missing() {
    let exe = TempExe::new("conf_missing");
    exe.ensure_img(); // img は在る・conf だけ無い

    let err = resolve_assets(&exe.root).expect_err("conf 欠落は Err");
    match err {
        AssetError::ConfDirNotFound(path) => {
            assert_eq!(path, exe.conf_dir(), "報告パスは conf 側");
        }
        other => panic!("conf 欠落なのに別種別で報告された: {other:?}"),
    }
}

/// img 欠落 → Err・欠落種別は img 側で判別でき、パスも img 側を指す。
#[test]
fn resolve_assets_reports_img_dir_missing() {
    let exe = TempExe::new("img_missing");
    exe.ensure_conf(); // conf は在る・img だけ無い

    let err = resolve_assets(&exe.root).expect_err("img 欠落は Err");
    match err {
        AssetError::ImgDirNotFound(path) => {
            assert_eq!(path, exe.img_dir(), "報告パスは img 側");
        }
        other => panic!("img 欠落なのに別種別で報告された: {other:?}"),
    }
}

/// 両方欠落 → Err（種別判別可能であることのみ pin・どちらの種別でもよい）。
/// ついでに std::error::Error 実装（thiserror 系）を確認する。
#[test]
fn resolve_assets_errors_when_both_dirs_missing() {
    fn assert_is_error<E: std::error::Error>(_e: &E) {}

    let exe = TempExe::new("both_missing");
    let err = resolve_assets(&exe.root).expect_err("両方欠落は Err");
    assert!(
        matches!(
            &err,
            AssetError::ConfDirNotFound(_) | AssetError::ImgDirNotFound(_)
        ),
        "欠落種別が判別できる（検査順は不問）"
    );
    assert_is_error(&err);
}
