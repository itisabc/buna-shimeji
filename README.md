# Shimeji (Rust 実装)

Windows 用デスクトップマスコット「しめじ」の Rust 実装です。
Java 版（Shimeji-ee / Shimeji-Desktop 系）を参考に、主要なマスコット挙動（歩く・落ちる・ジャンプ・
ドラッグ・スロー・増殖・変身・ウィンドウ運び）を、tao + windows-rs による
軽量実装で実現しています（透過描画は CreateDIBSection + UpdateLayeredWindow。

- 挙動: 主要なマスコット挙動を実装済み（アクションの式・定数は Java 版を参考に実装）
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

現状: 主要挙動の実装は完了（`cargo test` 485/485 PASS、2026-09-12、HEAD `3be3218`）。
今後の主開発は Phase 2（LLM エージェントによる Think/Chat、効果音）です。

## ライセンス

本プロジェクトは本家 Shimeji（Copyright (c) 2009-2011 Yuki Yamada、zlib/libpng ライセンス）
および DalekCraft2/Shimeji-Desktop（Shimeji-ee Group、New BSD ライセンス）の派生物です。
ライセンスの詳細は [LICENSE](LICENSE) を参照してください。
