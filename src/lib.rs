//! しめじ (Shimeji) Rust 再実装 — ライブラリルート。
//!
//! モジュール構成は AGENTS.md §4 / design.md §1.2 に従う。
//! `config` はタスク #3（XML 強型パース + スクリプト式評価）で追加。

pub mod app;
pub mod config;
pub mod mascot;
pub mod render;
pub mod win;
