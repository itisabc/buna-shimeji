# Shimeji (Rust 実装)

[![License: BSD-3-Clause](https://img.shields.io/badge/License-BSD_3--Clause-blue.svg)](LICENSE)
![Platform: Windows](https://img.shields.io/badge/Platform-Windows-0078D6.svg)
![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)

Windows 用デスクトップマスコット「しめじ」の Rust 実装です。
Java 版（Shimeji-ee / Shimeji-Desktop 系）を参考に、主要なマスコット挙動（歩く・落ちる・ジャンプ・
ドラッグ・スロー・増殖・変身・ウィンドウ運び）を、tao + windows-rs による
軽量実装で実現しています（透過描画は CreateDIBSection + UpdateLayeredWindow）。

- 挙動: 主要なマスコット挙動を実装済み（アクションの式・定数は Java 版を参考に実装）
- 追加機能（Rust 版独自）: **ウィンドウを掴んで最前面固定**。マスコットをウィンドウにドラッグ&ドロップすると、
  そのウィンドウが最前面に固定され、しめじがぶら下がって窓の移動に追従します。
  トレイ「Allowed Behaviours」で ON/OFF（既定 OFF）
- 軽量化: メモリ 5〜20MB / アイドル CPU ほぼ 0（tick 駆動・変化時のみ描画。実測: アイドル CPU 0.234%・Private 8.8MB・WorkingSet 22.4MB・スレッド 2）
- 拡張性: Phase 2 で OpenAI 互換 API による LLM エージェント化（Think/Chat）
- 画像差し替え容易: `img/<SetName>/` にフォルダを置くだけで新しい画像セットを利用可能

## ディレクトリ構成

```text
conf/            アクション・ビヘイビア定義（Shimeji-ee 互換 XML）
  actions.xml      アクション定義
  behaviors.xml    ビヘイビア（行動）定義と頻度
  Mascot.xsd       XML スキーマ（ドキュメント用。実行時の検証には使われない）
img/
  Shimeji/         標準しめじ画像セット（shime1.png〜shime46.png + banner.bmp）
  KuroShimeji/     くろしめじ画像セット（同構成）
```

`conf/`・`img/` の資産は
[DalekCraft2/Shimeji-Desktop](https://github.com/DalekCraft2/Shimeji-Desktop)
（コミット `dea89528c10c066626a09609f0e742cbe6405a8d`）から取得し、
内容を一切改変せずに同梱しています。

## ビルド・実行

```powershell
cargo build --release   # リリースビルド
cargo run --release     # 実行（exe と同じ場所に conf/ と img/ が必要）
cargo test              # 単体テスト
```

現状: 主要挙動の実装は完了（`cargo test` 504/504 PASS、2026-09-13）。
実装済み機能の一覧は [`doc/feature-status.md`](doc/feature-status.md) を参照してください。
今後の主開発は Phase 2（LLM エージェントによる Think/Chat、効果音）です。

## 対応環境・既知の制限

- **対応 OS: Windows のみ**（Win32 API・windows-rs による直描きに依存）。macOS / Linux は非対応です。要望があれば検討します。
- **長時間の連続起動は未検証**です（数時間〜日単位の連続稼働テストは未実施）。

## ライセンス

本プロジェクトは本家 Shimeji（Copyright (c) 2009-2011 Yuki Yamada、zlib/libpng ライセンス）
および Shimeji-ee / Shimeji-Desktop（Shimeji-ee Group、New BSD ライセンス。
Shimeji-ee の拡張は Kilkakon — https://kilkakon.com/shimeji/ — による）の派生物であり、
[DalekCraft2/Shimeji-Desktop](https://github.com/DalekCraft2/Shimeji-Desktop)
（コミット `dea89528c10c066626a09609f0e742cbe6405a8d`）のフォークから資産を同梱しています。

同梱画像セット `img/KuroShimeji/` の出所・ライセンスに関する注記を含め、
詳細は [LICENSE](LICENSE) を参照してください。
