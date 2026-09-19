//! タスク #4: src/render/imageset.rs（画像セット）の契約テスト。
//!
//! 検証対象は公開契約のみ（構造・文言・ハッシュ照合は検査しない。実資産は
//! 集計レベル＝件数・キー集合・代表 1 枚の一括 assert で検証）:
//! - PNG ヘッダ（IHDR）のみで寸法を読む（全体デコード不要。非 PNG/破損ヘッダは Err）
//! - set ロード: PNG を straight RGBA8 にデコードし、ロード時にプレマルチプライド
//!   ARGB（`Frame::argb`）へ 1 回変換して保持する。
//!   非 PNG は無言スキップ、デコード失敗 PNG は IHDR 寸法の全透明フレームで代替 + 警告、
//!   ヘッダ自体が読めない PNG はフレーム欠落 + 警告、サイズ混在は警告して続行、
//!   set ディレクトリ不在は Err（スキップ判断は呼び出し側の責務）
//! - set 単位 scale のロード時プリスケール（java_round 寸法・Lanczos3 フィルタ）。
//!   プリスケールは straight RGBA8 で行い、その後プレマルチプライド ARGB に変換する
//! - java_round（Java Math.round = floor(x+0.5)。負の半端で Rust f64::round と異なる）
//!   と scale_anchor（補正なし）/ scale_velocity（非ゼロ→0 を符号付き ±1 補正）/ scale_pose
//!   （Java AnimationBuilder.java L206-211・ImagePairs.java L81-82 の丸め規則）
//! - normalize_image_ref（先頭 '/' 除去）
//! - conf↔set 整合チェック: 欠落参照を含む Animation は丸ごと無効化（missing は正規化名）。
//!   欠落なし → disabled 空・warnings 空
//! - 実資産統合: conf/actions.xml の全 Image 参照（46 種）と img/Shimeji が集合一致 → disabled 0
//!
//! 実装 (src/render/imageset.rs) が存在する前提の GREEN 検証テスト。

use std::path::{Path, PathBuf};

use shimeji::config::{parse_actions, ActionDef, ActionsConfig, Animation, Pose, SequenceChild};
use shimeji::render::imageset::{
    available_refs, check_references, enumerate_sets, java_round, normalize_image_ref,
    read_png_size, scale_anchor, scale_pose, scale_velocity, Frame, ImageSet,
};
use shimeji::win::window::premultiply_rgba_to_argb;

// =====================================================================
// 共通ヘルパ
// =====================================================================

const OPAQUE_RED: [u8; 4] = [255, 0, 0, 255];
const OPAQUE_BLUE: [u8; 4] = [0, 0, 255, 255];
const OPAQUE_GREEN: [u8; 4] = [0, 255, 0, 255];

/// 実資産の img/ ディレクトリ。
fn assets_img() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("img")
}

/// 実資産の conf/ 下ファイル。
fn conf_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("conf")
        .join(name)
}

/// 使い捨ての合成画像用ディレクトリ（root = img_dir として使い、その下に
/// set ディレクトリ「SetA」等を作る）。Drop で再帰削除。
struct TempImg {
    root: PathBuf,
}

impl TempImg {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("shimeji_t4_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // 前回残留の掃除
        std::fs::create_dir_all(&root).expect("テンポラリ img ディレクトリを作れる");
        TempImg { root }
    }

    fn path(&self) -> &Path {
        &self.root
    }

    /// 単色 PNG を書き込む（rel は「SetA/name.png」形式）。
    fn write_png(&self, rel: &str, width: u32, height: u32, color: [u8; 4]) {
        self.save_image(
            rel,
            &image::RgbaImage::from_pixel(width, height, image::Rgba(color)),
        );
    }

    /// 任意内容の RgbaImage を書き込む（rel は「SetA/name.png」形式）。
    /// 親ディレクトリは save 前に自動作成する（image::save は親を作らない）。
    fn save_image(&self, rel: &str, image_buf: &image::RgbaImage) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("親ディレクトリを作れる");
        }
        image_buf.save(&path).expect("テンポラリ PNG を書ける");
    }

    fn write_bytes(&self, rel: &str, bytes: &[u8]) {
        let path = self.root.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("親ディレクトリを作れる");
        }
        std::fs::write(&path, bytes).expect("テンポラリファイルを書ける");
    }
}

impl Drop for TempImg {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// --- 合成 PNG（正当 IHDR + IDAT 欠落 = ヘッダは読めるが全体デコードは失敗） ---

const PNG_SIGNATURE: [u8; 8] = [0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

/// PNG チャンク用 CRC32（標準の反射多項式 0xEDB88320）。
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

fn png_chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut chunk = Vec::with_capacity(12 + payload.len());
    chunk.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    chunk.extend_from_slice(kind);
    chunk.extend_from_slice(payload);
    let mut crc_input = Vec::with_capacity(4 + payload.len());
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(payload);
    chunk.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    chunk
}

fn ihdr_chunk(width: u32, height: u32) -> Vec<u8> {
    let mut payload = Vec::with_capacity(13);
    payload.extend_from_slice(&width.to_be_bytes());
    payload.extend_from_slice(&height.to_be_bytes());
    payload.push(8); // bit depth
    payload.push(6); // color type = RGBA
    payload.push(0); // compression
    payload.push(0); // filter
    payload.push(0); // interlace
    png_chunk(b"IHDR", &payload)
}

/// シグネチャ + 正当 IHDR のみ（IDAT なし → ヘッダ読みは成功・デコードは失敗）。
fn truncated_png_bytes(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(&PNG_SIGNATURE);
    bytes.extend_from_slice(&ihdr_chunk(width, height));
    bytes
}

/// set から指定キーのフレーム寸法を取り出す（省略用ヘルパ）。
fn dims_of(set: &ImageSet, key: &str) -> (u32, u32) {
    let frame = set
        .frame(key)
        .unwrap_or_else(|| panic!("frame {key} が存在しない"));
    (frame.width, frame.height)
}

// --- 合成 actions.xml（config_parse_test.rs のパターンを踏襲） ---

const XML_HEAD: &str = "<Mascot xmlns=\"http://www.group-finity.com/Mascot\">\n<ActionList>\n";
const XML_TAIL: &str = "</ActionList>\n</Mascot>\n";

fn pose_xml(image: &str) -> String {
    format!("<Pose Image=\"{image}\" ImageAnchor=\"64,128\" Velocity=\"0,0\" Duration=\"1\"/>")
}

fn animation_xml(images: &[&str]) -> String {
    let poses: Vec<String> = images.iter().map(|img_ref| pose_xml(img_ref)).collect();
    format!("<Animation>{}</Animation>", poses.concat())
}

fn action_xml(kind: &str, class: Option<&str>, name: &str, animations: &[&[&str]]) -> String {
    let anims: Vec<String> = animations.iter().map(|a| animation_xml(a)).collect();
    let class_attr = class.map_or(String::new(), |c| format!(" Class=\"{c}\""));
    format!(
        "<Action Name=\"{name}\" Type=\"{kind}\"{class_attr}>{}</Action>",
        anims.concat()
    )
}

fn actions_xml_doc(actions: &[String]) -> String {
    format!("{}{}{}", XML_HEAD, actions.concat(), XML_TAIL)
}

fn parse_actions_xml(tag: &str, body: &str) -> ActionsConfig {
    let mut path = std::env::temp_dir();
    path.push(format!("shimeji_t4_{}_{tag}.xml", std::process::id()));
    std::fs::write(&path, body).expect("一時 actions.xml を書ける");
    let result = parse_actions(&path);
    let _ = std::fs::remove_file(&path);
    result.expect("一時 actions.xml をパースできる")
}

// --- conf 側の全 Image 参照収集（ネスト Inline 含む・実資産統合検証用） ---

fn collect_animations<'a>(def: &'a ActionDef, out: &mut Vec<&'a Animation>) {
    match def {
        ActionDef::Embedded { animations, .. }
        | ActionDef::Stay { animations, .. }
        | ActionDef::Move { animations, .. }
        | ActionDef::Animate { animations, .. } => out.extend(animations.iter()),
        ActionDef::Sequence {
            animations,
            children,
            ..
        }
        | ActionDef::Select {
            animations,
            children,
            ..
        } => {
            out.extend(animations.iter());
            for child in children {
                if let SequenceChild::Inline(inner) = child {
                    collect_animations(inner, out);
                }
            }
        }
    }
}

fn all_image_refs(cfg: &ActionsConfig) -> Vec<String> {
    let mut animations = Vec::new();
    for def in cfg.actions.values() {
        collect_animations(def, &mut animations);
    }
    let mut refs: Vec<String> = animations
        .iter()
        .flat_map(|a| a.poses.iter())
        .map(|p| normalize_image_ref(&p.image).to_string())
        .collect();
    refs.sort();
    refs.dedup();
    refs
}

// =====================================================================
// 丸め規則（Java Math.round / AnimationBuilder / ImagePairs）
// =====================================================================

#[test]
fn java_round_follows_java_math_round() {
    // Java Math.round = floor(x + 0.5)。負の半端は切り上げ。
    //（Rust f64::round は 0 から遠ざかる方向へ丸めるため -0.5 → -1 と異なる）
    let cases: &[(f64, i32)] = &[
        (0.5, 1),
        (2.5, 3),
        (-0.5, 0),  // Rust round なら -1
        (-1.5, -1), // Rust round なら -2
        (-2.5, -2),
        (0.4, 0),
        (-0.4, 0),
        (1.4, 1),
        (1.6, 2),
        (-1.4, -1),
        (-1.6, -2),
        (0.0, 0),
        (3.0, 3),
    ];
    for &(input, expected) in cases {
        assert_eq!(java_round(input), expected, "java_round({input})");
    }
}

#[test]
fn scale_anchor_rounds_without_correction() {
    // Java ImagePairs.java L81-82: (int) Math.round(anchorX * scaling) — ±1 補正はしない
    assert_eq!(scale_anchor((64, 128), 0.5), (32, 64));
    assert_eq!(scale_anchor((64, 48), 2.0), (128, 96));
    assert_eq!(
        scale_anchor((1, 1), 0.25),
        (0, 0),
        "補正なし: 非ゼロ→0 に丸まっても ±1 に直さない"
    );
    assert_eq!(
        scale_anchor((3, -1), 0.5),
        (2, 0),
        "負の半端は java_round（Rust round なら (2, -1)）"
    );
}

#[test]
fn scale_velocity_clamps_rounded_zero_to_sign() {
    // Java AnimationBuilder.java L206-211:
    //   scaledDx = (int) Math.round(dx * scaling);
    //   scaledDx = dx != 0 && scaledDx == 0 ? (dx < 0 ? -1 : 1) : scaledDx;
    assert_eq!(scale_velocity((1, 0), 0.5), (1, 0));
    assert_eq!(
        scale_velocity((1, 0), 0.25),
        (1, 0),
        "0 に丸まった非ゼロ → +1 補正"
    );
    assert_eq!(
        scale_velocity((-3, 0), 0.5),
        (-1, 0),
        "java_round(-1.5) = -1（Rust round なら -2）"
    );
    assert_eq!(
        scale_velocity((-1, 0), 0.4),
        (-1, 0),
        "0 に丸まった非ゼロ → -1 補正"
    );
    assert_eq!(scale_velocity((0, -1), 0.4), (0, -1), "dy 側の補正");
    assert_eq!(scale_velocity((0, 0), 0.5), (0, 0), "ゼロは補正しない");
    assert_eq!(scale_velocity((2, 4), 0.5), (1, 2));
}

#[test]
fn scale_pose_scales_anchor_and_velocity_keeps_image_and_duration() {
    let pose = Pose {
        image: "/shime1.png".to_string(),
        anchor: (64, 128),
        velocity: (-3, 0),
        duration: 6,
        sound: Some("se.wav".to_string()),
        volume: 0.5,
    };
    let scaled = scale_pose(&pose, 0.5);
    assert_eq!(scaled.image, "/shime1.png", "image は不変");
    assert_eq!(scaled.duration, 6, "duration は不変");
    assert_eq!(scaled.anchor, (32, 64), "anchor は java_round（補正なし）");
    assert_eq!(
        scaled.velocity,
        (-1, 0),
        "velocity は java_round（Rust round なら -2）"
    );
    assert_eq!(
        scaled.sound.as_deref(),
        Some("se.wav"),
        "sound は不変（XML Sound 属性）"
    );
    assert_eq!(scaled.volume, 0.5, "volume は不変（XML Volume 属性）");

    // 0 に丸まる非ゼロ速度は ±1 補正される
    let walk = Pose {
        image: "/shime2.png".to_string(),
        anchor: (64, 112),
        velocity: (1, 0),
        duration: 3,
        sound: None,
        volume: 0.0,
    };
    let tiny_walk = scale_pose(&walk, 0.25);
    assert_eq!(tiny_walk.velocity, (1, 0), "java_round(0.25)=0 → +1 補正");
    assert_eq!(tiny_walk.anchor, (16, 28));
    assert_eq!(tiny_walk.duration, 3, "duration は不変");
}

#[test]
fn normalize_image_ref_removes_leading_slash() {
    assert_eq!(normalize_image_ref("/shime1.png"), "shime1.png");
    assert_eq!(normalize_image_ref("shime1.png"), "shime1.png");
}

// =====================================================================
// PNG ヘッダ読み（IHDR のみで寸法）
// =====================================================================

#[test]
fn read_png_size_reads_ihdr_only_dimensions() {
    // 実資産の代表 1 枚: 128×128
    let real = assets_img().join("Shimeji").join("shime1.png");
    assert_eq!(
        read_png_size(&real).expect("shime1.png のヘッダ"),
        (128, 128)
    );

    let img = TempImg::new("hdr");
    img.write_png("SetA/small.png", 8, 6, OPAQUE_RED);
    assert_eq!(
        read_png_size(&img.path().join("SetA/small.png")).expect("small.png のヘッダ"),
        (8, 6)
    );

    // IDAT 等がなく IHDR のみでも寸法は読める（全体デコード不要の証明）
    img.write_bytes("SetA/truncated.png", &truncated_png_bytes(16, 9));
    assert_eq!(
        read_png_size(&img.path().join("SetA/truncated.png")).expect("IHDR のみで読める"),
        (16, 9)
    );
}

#[test]
fn read_png_size_rejects_non_png_and_corrupt_headers() {
    // 非 PNG（実資産の banner.bmp）
    let bmp = assets_img().join("Shimeji").join("banner.bmp");
    assert!(read_png_size(&bmp).is_err(), "BMP は Err");

    let img = TempImg::new("hdr_bad");
    img.write_bytes("SetA/garbage.png", b"not a png at all");
    img.write_bytes("SetA/empty.png", b"");
    let mut corrupt = Vec::new();
    corrupt.extend_from_slice(&PNG_SIGNATURE);
    corrupt.extend_from_slice(b"\x00\x01");
    img.write_bytes("SetA/corrupt.png", &corrupt);
    assert!(
        read_png_size(&img.path().join("SetA/garbage.png")).is_err(),
        "テキストは Err"
    );
    assert!(
        read_png_size(&img.path().join("SetA/empty.png")).is_err(),
        "空ファイルは Err"
    );
    assert!(
        read_png_size(&img.path().join("SetA/corrupt.png")).is_err(),
        "シグネチャ後即破損（IHDR なし）は Err"
    );
}

// =====================================================================
// set 列挙・参照一覧
// =====================================================================

#[test]
fn enumerate_sets_lists_subdirectories_only() {
    let img = TempImg::new("enum");
    std::fs::create_dir_all(img.path().join("Alpha")).expect("set ディレクトリ");
    std::fs::create_dir_all(img.path().join("Beta")).expect("set ディレクトリ");
    img.write_bytes("not_a_set.txt", b"file is not a set");
    let mut sets = enumerate_sets(img.path()).expect("列挙できる");
    sets.sort();
    assert_eq!(
        sets,
        vec!["Alpha".to_string(), "Beta".to_string()],
        "ファイルは列挙しない"
    );
}

#[test]
fn enumerate_sets_on_real_img_dir_finds_both_sets() {
    let mut sets = enumerate_sets(&assets_img()).expect("実資産 img/ を列挙できる");
    sets.sort();
    assert_eq!(sets, vec!["KuroShimeji".to_string(), "Shimeji".to_string()]);
}

#[test]
fn enumeration_on_missing_directories_is_err() {
    let missing =
        std::env::temp_dir().join(format!("shimeji_t4_no_such_dir_{}", std::process::id()));
    assert!(enumerate_sets(&missing).is_err(), "img_dir 不在 → Err");
    assert!(
        available_refs(&missing, "Shimeji").is_err(),
        "set ディレクトリ不在 → Err"
    );
}

#[test]
fn available_refs_real_set_has_46_normalized_png_names() {
    let refs = available_refs(&assets_img(), "Shimeji").expect("実資産 Shimeji の参照一覧");
    assert_eq!(refs.len(), 46, "PNG 46 枚（banner.bmp を含まない）");
    assert!(refs.iter().all(|r| r.ends_with(".png")), "全要素が .png");
    assert!(
        refs.iter().all(|r| !r.starts_with('/')),
        "正規化済み（先頭 / なし）"
    );
    assert!(refs.contains(&"shime1.png".to_string()));
    assert!(refs.contains(&"shime46.png".to_string()));
    assert!(
        refs.iter().all(|r| !r.contains("banner")),
        "banner.bmp は含まない"
    );
}

#[test]
fn available_refs_lists_top_level_pngs_only() {
    let img = TempImg::new("avail");
    img.write_png("SetA/top.png", 4, 4, OPAQUE_RED);
    img.write_png("SetA/nested/inner.png", 4, 4, OPAQUE_BLUE);
    img.write_bytes("SetA/banner.bmp", b"BM");
    let mut refs = available_refs(img.path(), "SetA").expect("参照一覧");
    refs.sort();
    assert_eq!(
        refs,
        vec!["top.png".to_string()],
        "トップレベル .png のみ・正規化名"
    );
}

// =====================================================================
// ImageSet::load（実資産・集計レベル検証）
// =====================================================================

#[test]
fn load_real_shimeji_set_has_46_frames() {
    let set =
        ImageSet::load(&assets_img(), "Shimeji", None).expect("実資産 Shimeji をロードできる");
    assert_eq!(set.name, "Shimeji");
    assert_eq!(set.frames.len(), 46, "PNG 46 枚。banner.bmp は無言スキップ");
    assert!(
        set.warnings.is_empty(),
        "実資産は警告なし: {:?}",
        set.warnings
    );

    // キー集合 == available_refs（banner.bmp を含まない・正規化名）
    let mut frame_keys: Vec<String> = set.frames.keys().cloned().collect();
    frame_keys.sort();
    let mut sorted_available = available_refs(&assets_img(), "Shimeji").expect("参照一覧");
    sorted_available.sort();
    assert_eq!(
        frame_keys, sorted_available,
        "frames のキー集合 == available_refs"
    );

    // 全 frames が 128×128 のプレマルチプライド ARGB（一括 assert 一発）
    for frame in set.frames.values() {
        assert_eq!(
            (frame.width, frame.height, frame.argb.len()),
            (128, 128, (128 * 128) as usize),
            "全 frames は 128×128 プレマルチプライド ARGB"
        );
    }
}

#[test]
fn load_real_frame_lookup_normalizes_leading_slash() {
    let set = ImageSet::load(&assets_img(), "Shimeji", None).expect("ロードできる");
    // 代表 1 枚（shime1.png）で先頭 '/' あり/なしが同じフレームを引けること
    let plain = set.frame("shime1.png").expect("shime1.png が引ける");
    let slashed = set.frame("/shime1.png").expect("/shime1.png が引ける");
    assert_eq!((plain.width, plain.height), (128, 128));
    assert_eq!((slashed.width, slashed.height), (128, 128));
    assert_eq!(plain.argb, slashed.argb, "両ルックアップは同一フレーム");
    assert!(set.frame("no_such.png").is_none());
    assert!(set.frame("/no_such.png").is_none());
}

#[test]
fn load_real_set_with_half_scale_prescales_frames() {
    let set = ImageSet::load(&assets_img(), "Shimeji", Some(0.5)).expect("scale 付きロードできる");
    assert_eq!(set.frames.len(), 46);
    assert!(
        set.warnings.is_empty(),
        "プリスケールで警告は出ない: {:?}",
        set.warnings
    );
    // 一括 assert 一発: 全 frames が java_round(128*0.5)=64 の正方形
    for frame in set.frames.values() {
        assert_eq!(
            (frame.width, frame.height, frame.argb.len()),
            (64, 64, (64 * 64) as usize),
            "128×0.5 → 64×64 にプリスケールされる"
        );
    }
}

// =====================================================================
// ImageSet::load（合成データ: RGBA / scale / 破損 / 混在 / 不在）
// =====================================================================

#[test]
fn load_premultiplies_straight_rgba_to_argb() {
    // 契約: ロード時に straight RGBA8 がプレマルチプライド ARGB へ 1 回変換される。
    // 半透明 [10,20,30,128] は切り捨て (v*a)/255 で [5,10,15,128] になる。
    let img = TempImg::new("rgba");
    let mut pixels = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0]));
    pixels.put_pixel(0, 0, image::Rgba([10, 20, 30, 128]));
    pixels.put_pixel(0, 1, image::Rgba([255, 255, 255, 255]));
    img.save_image("SetA/alpha.png", &pixels);

    let set = ImageSet::load(img.path(), "SetA", None).expect("ロードできる");
    let frame = set.frame("alpha.png").expect("alpha.png が引ける");
    assert_eq!((frame.width, frame.height), (2, 2));
    assert_eq!(frame.argb.len(), 4);

    // 行優先の straight RGBA（自己参照回避のため焼き込み値も併記する）
    let straight: Vec<u8> = vec![
        10, 20, 30, 128, // 半透明 → 0x80050A0F
        0, 0, 0, 0, // 透明 → 0
        255, 255, 255, 255, // 不透明 → 0xFFFFFFFF
        0, 0, 0, 0, // 透明 → 0
    ];
    assert_eq!(frame.argb, premultiply_rgba_to_argb(&straight));
    assert_eq!(
        frame.argb,
        vec![0x8005_0A0F, 0x0000_0000, 0xFFFF_FFFF, 0x0000_0000],
        "半透明は切り捨てプレマルチプライ・不透明/透明はそのまま"
    );
}

#[test]
fn load_scale_follows_java_round_dimensions() {
    let img = TempImg::new("scale_dim");
    img.write_png("SetA/four.png", 4, 4, OPAQUE_RED);
    img.write_png("SetA/three.png", 3, 3, OPAQUE_BLUE);
    img.write_png("SetA/solid.png", 4, 4, OPAQUE_GREEN);

    let none = ImageSet::load(img.path(), "SetA", None).expect("None ロード");
    assert_eq!(
        dims_of(&none, "four.png"),
        (4, 4),
        "None はリサンプルしない"
    );
    assert_eq!(dims_of(&none, "three.png"), (3, 3));

    let one = ImageSet::load(img.path(), "SetA", Some(1.0)).expect("Some(1.0) ロード");
    assert_eq!(dims_of(&one, "four.png"), (4, 4), "1.0 はリサンプルしない");
    assert_eq!(dims_of(&one, "three.png"), (3, 3));

    let half = ImageSet::load(img.path(), "SetA", Some(0.5)).expect("Some(0.5) ロード");
    assert_eq!(dims_of(&half, "four.png"), (2, 2), "java_round(2.0)=2");
    assert_eq!(dims_of(&half, "three.png"), (2, 2), "java_round(1.5)=2");
    assert_eq!(dims_of(&half, "solid.png"), (2, 2));
    // 単色 OPAQUE_GREEN [0,255,0,255] はプレマルチプライ後 0xFF00FF00。
    let solid_argb = premultiply_rgba_to_argb(&OPAQUE_GREEN);
    assert_eq!(
        solid_argb,
        vec![0xFF00_FF00],
        "不透明緑のプレマルチプライ値"
    );
    for &px in &half.frame("solid.png").expect("solid.png").argb {
        assert_eq!(px, solid_argb[0], "縮小後も単色内容を保持");
    }

    let double = ImageSet::load(img.path(), "SetA", Some(2.0)).expect("Some(2.0) ロード");
    assert_eq!(dims_of(&double, "four.png"), (8, 8), "4×2.0 → 8");
    assert_eq!(dims_of(&double, "three.png"), (6, 6), "3×2.0 → 6");
}

#[test]
fn load_retains_resolved_scale_while_preserving_frame_prescale() {
    // 契約: ImageSet::load は解決済み scale（None → 1.0 / Some(s) → s）を保持する。
    // ポーズの ImageAnchor / velocity は呼び出し側（action 構築 / Mascot）がこの
    // scale で変換するため、未スケール anchor が混入すると浮き・ズレになる。
    // 画像フレームのプリスケールは従来どおり行われる（回帰確認込み）。
    let img = TempImg::new("retain_scale");
    img.write_png("SetA/four.png", 4, 4, OPAQUE_RED);

    let none = ImageSet::load(img.path(), "SetA", None).expect("None ロード");
    assert_eq!(none.scale, 1.0, "None は 1.0 に解決して保持");

    let one = ImageSet::load(img.path(), "SetA", Some(1.0)).expect("Some(1.0) ロード");
    assert_eq!(one.scale, 1.0, "Some(1.0) は 1.0 を保持");

    let half = ImageSet::load(img.path(), "SetA", Some(0.5)).expect("Some(0.5) ロード");
    assert_eq!(half.scale, 0.5, "Some(0.5) の解決値を保持");
    assert_eq!(
        dims_of(&half, "four.png"),
        (2, 2),
        "画像フレームのプリスケールは既存挙動のまま"
    );
}

/// 画像を左半 / 右半に分けた (R合計, G合計, B合計) を返す（配置の集計検証用）。
/// 1px 単位の絶対値ではなく空間的な優勢色をロバストに比較するために使う。
/// 入力はプレマルチプライド ARGB（不透明前提のため α は考慮しない）。
fn half_rgb_totals(argb: &[u32], width: u32, height: u32, left_half: bool) -> (u64, u64, u64) {
    let (mut r, mut g, mut b) = (0u64, 0u64, 0u64);
    for y in 0..height {
        for x in 0..width {
            if (x < width / 2) != left_half {
                continue;
            }
            let px = argb[(y * width + x) as usize];
            r += u64::from((px >> 16) & 0xFF);
            g += u64::from((px >> 8) & 0xFF);
            b += u64::from(px & 0xFF);
        }
    }
    (r, g, b)
}

#[test]
fn load_scale_uses_lanczos3_interpolation() {
    // 契約: set 単位 scale のロード時プリスケールは Lanczos3 で行われる
    //（プリスケールは straight RGBA8 で行い、その後プレマルチプライド ARGB へ変換）。
    // Lanczos3 は補間するため「赤でも青でもない中間色（不透明）」が生成される。
    // Nearest なら中間色は一切生成されない = この断言が RED の根拠。
    let img = TempImg::new("scale_lanczos3");
    let mut two = image::RgbaImage::from_pixel(2, 2, image::Rgba(OPAQUE_RED));
    two.put_pixel(1, 0, image::Rgba(OPAQUE_BLUE));
    two.put_pixel(1, 1, image::Rgba(OPAQUE_BLUE));
    img.save_image("SetA/two.png", &two);

    let set = ImageSet::load(img.path(), "SetA", Some(2.0)).expect("Some(2.0) ロード");
    let frame = set.frame("two.png").expect("two.png が引ける");
    assert_eq!((frame.width, frame.height), (4, 4), "java_round 寸法");

    // 中間色の存在 + 元が全て不透明なので出力も全ピクセル不透明
    let mut intermediate = 0usize;
    for &px in &frame.argb {
        assert_eq!(
            (px >> 24) & 0xFF,
            255,
            "元が全て不透明なら出力も不透明: {px:#010X}"
        );
        if px != 0xFFFF_0000 && px != 0xFF00_00FF {
            intermediate += 1;
        }
    }
    assert!(
        intermediate > 0,
        "Lanczos3 補間による中間色ピクセルが存在するはず（Nearest なら 0）"
    );

    // 空間配置（ロバストな集計）: 左半は赤優勢・右半は青優勢
    let (lr, _, lb) = half_rgb_totals(&frame.argb, frame.width, frame.height, true);
    let (rr, _, rb) = half_rgb_totals(&frame.argb, frame.width, frame.height, false);
    assert!(lr > lb, "左半は赤優勢のはず（R合計={lr} B合計={lb}）");
    assert!(rb > rr, "右半は青優勢のはず（R合計={rr} B合計={rb}）");

    // 決定論: 同一入力・同一 scale の再ロードは同一バイト列
    let reloaded = ImageSet::load(img.path(), "SetA", Some(2.0)).expect("再ロード");
    assert_eq!(
        reloaded.frame("two.png").expect("two.png").argb,
        frame.argb,
        "同一入力に対し決定的"
    );
}

#[test]
fn load_corrupt_png_with_readable_ihdr_substitutes_transparent_frame() {
    let img = TempImg::new("corrupt_ihdr");
    img.write_bytes("SetA/broken.png", &truncated_png_bytes(8, 6));
    let set = ImageSet::load(img.path(), "SetA", None)
        .expect("デコード失敗 PNG は代替され set 全体は Ok");
    assert_eq!(set.frames.len(), 1, "代替フレームが登録される");
    let broken = set
        .frame("broken.png")
        .expect("broken.png が代替フレームで引ける");
    assert_eq!(
        (broken.width, broken.height),
        (8, 6),
        "代替寸法は IHDR から取る"
    );
    assert_eq!(broken.argb.len(), (8 * 6) as usize);
    assert!(broken.argb.iter().all(|&p| p == 0), "代替フレームは全透明");
    assert_eq!(set.warnings.len(), 1, "代替で警告 1 件: {:?}", set.warnings);
}

#[test]
fn load_corrupt_png_with_unreadable_header_is_omitted_with_warning() {
    let img = TempImg::new("corrupt_junk");
    img.write_png("SetA/good.png", 4, 4, OPAQUE_RED);
    img.write_bytes("SetA/junk.png", b"\x00\x01\x02 junk bytes");
    let set = ImageSet::load(img.path(), "SetA", None)
        .expect("ヘッダ破損 PNG はフレーム欠落扱いで set 全体は Ok");
    assert_eq!(
        set.frames.len(),
        1,
        "junk.png は欠落・good.png だけが登録される"
    );
    assert!(set.frame("junk.png").is_none());
    assert!(
        !set.warnings.is_empty(),
        "ヘッダ破損は警告になる: {:?}",
        set.warnings
    );
    assert_eq!(dims_of(&set, "good.png"), (4, 4), "他のファイルは無傷");
}

#[test]
fn load_mixed_frame_sizes_continues_with_warning() {
    let img = TempImg::new("mixed");
    img.write_png("SetA/small.png", 4, 4, OPAQUE_RED);
    img.write_png("SetA/wide.png", 8, 6, OPAQUE_BLUE);
    let set = ImageSet::load(img.path(), "SetA", None).expect("サイズ混在でもロード続行");
    assert_eq!(set.frames.len(), 2, "両フレームとも保持される");
    assert_eq!(
        dims_of(&set, "small.png"),
        (4, 4),
        "各フレームは自分の寸法を保持"
    );
    assert_eq!(dims_of(&set, "wide.png"), (8, 6));
    assert!(
        !set.warnings.is_empty(),
        "サイズ混在は警告になる: {:?}",
        set.warnings
    );
}

#[test]
fn load_missing_set_directory_is_err() {
    assert!(
        ImageSet::load(&assets_img(), "NoSuchSet__", None).is_err(),
        "set ディレクトリ不在は Err（スキップ判断は呼び出し側の責務）"
    );
}

// =====================================================================
// conf↔set 整合チェック（check_references）
// =====================================================================

#[test]
fn check_references_disables_whole_animation_on_missing_ref() {
    // 第 1 アニメは在庫あり、第 2 アニメは 2 ポーズ中 1 参照が欠落。
    // 1 個でも欠落があればアニメ丸ごと無効化され、欠落のない参照名は missing に入らない。
    let cfg = parse_actions_xml(
        "consistency_basic",
        &actions_xml_doc(&[action_xml(
            "Stay",
            None,
            "A",
            &[&["/shime1.png"], &["/shime2.png", "/shime9.png"]],
        )]),
    );
    let available = vec!["shime1.png".to_string(), "shime2.png".to_string()];
    let report = check_references(&cfg, &available);

    assert_eq!(report.disabled.len(), 1, "無効化は 1 アニメのみ");
    let disabled = &report.disabled[0];
    assert_eq!(disabled.action, "A");
    assert_eq!(disabled.animation_index, 1, "第 2 アニメ（0 始まり）");
    assert_eq!(
        disabled.missing,
        vec!["shime9.png"],
        "missing は正規化済み名のみ"
    );
}

#[test]
fn check_references_covers_action_variants_with_normalized_refs() {
    // "shime1.png"（先頭 / なし）も normalize 後は一致する。Embedded / Move も検査対象。
    let cfg = parse_actions_xml(
        "consistency_variants",
        &actions_xml_doc(&[
            action_xml("Stay", None, "S", &[&["shime1.png"]]),
            action_xml(
                "Embedded",
                Some("com.group_finity.mascot.action.Breed"),
                "E",
                &[&["/missing_a.png"]],
            ),
            action_xml("Move", None, "M", &[&["/missing_b.png"]]),
        ]),
    );
    let available = vec!["shime1.png".to_string()];
    let report = check_references(&cfg, &available);

    assert_eq!(
        report.disabled.len(),
        2,
        "Embedded/Move の欠落アニメが無効化"
    );
    let mut actions: Vec<String> = report.disabled.iter().map(|d| d.action.clone()).collect();
    actions.sort();
    assert_eq!(actions, vec!["E".to_string(), "M".to_string()]);
    let mut missing: Vec<String> = report
        .disabled
        .iter()
        .flat_map(|d| d.missing.iter().cloned())
        .collect();
    missing.sort();
    assert_eq!(
        missing,
        vec!["missing_a.png".to_string(), "missing_b.png".to_string()],
        "missing は正規化済み名"
    );
}

#[test]
fn check_references_clean_when_all_refs_available() {
    let cfg = parse_actions_xml(
        "consistency_clean",
        &actions_xml_doc(&[
            action_xml(
                "Stay",
                None,
                "S1",
                &[&["/shime1.png", "/shime2.png"], &["shime2.png"]],
            ),
            action_xml("Move", None, "M1", &[&["/shime46.png"]]),
        ]),
    );
    // 未使用画像が available に混ざっていても欠落ではない
    let available = vec![
        "shime1.png".to_string(),
        "shime2.png".to_string(),
        "shime46.png".to_string(),
        "unused.png".to_string(),
    ];
    let report = check_references(&cfg, &available);
    assert!(report.disabled.is_empty(), "欠落なし → disabled 空");
    assert!(report.warnings.is_empty(), "欠落なし → warnings 空");
}

// =====================================================================
// 実資産統合（conf/actions.xml × img/Shimeji）
// =====================================================================

#[test]
fn real_actions_xml_and_shimeji_set_are_fully_consistent() {
    // asset-report §2: conf 側 Image 参照は 46 種（/shime1.png〜/shime46.png）で欠落なし
    let cfg = parse_actions(&conf_path("actions.xml")).expect("実資産 actions.xml");
    let available = available_refs(&assets_img(), "Shimeji").expect("実資産 Shimeji の一覧");

    // conf 側の全 Image 参照（ネスト含む）を正規化・重複排除 → 46 種
    let refs = all_image_refs(&cfg);
    assert_eq!(refs.len(), 46, "参照は 46 種に集約される");

    // 参照集合 == set の PNG 集合（集合比較）
    let mut sorted_available = available.clone();
    sorted_available.sort();
    assert_eq!(refs, sorted_available, "conf の参照集合 == set の PNG 集合");

    // 整合チェック: 欠落なし → disabled 0・warnings 空
    let report = check_references(&cfg, &available);
    assert!(
        report.disabled.is_empty(),
        "無効化 0 件（実測 {} 件）",
        report.disabled.len()
    );
    assert!(report.warnings.is_empty(), "警告 0 件");
}

// =====================================================================
// タスク #20: PNG 寸法ガード + scale 検証（異常入力でプロセスを死なせない）
// =====================================================================

/// 契約: 0 幅 / 0 高の IHDR のみ PNG（デコード不能・ヘッダ代替経路）は
/// panic せずフレーム欠落 + 警告になる。
#[test]
fn load_zero_dimension_png_is_omitted_with_warning() {
    let img = TempImg::new("zero_dim");
    img.write_bytes("SetA/zero_both.png", &truncated_png_bytes(0, 0));
    img.write_bytes("SetA/zero_width.png", &truncated_png_bytes(0, 4));
    img.write_bytes("SetA/zero_height.png", &truncated_png_bytes(4, 0));

    let set = ImageSet::load(img.path(), "SetA", None).expect("0 寸法 PNG でも set 全体は Ok");
    assert!(
        set.frame("zero_both.png").is_none(),
        "0x0 フレームは欠落する"
    );
    assert!(
        set.frame("zero_width.png").is_none(),
        "width=0 フレームは欠落する"
    );
    assert!(
        set.frame("zero_height.png").is_none(),
        "height=0 フレームは欠落する"
    );
    assert!(
        !set.warnings.is_empty(),
        "寸法不正は警告になる: {:?}",
        set.warnings
    );
}

/// 契約: 上限超過の巨大寸法 IHDR は panic（u32 乗算オーバーフロー / 巨大 alloc）
/// せずフレーム欠落 + 警告になる。
#[test]
fn load_oversized_png_is_omitted_with_warning() {
    let img = TempImg::new("oversized");
    img.write_bytes("SetA/huge.png", &truncated_png_bytes(100_000, 100_000));

    let set = ImageSet::load(img.path(), "SetA", None).expect("巨大寸法 PNG でも set 全体は Ok");
    assert!(
        set.frame("huge.png").is_none(),
        "上限超過フレームは欠落する"
    );
    assert!(
        !set.warnings.is_empty(),
        "上限超過は警告になる: {:?}",
        set.warnings
    );
}

/// 契約: 小さな実 PNG（16x16）+ 有効寸法上限を超える過大 scale は resize せず
/// panic せずフレーム欠落 + 警告になる。
///
/// RED 安全性: f64→u32 飽和域（1e9 等）は現行実装で `resize` が巨大 alloc を
/// 試みて abort（プロセス全体を殺す）ため使わない。ここでは 16x16 × scale 129
/// （= 2064² > 2048² のフレーム上限）で「上限超過」を作り、現行 RED でも
/// 約 17MB の小さい alloc に留める。
#[test]
fn load_oversized_scale_skips_frame_without_panic() {
    let img = TempImg::new("huge_scale");
    img.write_png("SetA/small.png", 16, 16, OPAQUE_RED);

    let set =
        ImageSet::load(img.path(), "SetA", Some(129.0)).expect("過大 scale でも set 全体は Ok");
    assert!(
        set.frame("small.png").is_none(),
        "上限超過 scale のフレームは欠落する"
    );
    assert!(
        !set.warnings.is_empty(),
        "過大 scale は警告になる: {:?}",
        set.warnings
    );
}

/// 契約（寸法ガードの最終防衛）: width=0 / 空 RGBA の Frame は panic せず
/// 空の argb を持つ（chunks_exact(0) 相当の panic 回避）。
#[test]
fn from_rgba_zero_width_frame_yields_empty_argb_without_panic() {
    let frame = Frame::from_rgba(0, 6, vec![0u8; 6 * 4]);
    assert!(frame.argb.is_empty(), "width=0 → argb 空");
    let frame = Frame::from_rgba(4, 4, Vec::new());
    assert!(frame.argb.is_empty(), "rgba 空 → argb 空");
}

/// 回帰: 通常寸法 PNG + 中程度 scale（2.0）は警告なしで正しくプリスケールされる
/// （寸法ガードが正常系を誤って弾かないことの確認）。
#[test]
fn load_normal_png_with_moderate_scale_stays_warning_free() {
    let img = TempImg::new("scale_regression");
    img.write_png("SetA/img.png", 16, 16, OPAQUE_RED);

    let set = ImageSet::load(img.path(), "SetA", Some(2.0)).expect("通常スケールは読める");
    assert_eq!(dims_of(&set, "img.png"), (32, 32));
    assert!(
        set.warnings.is_empty(),
        "通常系は警告なし: {:?}",
        set.warnings
    );
}
