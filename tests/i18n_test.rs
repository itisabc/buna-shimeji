//! T2: i18n モジュール + 辞書整合 + フォールバック契約テスト（RED → GREEN）。
//!
//! 設計正本: `docs/plans/done/i18n-design.md` §1（API）/ §2（settings）/ §3（キー）/
//! §6.2（本テストが満たす契約）/ §8（エッジケース）。
//! Java キー対応: `docs/plans/done/localization-table.md` §1・§3・§4。
//!
//! coder が実装する公開 API（本テストが固定する契約）:
//!
//! ```text
//! // ---- src/i18n.rs ----
//! pub enum UiKey { /* 設計 §3-A の 20 バリアント */ }   // Copy
//! impl UiKey {
//!     pub const ALL: &'static [UiKey];
//!     pub fn key(self) -> &'static str;               // 例 "CallShimeji"
//! }
//! pub struct Lang;
//! impl Lang {
//!     pub fn load(lang_dir: &Path, code: &str) -> Lang;
//!     pub fn text(&self, key: UiKey) -> &str;
//!     pub fn behavior_text<'a>(&'a self, name: &'a str) -> &'a str;
//! }
//! pub const DEFAULT_LANGUAGE: &str = "en";
//!
//! // ---- src/tray.rs ----
//! pub struct GeneralSettings {
//!     pub show_console: bool,
//!     pub language: String,   // 既定 "en"（#[serde(default)]）
//! }
//! ```
//!
//! 出荷辞書（フラット `key = "値"` TOML）:
//! - `assets/lang/en.toml`（バイナリ埋め込み英語ベース）
//! - `conf/lang/ja.toml`（外部日本語辞書）
//! - `conf/lang/en.toml`（外部英語辞書）
//!
//! テストはフォールバックの**戻り値**のみを検証する（ログ文言は検査しない）。

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use shimeji::i18n::{Lang, UiKey, DEFAULT_LANGUAGE};
use shimeji::tray::{GeneralSettings, Settings};

/// 設計 §3-A の UI キー数。
const UI_KEY_COUNT: usize = 21;
/// 設計 §6.2-2 の behaviors.xml の Behavior 数。
const BEHAVIOR_NAME_COUNT: usize = 57;

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn en_dict_path() -> PathBuf {
    manifest_dir().join("assets").join("lang").join("en.toml")
}

fn shipped_lang_dir() -> PathBuf {
    manifest_dir().join("conf").join("lang")
}

fn ja_dict_path() -> PathBuf {
    shipped_lang_dir().join("ja.toml")
}

/// フラット TOML 辞書を読む（読めない/パース不能はテスト失敗）。
fn read_dict(path: &Path) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{} を読める: {e}", path.display()));
    toml::from_str(&text)
        .unwrap_or_else(|e| panic!("{} をフラット TOML としてパースできる: {e}", path.display()))
}

/// conf/behaviors.xml の全 `<Behavior Name="...">` を flatten して列挙する
///（`<Condition>`/`<BehaviorList>` の入れ子を問わず全 descendants。Name 重複は除去）。
fn behavior_names() -> Vec<String> {
    let xml = std::fs::read_to_string(manifest_dir().join("conf").join("behaviors.xml"))
        .expect("conf/behaviors.xml を読める");
    let doc = roxmltree::Document::parse(&xml).expect("conf/behaviors.xml をパースできる");
    let mut names: Vec<String> = doc
        .descendants()
        .filter(|n| n.is_element() && n.tag_name().name() == "Behavior")
        .filter_map(|n| n.attribute("Name").map(str::to_string))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// 使い捨て辞書ディレクトリ（Drop で再帰削除）。
struct TempLangDir {
    root: PathBuf,
}

impl TempLangDir {
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("shimeji_i18n_{}_{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root); // 前回残留の掃除
        std::fs::create_dir_all(&root).expect("temp 辞書ディレクトリを作れる");
        TempLangDir { root }
    }

    fn dir(&self) -> &Path {
        &self.root
    }

    fn write_dict(&self, code: &str, content: &str) {
        std::fs::write(self.root.join(format!("{code}.toml")), content).expect("temp 辞書を書ける");
    }
}

impl Drop for TempLangDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

// =====================================================================
// 契約 1: UiKey::ALL / key() の型付き正本
// =====================================================================

/// `UiKey::ALL` は設計 §3-A の 20 キー。`key()` は非空の識別子文字列を返し重複しない。
#[test]
fn ui_key_all_is_unique_and_key_strings_are_identifiers() {
    let all = UiKey::ALL;
    assert_eq!(all.len(), UI_KEY_COUNT, "設計 §3-A の UI キー数");

    let mut seen: HashSet<String> = HashSet::new();
    for &key in all {
        let k = key.key();
        assert!(!k.is_empty(), "key() は非空");
        assert!(
            k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "key() は辞書識別子: {k}"
        );
        assert!(seen.insert(k.to_string()), "key() が重複している: {k}");
    }
    assert_eq!(seen.len(), all.len(), "全 key() が一意");
}

/// 主要キーの `key()` は Java language.properties のキー名そのもの（localization-table §1）。
#[test]
fn ui_key_key_maps_to_java_language_properties_names() {
    assert_eq!(UiKey::CallShimeji.key(), "CallShimeji");
    assert_eq!(UiKey::FollowCursor.key(), "FollowCursor");
    assert_eq!(UiKey::BreedingTransient.key(), "BreedingTransient");
    assert_eq!(UiKey::SetBehaviour.key(), "SetBehaviour");
    assert_eq!(UiKey::Shimeji.key(), "Shimeji");
}

// =====================================================================
// 契約 2: 辞書網羅（en.toml / ja.toml）
// =====================================================================

/// `UiKey::ALL` の全キーが en.toml と ja.toml の両方に非空値で存在する。
#[test]
fn every_ui_key_has_non_empty_translation_in_en_and_ja() {
    let en = read_dict(&en_dict_path());
    let ja = read_dict(&ja_dict_path());
    for &key in UiKey::ALL {
        let k = key.key();
        assert!(
            en.get(k).is_some_and(|v| !v.trim().is_empty()),
            "assets/lang/en.toml に非空値が無い: {k}"
        );
        assert!(
            ja.get(k).is_some_and(|v| !v.trim().is_empty()),
            "conf/lang/ja.toml に非空値が無い: {k}"
        );
    }
}

/// behaviors.xml に現れる全 Behavior 名が en.toml と ja.toml の両方に非空値で存在する
///（Java にキーの無い 4 件の新規訳を含む網羅の回帰防止）。
#[test]
fn every_behavior_name_has_non_empty_translation_in_en_and_ja() {
    let names = behavior_names();
    assert_eq!(
        names.len(),
        BEHAVIOR_NAME_COUNT,
        "behaviors.xml の Behavior 名数（flatten 後の一意数）"
    );

    let en = read_dict(&en_dict_path());
    let ja = read_dict(&ja_dict_path());
    for name in &names {
        assert!(
            en.get(name).is_some_and(|v| !v.trim().is_empty()),
            "assets/lang/en.toml に Behavior 訳が無い: {name}"
        );
        assert!(
            ja.get(name).is_some_and(|v| !v.trim().is_empty()),
            "conf/lang/ja.toml に Behavior 訳が無い: {name}"
        );
    }
}

// =====================================================================
// 契約 3: 解決優先順位（外部辞書 → 埋め込み英語 → キー文字列）
// =====================================================================

/// 出荷 ja.toml を load すると、UI キー・Behavior 名とも外部辞書の値が返る
///（外部辞書が埋め込み英語より優先される）。
#[test]
fn shipped_ja_dictionary_values_take_priority() {
    let ja = read_dict(&ja_dict_path());
    let lang = Lang::load(&shipped_lang_dir(), "ja");

    for &key in UiKey::ALL {
        let k = key.key();
        let expected = ja.get(k).expect("ja.toml に UI キーが存在する");
        assert_eq!(lang.text(key), expected.as_str(), "ja 辞書値が返る: {k}");
    }
    for name in behavior_names() {
        let expected = ja.get(&name).expect("ja.toml に Behavior 名が存在する");
        assert_eq!(
            lang.behavior_text(&name),
            expected.as_str(),
            "ja 辞書の Behavior 訳が返る: {name}"
        );
    }
}

/// 外部辞書ディレクトリが存在しない → panic せず埋め込み英語（en.toml）の値が返る
///（E1: 起動継続）。
#[test]
fn missing_external_dictionary_falls_back_to_embedded_english() {
    let en = read_dict(&en_dict_path());
    let missing = std::env::temp_dir().join(format!("shimeji_i18n_missing_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&missing);

    let lang = Lang::load(&missing, DEFAULT_LANGUAGE);
    for &key in UiKey::ALL {
        let k = key.key();
        let expected = en.get(k).expect("en.toml に UI キーが存在する");
        assert_eq!(lang.text(key), expected.as_str(), "外部辞書なしは en: {k}");
    }
    assert_eq!(
        lang.behavior_text("ChaseMouse"),
        en.get("ChaseMouse")
            .expect("en.toml に ChaseMouse")
            .as_str(),
        "外部辞書なしの Behavior も en にフォールバック"
    );
}

/// 外部辞書が「存在しない code」の場合も en にフォールバックする。
#[test]
fn missing_language_code_falls_back_to_embedded_english() {
    let en = read_dict(&en_dict_path());
    let lang = Lang::load(&shipped_lang_dir(), "no_such_language_code");
    for &key in UiKey::ALL {
        let k = key.key();
        let expected = en.get(k).expect("en.toml に UI キーが存在する");
        assert_eq!(lang.text(key), expected.as_str(), "code 不在は en: {k}");
    }
}

/// 外部辞書が読めない（ファイルではなくディレクトリ）→ panic せず en にフォールバック
///（E2: 読み込み失敗）。
#[test]
fn unreadable_external_dictionary_falls_back_to_english() {
    let en = read_dict(&en_dict_path());
    let tmp = TempLangDir::new("unreadable");
    std::fs::create_dir_all(tmp.dir().join("dircode.toml")).expect("ディレクトリを作れる");

    let lang = Lang::load(tmp.dir(), "dircode");
    assert_eq!(
        lang.text(UiKey::CallShimeji),
        en.get("CallShimeji")
            .expect("en.toml に CallShimeji")
            .as_str(),
        "読めない外部辞書は en へフォールバック"
    );
}

/// 外部辞書が壊れた TOML → panic せず en にフォールバック（E2 / E9）。
#[test]
fn corrupt_external_dictionary_falls_back_to_english() {
    let en = read_dict(&en_dict_path());
    let tmp = TempLangDir::new("corrupt");
    tmp.write_dict("broken", "CallShimeji = \"unterminated\n");

    let lang = Lang::load(tmp.dir(), "broken");
    assert_eq!(
        lang.text(UiKey::CallShimeji),
        en.get("CallShimeji")
            .expect("en.toml に CallShimeji")
            .as_str(),
        "壊れた TOML は en へフォールバック"
    );
}

/// 外部辞書はロード済みだが当該キーが欠落 → en にフォールバック（E3）。
#[test]
fn key_missing_from_loaded_external_dictionary_falls_back_to_english() {
    let en = read_dict(&en_dict_path());
    let tmp = TempLangDir::new("missing_key");
    tmp.write_dict("partial", "SomeUnrelatedKey = \"x\"\n");

    let lang = Lang::load(tmp.dir(), "partial");
    assert_eq!(
        lang.text(UiKey::CallShimeji),
        en.get("CallShimeji")
            .expect("en.toml に CallShimeji")
            .as_str(),
        "外部辞書のキー欠落は en へフォールバック"
    );
    assert_eq!(
        lang.behavior_text("ChaseMouse"),
        en.get("ChaseMouse")
            .expect("en.toml に ChaseMouse")
            .as_str(),
        "外部辞書の Behavior 名欠落も en へフォールバック"
    );
}

/// 外部辞書の値が空文字 → 「訳なし」扱いで en にフォールバック（E5）。
#[test]
fn empty_external_value_falls_back_to_english() {
    let en = read_dict(&en_dict_path());
    let tmp = TempLangDir::new("empty_value");
    tmp.write_dict("emptyval", "CallShimeji = \"\"\nChaseMouse = \"\"\n");

    let lang = Lang::load(tmp.dir(), "emptyval");
    assert_eq!(
        lang.text(UiKey::CallShimeji),
        en.get("CallShimeji")
            .expect("en.toml に CallShimeji")
            .as_str(),
        "空文字の UI 値は en へフォールバック"
    );
    assert_eq!(
        lang.behavior_text("ChaseMouse"),
        en.get("ChaseMouse")
            .expect("en.toml に ChaseMouse")
            .as_str(),
        "空文字の Behavior 値は en へフォールバック"
    );
}

/// en にも無い Behavior 名 → 名前（キー文字列）がそのまま返る（E4 / §8）。
/// UiKey 側の key() 返却は埋め込み en 破損時のみだが、公開 API から強制できないため
/// 動的キーである Behavior 名で契約を検証する。
#[test]
fn unknown_behavior_name_is_returned_verbatim() {
    let lang = Lang::load(&shipped_lang_dir(), DEFAULT_LANGUAGE);
    assert_eq!(
        lang.behavior_text("NoSuchBehaviorXyzzy"),
        "NoSuchBehaviorXyzzy",
        "en にも無いキーは名前をそのまま返す"
    );
}

/// `code` が空・不正文字（パストラバーサル等）→ panic せず既定 "en" 扱い（E11）。
/// 既定言語の出荷辞書値が返る。
#[test]
fn invalid_or_empty_language_code_falls_back_to_default_without_panic() {
    let en = read_dict(&en_dict_path());
    let expected = en
        .get("CallShimeji")
        .expect("en.toml に CallShimeji")
        .as_str();

    for code in ["", "../x", "a/b", "..", "lang..toml", "日本語"] {
        let lang = Lang::load(&shipped_lang_dir(), code);
        assert_eq!(
            lang.text(UiKey::CallShimeji),
            expected,
            "不正/空の code {code:?} は既定 en 扱い"
        );
    }
}

// =====================================================================
// 契約 4: settings.toml 後方互換（general.language）
// =====================================================================

/// `DEFAULT_LANGUAGE` は "en"。
#[test]
fn default_language_constant_is_en() {
    assert_eq!(DEFAULT_LANGUAGE, "en");
}

/// `GeneralSettings::default()` の language は既定 "en"。
#[test]
fn general_settings_default_language_is_en() {
    assert_eq!(
        GeneralSettings::default().language.as_str(),
        DEFAULT_LANGUAGE
    );
}

/// `language` フィールドを持たない settings TOML を読むと `language == "en"`（E6・後方互換）。
#[test]
fn settings_without_language_field_defaults_to_en() {
    let legacy = concat!("[allowed]\n", "breeding = false\n",);
    let settings: Settings =
        toml::from_str(legacy).expect("language 無し TOML を Settings にデシリアライズできる");
    assert_eq!(
        settings.general.language.as_str(),
        DEFAULT_LANGUAGE,
        "欠落時は serde default で en"
    );
    assert!(!settings.allowed.breeding, "他フィールドは従来どおり");
}

/// `[general]` セクションに language が無くても既定 "en"（E6）。
#[test]
fn settings_general_without_language_defaults_to_en() {
    let legacy = concat!("[general]\n", "show_console = true\n",);
    let settings: Settings = toml::from_str(legacy).expect("[general] TOML を読める");
    assert!(settings.general.show_console, "既存フィールドは読まれる");
    assert_eq!(settings.general.language.as_str(), DEFAULT_LANGUAGE);
}

/// `[general] language = "en"` を明示指定するとその値が読まれる（キー名の固定）。
#[test]
fn settings_general_language_is_read_from_toml() {
    let sample = concat!("[general]\n", "language = \"en\"\n",);
    let settings: Settings = toml::from_str(sample).expect("[general] language を読める");
    assert_eq!(settings.general.language.as_str(), "en");
}
