//! タスク #10a: `src/win/os_source.rs`（OsSource 実 Win32 実装体）から coder が
//! 抽出する公開純関数 `is_interactive_by_title` の契約テスト（TDD RED）。
//!
//! Java 正本: `.tmp/java-ref/platform/WindowsEnvironment.java` `isInteractive`
//! L87-142 のタイトル判定部分（HWND キャッシュ L88-91 を除く逐語）。
//! coder は `pub fn is_interactive_by_title(title: &str, whitelist: &[&str],
//! blacklist: &[&str]) -> bool` として抽出する前提。
//!
//! 設定は `interactiveWindows` / `interactiveWindowsBlacklist` の値のみが引数に
//! 流入する。資産の settings.properties に該当記述が無いため両リストは空固定
//! （orch 決定 = 全窓非 interactive・Java 資産既定と同一挙動）。
//! `interactiveCache` / `refreshCache` は設定不変のため非実装（compute-once）で
//! テスト対象外。getWindowStatus L144-176 / findActiveWindow L178-194 の
//! visible/cloaked/zoomed/iconic 選別は OS 依存領域のため本テスト外（手動確認）。
//!
//! 契約（Java 行番号引用・照合は String.contains 相当の部分一致・大文字小文字区別）:
//! - 空タイトルは最短で false（L97-100・`isEmpty` は正確な空。trim 後空ではない）
//! - blacklist 先行（L102-115）: trim 後空の項目はスキップ（L108）し、
//!   生項目の contains 一致で false（L110）
//! - whitelist（L118-131）: trim 後空の項目はスキップ（L123）し、
//!   生項目の contains 一致で true（L126）
//! - 末尾分岐（L133-141）: `whitelistInUse || !blacklistInUse` → false、
//!   それ以外（blacklist のみ使用中かつ不一致）→ true。
//!   inUse = 「trim 後空でない項目が 1 つ以上存在」の意
//! - 資産既定 pin: 両リスト空 → 任意タイトルで false
//!
//! TDD RED: `src/win/os_source.rs` は未作成のため import 未解決の
//! コンパイルエラーになることが正常。

use simeji::win::os_source::is_interactive_by_title;

// =====================================================================
// 契約 1: 空タイトルは最短で false（L97-100）
// =====================================================================

/// `""` はリスト内容に関係なく false。blacklist のみ使用中の構成
/// （リスト論理だけなら末尾 else 分岐で true になる）でも L97 が先行する。
#[test]
fn empty_title_is_not_interactive() {
    assert!(
        !is_interactive_by_title("", &[], &[]),
        "空タイトル・両リスト空"
    );
    assert!(
        !is_interactive_by_title("", &["notepad"], &[]),
        "空タイトル・whitelist 一致候補あり"
    );
    assert!(
        !is_interactive_by_title("", &[], &["x"]),
        "空タイトル・blacklist のみ使用中（else 分岐に到達する前に false）"
    );
}

/// L97 の `windowTitle.isEmpty()` は正確な空の判定（trim 後空ではない）。
/// 空白のみタイトル `" "` は空扱いしないためリスト判定に進む。
#[test]
fn whitespace_title_is_not_treated_as_empty() {
    // blacklist ["x"] は不一致・blacklist のみ使用中 → 末尾 else 分岐（L137-141）で true
    assert!(
        is_interactive_by_title(" ", &[], &["x"]),
        "空白のみタイトルは空扱いしない（blacklist のみ使用中・不一致 → interactive）"
    );
}

// =====================================================================
// 契約 2: blacklist 先行（L102-115）
// =====================================================================

/// whitelist も blacklist も一致する場合、blacklist が先に判定され false。
#[test]
fn blacklist_match_takes_precedence_over_whitelist() {
    assert!(
        !is_interactive_by_title("Notepad - Untitled", &["Notepad"], &["Untitled"]),
        "両リスト一致 → blacklist 先行で false"
    );
    assert!(
        !is_interactive_by_title("Firefox", &["fire"], &["Fox"]),
        "両リスト部分一致 → false"
    );
}

/// blacklist のみで一致 → false。複数項目の場合、後続項目の一致でも false。
#[test]
fn blacklist_match_is_not_interactive() {
    assert!(
        !is_interactive_by_title("Notepad", &[], &["pad"]),
        "blacklist 部分一致"
    );
    assert!(
        !is_interactive_by_title("Notepad", &[], &["Chrome", "Note"]),
        "複数 blacklist 項目の後続が一致"
    );
}

// =====================================================================
// 契約 3: whitelist 一致（L118-131）
// =====================================================================

/// whitelist 一致 → true（blacklist が不一致なら whitelist で判定される）。
/// 複数項目の場合、後続項目の一致でも true。
#[test]
fn whitelist_match_is_interactive() {
    assert!(
        is_interactive_by_title("Notepad", &["pad"], &["Chrome"]),
        "whitelist 一致・blacklist 不一致"
    );
    assert!(
        is_interactive_by_title("Notepad", &["Firefox", "Note"], &[]),
        "複数 whitelist 項目の後続が一致"
    );
}

/// contains は部分一致（exact 等価ではない）・大文字小文字を区別する
/// （Java String.contains 相当）。
#[test]
fn contains_is_partial_and_case_sensitive() {
    assert!(
        is_interactive_by_title("Notepad", &["pad"], &[]),
        "部分一致（完全一致でなくても true）"
    );
    assert!(
        !is_interactive_by_title("NOTEPAD", &["notepad"], &[]),
        "大文字小文字が違うと不一致（whitelist 使用中・不一致 → false）"
    );
    assert!(
        !is_interactive_by_title("notepad", &["NOTEPAD"], &[]),
        "逆方向も大文字小文字を区別"
    );
}

// =====================================================================
// 契約 4: 末尾分岐（L133-141）
// =====================================================================

/// whitelist 使用中（不一致）→ false。
#[test]
fn whitelist_in_use_without_match_is_not_interactive() {
    assert!(
        !is_interactive_by_title("Something Else", &["Notepad"], &[]),
        "whitelist 使用中・不一致"
    );
    assert!(
        !is_interactive_by_title("c", &["a"], &["b"]),
        "両リスト使用中・不一致 → `whitelistInUse || !blacklistInUse` が true で false"
    );
}

/// blacklist のみ使用中・不一致 → true（L137-141 else・Java の
/// 「blacklist 以外は interactive」意味論）。
#[test]
fn blacklist_only_without_match_is_interactive() {
    assert!(
        is_interactive_by_title("Some Title", &[], &["Explorer"]),
        "blacklist のみ使用中・不一致 → interactive"
    );
    assert!(
        is_interactive_by_title("Some Title", &[], &["A", "B"]),
        "複数 blacklist 項目がすべて不一致でも同じ"
    );
}

// =====================================================================
// 契約 5: 資産既定（両リスト空固定・orch 決定）
// =====================================================================

/// 資産の settings.properties に interactiveWindows / interactiveWindowsBlacklist
/// の記述が無いため両リスト空固定 = 任意タイトルで false（全窓非 interactive）。
#[test]
fn empty_lists_not_interactive_for_any_title() {
    for title in ["anything", "Notepad", "x y z"] {
        assert!(
            !is_interactive_by_title(title, &[], &[]),
            "両リスト空 → title={title:?} も false"
        );
    }
}

// =====================================================================
// 契約 6: trim 後空の項目はスキップし inUse に数えない（L108 / L123）
// =====================================================================

/// 空項目 `""` をスキップしないと `contains("")` が常に真になり、
/// 空白項目 `" "` を inUse に数えて照合するとタイトル中の空白に一致してしまう。
/// いずれも Java と矛盾。
#[test]
fn blank_entries_are_skipped_and_do_not_count_as_in_use() {
    assert!(
        !is_interactive_by_title("anything", &[""], &[]),
        "whitelist 空項目はスキップ（skip しないと contains(\"\") で true になる）"
    );
    assert!(
        !is_interactive_by_title("x", &[], &[" "]),
        "blacklist 空白項目は inUse に数えない（数えると else 分岐で true になる）"
    );
    assert!(
        !is_interactive_by_title("anything", &["", "  "], &[" ", ""]),
        "両リストが全項目 blank = 両リスト空と同じ"
    );
    // タイトルが空白を含むため、blacklist `" "` を skip 判定せず inUse に数えて
    // 照合すると `"notepad - memo.txt".contains(" ")` = true → blacklist 先行で
    // false になる。正実装は L108 で skip（inUse=false・照合しない）し、whitelist
    // `"notepad"` の部分一致（大文字小文字込み・L126）で true。
    assert!(
        is_interactive_by_title("notepad - memo.txt", &["  ", "notepad"], &[" "]),
        "blank 項目を飛ばして whitelist 項目で一致 → true（skip しないと blacklist \" \" 一致で false）"
    );
}

// =====================================================================
// 契約 7: contains の照合対象は生項目（trim は空判定のみ・L110 / L126）
// =====================================================================

/// Java は `windowTitle.contains(title)` にループ変数（生項目）を渡す
/// （L110 / L126）。trim は「trim 後空ならスキップ」の判定にのみ使う。
/// 照合前に trim すると `"otepad "` → `"otepad"` となり Notepad に一致してしまう。
#[test]
fn contains_uses_raw_entry_trim_only_guards_blank() {
    assert!(
        !is_interactive_by_title("Notepad", &["otepad "], &[]),
        "生項目 \"otepad \" は \"Notepad\" に含まれない（whitelist 使用中・不一致 → false）"
    );
}
