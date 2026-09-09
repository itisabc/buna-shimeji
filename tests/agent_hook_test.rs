//! タスク #9e: src/agent モジュール（新規・Phase 2 フック土台）の最小 smoke テスト。
//!
//! pin する最小契約のみ:
//! - `Agent` トレイトが存在し、オブジェクト安全である（`Box<dyn Agent>` を構築できる）
//! - `NoopAgent` が `Agent` を実装する（Phase 1 は空実装）
//!
//! トレイトのメソッド名・シグネチャ・Send/Sync 等のスーパートレイト要求の有無は
//! 実装側の設計判断に委ねるため、テストではメソッド呼び出しを固定しない。
//! Phase 1 は空実装のため契約が薄く、これ以上の過剰テストは書かない。
//!
//! TDD RED: simeji::agent は未実装のため「解決できない名前」の
//! コンパイルエラーになることが正常。

use simeji::agent::{Agent, NoopAgent};

/// NoopAgent は Agent を実装し、Box<dyn Agent> として構築できる
///（トレイトがオブジェクト安全であることの smoke）。
#[test]
fn noop_agent_builds_as_box_dyn_agent() {
    let agent: Box<dyn Agent> = Box::new(NoopAgent);
    drop(agent);
}
