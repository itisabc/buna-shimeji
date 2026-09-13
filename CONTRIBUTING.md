# コントリビューションガイド

Shimeji (Rust 実装) への貢献を歓迎します。

## 開発環境

- **Windows のみ**（Win32 API / windows-rs に依存するため、ビルド・実行は Windows が必要です）
- Rust stable（MSVC ツールチェーン）

```powershell
cargo build --release   # リリースビルド
cargo run --release     # 実行（exe と同じ場所に conf/ と img/ が必要）
cargo test              # テスト
cargo fmt               # フォーマット
cargo clippy            # lint
```

## プルリクエスト

- 変更は 1 つの目的に絞ってください。
- `cargo test` が全て通ることを確認してください（既存テストを壊さない）。
- `cargo fmt` と `cargo clippy` を通してください。
- 挙動を変える場合は、PR の説明に理由を書いてください。

## 画像セットの追加

`img/<SetName>/` にフォルダを置くだけで新しい画像セットを追加できます。
詳細は [`doc/imageset-guide.md`](doc/imageset-guide.md) を参照してください。

## ライセンス

本プロジェクトは [LICENSE](LICENSE) の下で公開されています。
貢献したコードは同じライセンスで提供されることに同意したものとみなします。
