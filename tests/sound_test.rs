//! #36 効果音の配線（Pose の Sound/Volume → apply_pose → 再生/停止要求）の契約テスト。
//!
//! Java 正本: `.tmp/java-ref/Pose.java` L26-31（`setSound`）/
//! `Main.java` L446-461（音声探索順）/ `Mascot.java` L699-707（再生）/
//! `action/Mute.java` L28-52（停止）。
//!
//! 音声の実体（`PlaySound`）は呼ばない: パス解決の順序と「鳴らすか否か」の判定までを
//! 検証する（実機の聴取は手動手順・AGENTS §9）。実再生の呼び出しは
//! `SoundPlayer::play_if_idle` 内の 1 箇所のみ（design §1.10 (z-12)）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use shimeji::app::environment::SoundPlayer;
use shimeji::config::Pose;
use shimeji::mascot::animation::apply_pose;
use shimeji::mascot::Mascot;
use shimeji::render::imageset::ImageSet;
use shimeji::win::sound::{resolve_sound_path, wav_duration_ms, WinSoundPlayer};

fn temp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("shimeji_sound_{}_{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("temp dir を作れる");
    root
}

/// 音声ファイルを作る（中身は読まれない。存在だけが解決条件）。
fn touch(path: &Path) {
    std::fs::create_dir_all(path.parent().expect("親ディレクトリ")).expect("親を作れる");
    std::fs::write(path, b"RIFF").expect("ファイルを作れる");
}

/// 最小の PCM WAV を書く（`wav_duration_ms` の算出だけに使う。再生はしない）。
fn write_wav(path: &Path, byte_rate: u32, data_len: u32) {
    std::fs::create_dir_all(path.parent().expect("親ディレクトリ")).expect("親を作れる");
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVE");
    bytes.extend_from_slice(b"fmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes()); // PCM
    bytes.extend_from_slice(&1u16.to_le_bytes()); // mono
    bytes.extend_from_slice(&44_100u32.to_le_bytes());
    bytes.extend_from_slice(&byte_rate.to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    bytes.resize(bytes.len() + data_len as usize, 0);
    std::fs::write(path, bytes).expect("WAV を作れる");
}

fn empty_set() -> Arc<ImageSet> {
    Arc::new(ImageSet {
        name: "TestSet".to_string(),
        frames: BTreeMap::new(),
        warnings: Vec::new(),
        scale: 1.0,
    })
}

fn pose_with_sound(sound: Option<&str>, volume: f32) -> Pose {
    Pose {
        image: "walk.png".to_string(),
        anchor: (64, 64),
        velocity: (0, 0),
        duration: 10,
        sound: sound.map(str::to_string),
        volume,
    }
}

/// Java `Main.getSoundFilePath` L446-461 の探索順:
/// `img/<set>/sound/<file>` → `sound/<set>/<file>` → `sound/<file>`（最初に存在するもの）。
/// どこにも無ければ None（Java は FileNotFoundException）。
#[test]
fn resolve_sound_path_prefers_image_set_local_sound_directory() {
    let root = temp_root("resolve");
    let img = root.join("img");
    std::fs::create_dir_all(&img).expect("img を作れる");
    let in_set = img.join("TestSet").join("sound").join("se.wav");
    let in_sound_set = root.join("sound").join("TestSet").join("se.wav");
    let in_sound = root.join("sound").join("se.wav");
    touch(&in_set);
    touch(&in_sound_set);
    touch(&in_sound);

    assert_eq!(
        resolve_sound_path(&img, "TestSet", "se.wav"),
        Some(in_set.clone()),
        "img/<set>/sound/ が最優先"
    );
    std::fs::remove_file(&in_set).expect("削除できる");
    assert_eq!(
        resolve_sound_path(&img, "TestSet", "se.wav"),
        Some(in_sound_set.clone()),
        "次に sound/<set>/"
    );
    std::fs::remove_file(&in_sound_set).expect("削除できる");
    assert_eq!(
        resolve_sound_path(&img, "TestSet", "se.wav"),
        Some(in_sound),
        "最後に sound/"
    );
    assert_eq!(
        resolve_sound_path(&img, "TestSet", "missing.wav"),
        None,
        "不在は None"
    );
}

/// 同じ音の再要求は、再生長 + 余裕（150ms）が経過するまで見送る（= 鳴っている音を
/// 途中で切らない・design §1.10 (z-12) 差異 3）。経過後は再び鳴らす。
/// 別の音・初回は即、停止（Mute）後も即鳴る。
#[test]
fn plan_play_waits_for_wav_to_finish_before_retrigger() {
    let root = temp_root("plan");
    // 0.1 秒の WAV（byte_rate 88200 / data 8820 バイト）
    write_wav(&root.join("sound").join("se.wav"), 88_200, 8_820);
    touch(&root.join("sound").join("other.wav"));
    let mut player = WinSoundPlayer::new(root.join("img"));
    let t0 = Instant::now();

    let first = player
        .plan_play("TestSet", "se.wav", t0)
        .expect("解決できる");
    assert!(first.ends_with("se.wav"), "初回は鳴らす");

    assert_eq!(
        player.plan_play("TestSet", "se.wav", t0 + Duration::from_millis(50)),
        None,
        "再生中とみなせる間は呼ばない（デバイス遅延で音を切らないため）"
    );
    assert_eq!(
        player.plan_play("TestSet", "se.wav", t0 + Duration::from_millis(200)),
        None,
        "再生長ちょうど（100ms）でも余裕（150ms）までは呼ばない"
    );
    assert_eq!(
        player.plan_play("TestSet", "se.wav", t0 + Duration::from_millis(250)),
        Some(first.clone()),
        "余裕を過ぎたら鳴り直す（Java はクリップ終了ごとに再開する）"
    );

    let other = player
        .plan_play("TestSet", "other.wav", t0)
        .expect("別の音は即鳴る");
    assert!(other.ends_with("other.wav"));

    assert_eq!(
        player.plan_play("TestSet", "missing.wav", t0),
        None,
        "存在しない音は鳴らせない（Java FileNotFound 相当）"
    );

    player.stop("TestSet", None);
    assert_eq!(
        player.plan_play("TestSet", "other.wav", t0),
        Some(other),
        "停止（Mute）後は再生状態が消えて即鳴る"
    );
}

/// 再生長が不明（WAV 以外・ヘッダ不正）の音は、鳴り終わりを判定できないため
/// 再要求しない（鳴っている音を切らない安全側・差異 3）。
#[test]
fn plan_play_does_not_retrigger_sound_with_unknown_duration() {
    let root = temp_root("unknown");
    touch(&root.join("sound").join("se.wav"));
    assert_eq!(
        wav_duration_ms(&root.join("sound").join("se.wav")),
        None,
        "RIFF ヘッダだけのファイルは再生長が分からない"
    );
    let mut player = WinSoundPlayer::new(root.join("img"));
    let t0 = Instant::now();

    assert!(
        player.plan_play("TestSet", "se.wav", t0).is_some(),
        "初回は鳴らす"
    );
    assert_eq!(
        player.plan_play("TestSet", "se.wav", t0 + Duration::from_secs(60)),
        None,
        "長さ不明の音は再要求しない"
    );
}

/// Java `Pose.apply` L30 逐語: ポーズ適用は常に `setSound` する（音の無いポーズは
/// null 相当で上書き）。音量（`Volume`）も対で運ぶ。
#[test]
fn apply_pose_sets_mascot_sound_and_volume() {
    let mut mascot = Mascot::new("TestSet", empty_set(), (100, 200));

    apply_pose(&pose_with_sound(Some("se.wav"), 0.5), &mut mascot);
    assert_eq!(
        mascot.sound(),
        Some("se.wav"),
        "ポーズの Sound が保留音になる"
    );
    assert_eq!(mascot.sound_volume(), 0.5, "ポーズの Volume が対で入る");

    apply_pose(&pose_with_sound(None, 0.0), &mut mascot);
    assert_eq!(
        mascot.sound(),
        None,
        "音を持たないポーズは保留音をクリアする"
    );
    assert_eq!(mascot.sound_volume(), 0.0);
}
