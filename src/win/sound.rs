//! [`SoundPlayer`](crate::app::environment::SoundPlayer) の実 Win32 実装（#36）。
//!
//! Java 正本を仕様とする:
//! - `Main.getSoundFilePath` L446-461: 音声ファイルの探索順
//!   `img/<set>/sound/<file>` → `sound/<set>/<file>` → `sound/<file>`
//! - `Mascot.apply` L699-707: `Sounds.isEnabled() && sound != null` かつ
//!   `!clip.isRunning()` なら頭から再生
//! - `Mute.apply` L28-52: `Some` = その音の再生中クリップを停止 / `None` = 全停止
//!
//! 再生は `PlaySoundW(SND_FILENAME | SND_ASYNC | SND_NODEFAULT)`、停止は公式の
//! `PlaySound(NULL, 0, 0)`（= [`SND_SYNC`] で NULL・winmm）。
//!
//! 意図的差異（design §1.10 (z-12)・Java 一致検証時に差し引くこと）:
//! 1. **同時再生は 1 音のみ**: Java は音声キー毎に `Clip` を保持して重ねられるが、
//!    `PlaySound` はプロセス内 1 ストリームのため、別の音を要求すると前の音は止まる
//! 2. **`Volume` は無視する**: `PlaySound` に音量 API が無い（要求値は受け取るが使わない）
//! 3. **「再生中」は WAV の再生長 + 余裕で判定し、その間は `PlaySound` を呼ばない**:
//!    `PlaySound` は再生状態を問い合わせられない。`SND_NOSTOP` は公式には「*別の*
//!    サウンドが再生中」の場合しか FALSE を返さず、**同じ音の再要求時の挙動は不記載**で、
//!    実測でも環境により正規の再要求（鳴り終わり後の再開）まで FALSE で落ちた。そこで
//!    [`wav_duration_ms`] の長さに [`RETRIGGER_MARGIN`] を足した時刻までは**呼ばずに**
//!    見送る。再生中に `PlaySound` を呼ぶと Windows が現在の再生を打ち切るため、
//!    長さちょうどで再要求するとデバイス遅延のぶん音が途中で切れてプツッと鳴る
//!    （実機で確認済み）。長さ不明の形式は再要求しない（安全側）
//! 4. **音声ファイル不在は log warn で継続**: Java は `AnimationBuilder` L221-237 が
//!    読込時に IOException を投げ起動失敗になる（Rust は再生要求時に解決する）

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_FILENAME, SND_NODEFAULT, SND_SYNC};

use crate::app::environment::SoundPlayer;

/// 再生長に足す余裕（差異 3）。`PlaySound` の非同期再生はデバイスバッファぶん実際の
/// 音が遅れるため、ファイル長ちょうどで再要求すると鳴っている音を切ってしまう。
/// 実測（0.1 秒の WAV + 150ms 余裕）でプツ音が出ないことを確認済み。
const RETRIGGER_MARGIN: Duration = Duration::from_millis(150);

/// Java `Main.getSoundFilePath` L446-461 逐語: `img/<set>/sound/<file>` →
/// `sound/<set>/<file>` → `sound/<file>` の順で最初に存在する通常ファイルを返す。
///
/// `img_dir` は exe 同場所の `img` ディレクトリ（[`crate::app::assets::AssetDirs::img_dir`]）。
/// Java の `SOUND_DIRECTORY` は cwd 相対だが、本実装は exe 同場所を基点にする
/// （`img` の親 = exe ディレクトリ・design §1.10 (z-12)）。
pub fn resolve_sound_path(img_dir: &Path, image_set: &str, sound_file: &str) -> Option<PathBuf> {
    let root = img_dir.parent().unwrap_or(img_dir);
    let candidates = [
        img_dir.join(image_set).join("sound").join(sound_file),
        root.join("sound").join(image_set).join(sound_file),
        root.join("sound").join(sound_file),
    ];
    candidates.into_iter().find(|path| path.is_file())
}

/// WAV の再生長（ms）。`PlaySound` は再生状態を問い合わせられないため、差異 3 の
/// 「鳴り終わったか」判定にこの長さを使う。
///
/// RIFF/WAVE のチャンクを走査し、`fmt ` の平均バイト/秒（`nAvgBytesPerSec`）と
/// `data` のサイズから `data バイト数 * 1000 / 平均バイト毎秒` を返す。
/// RIFF/WAVE でない・チャンクが壊れている・`nAvgBytesPerSec` が 0 なら None
/// （長さ不明 = 再要求しない安全側）。
pub fn wav_duration_ms(path: &Path) -> Option<u64> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    let mut byte_rate: Option<u64> = None;
    let mut data_len: Option<u64> = None;
    while pos.checked_add(8)? <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u64::from(u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?));
        let body = pos + 8;
        match id {
            // fmt チャンク: wFormatTag(2) nChannels(2) nSamplesPerSec(4)
            // nAvgBytesPerSec(4) blockAlign(2) bitsPerSample(2)
            b"fmt " if size >= 12 && body + 12 <= bytes.len() => {
                byte_rate = Some(u64::from(u32::from_le_bytes(
                    bytes[body + 8..body + 12].try_into().ok()?,
                )));
            }
            b"data" => {
                // 宣言サイズが実ファイルより大きい（切り詰め）場合は実サイズを使う
                let available = (bytes.len() - body) as u64;
                data_len = Some(size.min(available));
                break;
            }
            _ => {}
        }
        // RIFF のチャンクは偶数境界（奇数サイズは 1 バイトのパディング）
        let advance = usize::try_from(size).ok()?;
        pos = body.checked_add(advance)?.checked_add(advance % 2)?;
    }
    let byte_rate = byte_rate.filter(|rate| *rate > 0)?;
    Some(data_len? * 1000 / byte_rate)
}

/// `PlaySoundW(SND_FILENAME | SND_ASYNC | SND_NODEFAULT)` で非同期再生する。
/// `SND_NODEFAULT` は「音が見つからないときに既定音を鳴らさない」ため（Java の
/// 無音失敗と同じ観測）。戻り値は Win32 の成否（TRUE = 成功 / FALSE = 失敗）。
fn play_wav(path: &Path) -> bool {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: `wide` はこの呼び出しの間生存する NUL 終端 UTF-16。
    // `SND_ASYNC` のため再生は非同期で、この関数は待たない。
    unsafe {
        PlaySoundW(
            PCWSTR::from_raw(wide.as_ptr()),
            None,
            SND_FILENAME | SND_ASYNC | SND_NODEFAULT,
        )
        .as_bool()
    }
}

/// 非同期再生中の音を止める。公式ドキュメントの `PlaySound(NULL, 0, 0)` 形式
/// （`SND_PURGE` は現行 Windows では **Not supported**＝無視されるため使わない）。
fn stop_current() {
    // SAFETY: 停止のみ（pszSound = NULL・フラグ 0）。
    unsafe {
        let _ = PlaySoundW(None, None, SND_SYNC);
    }
}

/// 再生中の音（差異 3 の「鳴り終わったか」判定用）。
struct Playing {
    path: PathBuf,
    started: Instant,
    /// 再生長。None = 長さ不明 → 再要求しない（安全側）。
    duration: Option<Duration>,
}

/// [`SoundPlayer`] の実 Win32 実装（`PlaySound`・#36）。
///
/// `img_dir` は起動時の [`crate::app::assets::AssetDirs::img_dir`]。
pub struct WinSoundPlayer {
    img_dir: PathBuf,
    /// 直近に再生した音（差異 3 の判定用・停止で消える）。
    playing: Option<Playing>,
}

impl WinSoundPlayer {
    pub fn new(img_dir: impl Into<PathBuf>) -> WinSoundPlayer {
        WinSoundPlayer {
            img_dir: img_dir.into(),
            playing: None,
        }
    }

    /// 再生要求の判定（`PlaySound` を呼ばない純ロジック）。鳴らすファイルを返す。None:
    /// - 音声ファイルが見つからない（Java `getSoundFilePath` の FileNotFound 相当・差異 4）
    /// - 同じ音がまだ鳴っている可能性がある（差異 3: 再生長 + [`RETRIGGER_MARGIN`] まで）
    ///
    /// `now` は呼び出し側の時刻（テストで時間経過を固定するため引数で受ける）。
    pub fn plan_play(&mut self, image_set: &str, sound: &str, now: Instant) -> Option<PathBuf> {
        let path = resolve_sound_path(&self.img_dir, image_set, sound)?;
        if let Some(playing) = &self.playing {
            if playing.path == path {
                let finished = playing.duration.is_some_and(|duration| {
                    now.saturating_duration_since(playing.started) >= duration + RETRIGGER_MARGIN
                });
                if !finished {
                    // まだ鳴っている可能性がある → 呼ばない（切ってプツッと鳴らさない）
                    return None;
                }
            }
        }
        self.playing = Some(Playing {
            path: path.clone(),
            started: now,
            duration: wav_duration_ms(&path).map(Duration::from_millis),
        });
        Some(path)
    }
}

impl SoundPlayer for WinSoundPlayer {
    /// Java `Mascot.apply` L699-707 相当。`volume` は使わない（差異 2）。
    fn play_if_idle(&mut self, image_set: &str, sound: &str, _volume: f32) {
        let Some(path) = self.plan_play(image_set, sound, Instant::now()) else {
            if resolve_sound_path(&self.img_dir, image_set, sound).is_none() {
                log::warn!("sound file not found: {sound} (image set {image_set})");
            }
            return;
        };
        if play_wav(&path) {
            log::debug!("playing sound: {}", path.display());
        } else {
            log::warn!("failed to play sound: {}", path.display());
        }
    }

    /// Java `Mute.apply` L28-52 相当。`Some` は対象が直前の音と一致するときだけ止め、
    /// `None` は無条件に全停止する（`Sounds.isEnabled()` のゲートは
    /// [`EnvironmentView::stop_sound`](crate::mascot::EnvironmentView::stop_sound) 側）。
    fn stop(&mut self, image_set: &str, sound: Option<&str>) {
        match sound {
            Some(name) => {
                let target = resolve_sound_path(&self.img_dir, image_set, name);
                let is_current = match (&self.playing, &target) {
                    (Some(playing), Some(target)) => playing.path == *target,
                    _ => false,
                };
                if is_current {
                    stop_current();
                    self.playing = None;
                }
            }
            None => {
                stop_current();
                self.playing = None;
            }
        }
    }
}
