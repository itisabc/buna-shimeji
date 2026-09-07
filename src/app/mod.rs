//! app モジュール — Manager（マスコット集合 + tick スケジュール）と
//! Environment（[`EnvironmentView`](crate::mascot::EnvironmentView) の OS 供給実装）。
//!
//! design.md §1.2 / §1.5 / §1.7 / §1.8・タスク #8。Java 正本
//! （.tmp/java-ref/Manager.java・environment/AbstractEnvironment.java・
//! environment/Location.java・environment/Area.java・platform/WindowsEnvironment.java）
//! を仕様として逐語移植する。
//!
//! 構造は design §1.7 の一方向依存（mascot ← app）。所有は Manager:
//! `Manager` が `Vec<Mascot>` と `Environment` を所有し、不変の `ImageSet` のみ
//! `Arc` で共有する（design §1.5・グローバル可変状態禁止）。
//!
//! 意図的差異の列挙は各モジュール（environment.rs / manager.rs）のモジュール doc 参照。

pub mod environment;
pub mod manager;
