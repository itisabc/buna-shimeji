//! エージェントフック（Phase 2 用骨格・design.md §5）。
//!
//! - Mascot は思考アクション（Think/Chat）を [`Agent`] 経由で取得する（Phase 2）。
//!   Think/Chat アクションを Action enum に追加可能な形とする。
//! - API 呼び出しは `std::thread` + `mpsc` で非同期化し、応答は tao の
//!   `EventLoopProxy::send_event` によるイベント受信で受け取る
//!   （tick ループはポーリングしない）。
//! - AgentState は Mascot の外に置き Manager が所有する。
//! - 設定は `conf/agent.toml`（Phase 2 で確定）。
//!
//! Phase 1 はこの骨格のみ。実装本体（HTTP 呼び出し・トークン管理等）は
//! Phase 2 で追加する。

/// 思考/対話エージェントのフック。
///
/// オブジェクト安全（`Box<dyn Agent>` で保持可能）。Send/Sync は
/// スーパートレイトとして要求しない（Phase 2 の非同期化は
/// std::thread + mpsc で行い、tick ループ側はイベント受信のみのため）。
pub trait Agent {
    // Phase 2 で Think/Chat 用メソッドを確定する。Phase 1 は骨格のみ。
}

/// 何もしないエージェント（Phase 1 の既定実装）。
pub struct NoopAgent;

impl Agent for NoopAgent {}
