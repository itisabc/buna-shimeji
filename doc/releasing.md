# 配布とリリース（Releasing）

リリース用 zip の作成と GitHub Release の手順をまとめたドキュメントです（メンテナ向け）。

## 1. 配布物（zip）の構成

```
buna-shimeji-v0.1.1-win64/
├─ shimeji.exe
├─ conf/
│   ├─ actions.xml
│   ├─ behaviors.xml
│   ├─ Mascot.xsd
│   ├─ lang/            (en.toml, ja.toml)
│   └─ settings.toml    ← conf/settings.default.toml をリネームして同封
├─ img/
│   ├─ Shimeji/
│   └─ KuroShimeji/
├─ README.md
└─ LICENSE
```

ポイント:

- `shimeji.exe` と **同じフォルダ**に `conf/` と `img/` が必要です（欠けると起動時エラーになります）。
- `conf/settings.toml` は、リポジトリの **`conf/settings.default.toml`**（デフォルト設定 + 記入例コメント）をリネームして同封します。
- `settings.toml` は**ユーザー固有の設定**（言語・トグル・対象ウィンドウの whitelist など）を含むため、**開発環境のものを混ぜない**こと。zip 内に `settings.default.toml` を残さないこと。

## 2. ビルド

```powershell
cargo build --release
```

## 3. パッケージ（zip 作成）

```powershell
$ver   = "0.1.1"   # Cargo.toml の version に合わせる
$stage = "target/package/buna-shimeji-v$ver-win64"
$zip   = "target/package/buna-shimeji-v$ver-win64.zip"

Remove-Item -Recurse -Force $stage -ErrorAction Ignore
New-Item -ItemType Directory -Force $stage | Out-Null

Copy-Item -Path target/release/shimeji.exe -Destination $stage
Copy-Item -Recurse -Path conf -Destination $stage/conf
Copy-Item -Recurse -Path img  -Destination $stage/img
Copy-Item -Path README.md, LICENSE -Destination $stage

# 開発環境の settings.toml が混ざっていれば除去 → テンプレートを settings.toml として同封
Remove-Item -Force "$stage/conf/settings.toml" -ErrorAction Ignore
Rename-Item "$stage/conf/settings.default.toml" settings.toml

Compress-Archive -Path $stage -DestinationPath $zip -Force
(Get-FileHash $zip -Algorithm SHA256).Hash   # この値を Release 説明文に記載する
```

## 4. 動作確認

- zip を**空のフォルダ**に展開 → `shimeji.exe` を実行 → トレイ常駐・しめじの表示を確認。
- 展開先の `conf/settings.toml` がデフォルトで存在し、`conf/settings.default.toml` が無いことを確認。
- `conf/settings.toml` を削除して再起動すると、既定設定で再生成されることも確認できます。

## 5. GitHub Release の手順

1. リポジトリ → **Releases** → 「Draft a new release」
2. **Tag**: `v0.1.1`（`Cargo.toml` の `version` と一致させる）
3. **Title**: `v0.1.1`
4. **説明欄**: 下のテンプレートを貼る（`<SHA256>` を実際の値に置換）
5. zip を「Attach binaries」で添付 → **Publish release**

## 6. Release 説明文テンプレート（コピペ用）

````markdown
Windows 用デスクトップマスコット「しめじ」の Rust 実装です。

## ダウンロード

下の `buna-shimeji-v0.1.1-win64.zip` をダウンロードして展開してください。

## 使い方

1. zip を好きなフォルダに展開する
2. `shimeji.exe` を実行する
   - `shimeji.exe` と同じフォルダに `conf/` と `img/` が必要です（フォルダ構成は変えないでください）
3. タスクトレイのアイコンから操作（呼ぶ / 一時停止 / Dismiss All など）

## インストール不要・レジストリを変更しません

- インストール作業は不要です。zip を展開して実行するだけです。
- **レジストリは変更しません。** 設定は exe と同じフォルダの `conf/settings.toml` に保存されます。
- アンインストールはフォルダごと削除するだけです。

## 初回起動時の注意

- 「Windows によって PC が保護されました」と表示される場合があります。これはコード署名がないためで、「詳細情報」→「実行」で起動できます。
- ウイルス対策ソフトが誤検知することがあります（画面に重ねて描画する性質上、ヒューリスティック検知されやすいため）。ソースコードは公開しているので確認できます。

## 動作環境

- Windows 10 / 11（64bit）

## 既知の制限

- macOS / Linux は非対応です
- 長時間の連続起動は未検証です

## ライセンス

BSD-3-Clause（詳細は同梱の LICENSE を参照）

## ファイルの整合性

SHA256: `<SHA256>`
````

## 7. メンテナ向け補足

- `conf/settings.default.toml` は、アプリが初回起動時に生成する `settings.toml`（`Settings::create_default_if_missing`・`src/tray.rs`）と**同じ内容**に保ってください。デフォルト値（言語・トグル・scale など）を変更した場合は、このテンプレートも更新します。
- コード署名は未導入のため SmartScreen 警告が出ます。警告を消すには有料のコード署名証明書が必要です（現状は見送り）。
- 追加ランタイム依存を最小化したい場合は、完全静的リンクでビルドできます。クリーンな Windows 環境での動作確認を推奨します。

  ```powershell
  $env:RUSTFLAGS = "-C target-feature=+crt-static"
  cargo build --release
  ```
