//! UI 文言の国際化（i18n）基盤（設計 `docs/plans/done/i18n-design.md` §1）。
//!
//! 実行時 UI（トレイ / ポップアップ）のラベルを辞書経由で解決する。
//! - 埋め込み英語ベース: `assets/lang/en.toml`（バイナリ同梱・ユーザー編集対象外）
//! - 外部辞書: `conf/lang/<code>.toml`（英語・日本語を同梱。`settings.toml` の
//!   `[general] language` で選択・既定 [`DEFAULT_LANGUAGE`]）
//! - 解決優先順: 外部辞書 → 埋め込み英語 → キー文字列
//!
//! `Lang` は tao イベントループ 1 本の単一スレッドでのみ使用する前提
//! （per-key warn 抑制に `RefCell` を使う）。

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// 言語コードの既定（`settings.toml` の serde default と手書き `Default` で共用）。
pub const DEFAULT_LANGUAGE: &str = "en";

/// 埋め込み英語ベース（shipped conf には置かない）。
const EMBEDDED_EN: &str = include_str!("../assets/lang/en.toml");

/// UI キーの型付き正本（タイポをコンパイルエラーで防ぐ）。
/// バリアント名 = [`UiKey::key`] の返す辞書キー（Java `language.properties` 準拠）。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UiKey {
    CallShimeji,
    SpawnRandom,
    FollowCursor,
    ReduceToOne,
    RestoreWindows,
    AllowedBehaviours,
    BreedingCloning,
    Transformation,
    ThrowingWindows,
    Multiscreen,
    SoundEffects,
    BreedingTransient,
    PinDroppedWindow,
    PauseAnimations,
    ResumeAnimations,
    DismissAll,
    Reload,
    CallAnother,
    SetBehaviour,
    Dismiss,
    Shimeji,
}

impl UiKey {
    /// 全キー（辞書網羅テストが走査する）。
    pub const ALL: &'static [UiKey] = &[
        UiKey::CallShimeji,
        UiKey::SpawnRandom,
        UiKey::FollowCursor,
        UiKey::ReduceToOne,
        UiKey::RestoreWindows,
        UiKey::AllowedBehaviours,
        UiKey::BreedingCloning,
        UiKey::Transformation,
        UiKey::ThrowingWindows,
        UiKey::Multiscreen,
        UiKey::SoundEffects,
        UiKey::BreedingTransient,
        UiKey::PinDroppedWindow,
        UiKey::PauseAnimations,
        UiKey::ResumeAnimations,
        UiKey::DismissAll,
        UiKey::Reload,
        UiKey::CallAnother,
        UiKey::SetBehaviour,
        UiKey::Dismiss,
        UiKey::Shimeji,
    ];

    /// 辞書キー文字列（= Java `language.properties` のキー。Rust 独自キーは同名新規）。
    pub fn key(self) -> &'static str {
        match self {
            UiKey::CallShimeji => "CallShimeji",
            UiKey::SpawnRandom => "SpawnRandom",
            UiKey::FollowCursor => "FollowCursor",
            UiKey::ReduceToOne => "ReduceToOne",
            UiKey::RestoreWindows => "RestoreWindows",
            UiKey::AllowedBehaviours => "AllowedBehaviours",
            UiKey::BreedingCloning => "BreedingCloning",
            UiKey::Transformation => "Transformation",
            UiKey::ThrowingWindows => "ThrowingWindows",
            UiKey::Multiscreen => "Multiscreen",
            UiKey::SoundEffects => "SoundEffects",
            UiKey::BreedingTransient => "BreedingTransient",
            UiKey::PinDroppedWindow => "PinDroppedWindow",
            UiKey::PauseAnimations => "PauseAnimations",
            UiKey::ResumeAnimations => "ResumeAnimations",
            UiKey::DismissAll => "DismissAll",
            UiKey::Reload => "Reload",
            UiKey::CallAnother => "CallAnother",
            UiKey::SetBehaviour => "SetBehaviour",
            UiKey::Dismiss => "Dismiss",
            UiKey::Shimeji => "Shimeji",
        }
    }
}

/// UI 文言辞書。外部辞書と埋め込み英語ベースを保持する。
pub struct Lang {
    /// `conf/lang/<code>.toml`（空 = 外部辞書なし / 読込失敗）。
    table: HashMap<String, String>,
    /// 埋め込み `assets/lang/en.toml`（フォールバック）。
    base: HashMap<String, String>,
    /// 外部辞書を読めたか（per-key warn を出すかの分岐）。
    external_loaded: bool,
    /// per-key warn の多重出力抑制。
    warned: RefCell<HashSet<String>>,
}

impl Lang {
    /// 辞書を読み込む。呼び出しは `env_logger` 初期化後に行うこと（warn を消さない）。
    ///
    /// `code` は検証する: 空 → 既定 `"en"`、`[A-Za-z0-9_-]` 以外を含む → warn +
    /// 既定 `"en"`（パストラバーサル防止）。外部辞書の不在 / 読み込み失敗 /
    /// パース失敗は warn（1 回）のうえ埋め込み英語へフォールバックする。
    pub fn load(lang_dir: &Path, code: &str) -> Lang {
        let code = sanitize_code(code);

        let base = match toml::from_str::<HashMap<String, String>>(EMBEDDED_EN) {
            Ok(map) => map,
            Err(err) => {
                log::warn!("failed to parse embedded English dictionary: {err}");
                HashMap::new()
            }
        };

        let path = lang_dir.join(format!("{code}.toml"));
        let (table, external_loaded) = match std::fs::read_to_string(&path) {
            Ok(text) => match toml::from_str::<HashMap<String, String>>(&text) {
                Ok(map) => (map, true),
                Err(err) => {
                    log::warn!("failed to parse dictionary {}: {err}", path.display());
                    (HashMap::new(), false)
                }
            },
            Err(err) => {
                log::warn!("failed to load dictionary {}: {err}", path.display());
                (HashMap::new(), false)
            }
        };

        Lang {
            table,
            base,
            external_loaded,
            warned: RefCell::new(HashSet::new()),
        }
    }

    /// UI キー（型付き）を解決する。優先順: 外部辞書 → 埋め込み英語 → `key()`。
    pub fn text(&self, key: UiKey) -> &str {
        let k = key.key();
        self.resolve(k).unwrap_or(k)
    }

    /// Behavior 名（動的キー）を解決する。優先順: 外部辞書 → 埋め込み英語 → `name`。
    pub fn behavior_text<'a>(&'a self, name: &'a str) -> &'a str {
        self.resolve(name).unwrap_or(name)
    }

    /// 解決の共通経路。外部辞書ロード済みで当該キーが無い/空の場合は
    /// per-key warn（キー毎に 1 回）を出す。
    fn resolve<'a>(&'a self, key: &'a str) -> Option<&'a str> {
        if let Some(value) = self.table.get(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
        if self.external_loaded {
            self.warn_once(key);
        }
        if let Some(value) = self.base.get(key) {
            if !value.trim().is_empty() {
                return Some(value);
            }
        }
        log::warn!("no translation for dictionary key {key:?}");
        None
    }

    /// 同一キーの warn を 1 回に抑制する。
    fn warn_once(&self, key: &str) {
        let mut warned = self.warned.borrow_mut();
        if warned.insert(key.to_string()) {
            log::warn!("missing translation in external dictionary: {key:?}");
        }
    }
}

/// 言語コードを検証し、空 / 不正文字なら既定 [`DEFAULT_LANGUAGE`] を返す。
fn sanitize_code(code: &str) -> &str {
    if code.is_empty() {
        log::warn!("language code is empty; falling back to {DEFAULT_LANGUAGE}");
        return DEFAULT_LANGUAGE;
    }
    if !code
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        log::warn!("invalid language code {code:?}; falling back to {DEFAULT_LANGUAGE}");
        return DEFAULT_LANGUAGE;
    }
    code
}
