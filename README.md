# Shimeji (Rust 実装)

[![License: BSD-3-Clause](https://img.shields.io/badge/License-BSD_3--Clause-blue.svg)](LICENSE)
![Platform: Windows](https://img.shields.io/badge/Platform-Windows-0078D6.svg)
![Rust](https://img.shields.io/badge/Rust-2021-orange.svg)

Windows 用デスクトップマスコット「しめじ」の Rust 実装です。
Java 版（Shimeji-ee / Shimeji-Desktop 系）を参考に、主要なマスコット挙動（歩く・落ちる・ジャンプ・
ドラッグ・スロー・増殖・変身・ウィンドウ運び）を、tao + windows-rs による
軽量実装で実現しています（透過描画は CreateDIBSection + UpdateLayeredWindow）。
移植したものの本人が大して詳しくもないのでなんか問題あったら教えてください。

- 挙動: 主要なマスコット挙動を実装済み（アクションの式・定数は Java 版を参考に実装）
- アフォーダンス: 他個体へ「放送」して相手を探させ、到達したら互いの行動を差し替える
  Shimeji-ee のアフォーダンス機能（`Broadcast` / `ScanMove`）を実装済み
- set ごとの設定: `img/<Set>/conf/` → `conf/<Set>/` → `conf/` の順に `Actions.xml` /
  `Behavior.xml` を解決するため、set ごとに同名アクションでも別の内容を持てます
- 追加機能（Rust 版独自）: **ウィンドウを掴んで最前面固定**。マスコットをウィンドウにドラッグ&ドロップすると、
  そのウィンドウが最前面に固定され、しめじがぶら下がって窓の移動に追従します。
  最大化中のウィンドウは固定対象外で、固定中にその窓が最大化されたら自動で解除します。
  トレイ「Allowed Behaviours」で ON/OFF（既定 OFF）
- 追加機能（Rust 版独自）: **色づけ（tint）**。`img/<Set>/conf/actions.xml` に `<TintPalette>` を
  書くと色つきの個体を出せます（色相を回し続ける / 出現のたびに抽選する）。出てよい色は
  `conf/settings.toml` の `[tint.sets.<set>]` で絞り、トレイの「呼ぶ → 画像セット → 色」から選べます。
  詳しくは [画像セット差し替えガイド](doc/imageset-guide.md) §12
- 軽量化: メモリ 5〜20MB / アイドル CPU ほぼ 0（tick 駆動・変化時のみ描画。実測値は「[性能](#性能)」を参照）
- 画像差し替え容易: `img/<SetName>/` にフォルダを置くだけで新しい画像セットを利用可能
- デフォルトのしめじ以外ほとんど使わないので差し替え機能は検証不足です。


## 性能

計測: Windows / Core i7-1255U（12 コア）/ `cargo build --release` のバイナリ。

### アイドル時（1 体・5 秒サンプル）

| 指標 | 実測 |
|---|---|
| CPU | 0.234%（全コア換算） |
| Private | 8.8 MB |
| WorkingSet | 22.4 MB |
| スレッド | 2 |

tick は 40ms 固定ですが、次 tick までは `ControlFlow::WaitUntil` で OS に待たせ、
変化が無ければ描画もウィンドウ移動もしません（`needs_repaint`）。

### 100 体同時表示時（描画改善の前後比較）

1 tick の内訳です（条件: `Shimeji` セット・体数を 100 に固定・Transients OFF）。

| 内訳 | 改善前 | 改善後 |
|---|---|---|
| tick 合計 | 32.8 ms | 7.4 ms |
| うち sim（状態更新・スクリプト） | 1.0 ms | 1.0 ms |
| うち draw（合成・窓反映） | 31.8 ms | 4.7 ms |
| WorkingSet | 39.3 MB | 39.5 MB |

計測の結果、描画コストの主因は `UpdateLayeredWindow` ではなく **毎 tick の `SetWindowPos`（窓移動）**
でした（100 窓で 33.2 ms/iter、体数に対して超線形）。そこで窓をフレーム寸法ぴったりにし、
sprite は常にローカル座標 `(0,0)` に置いて（内容を位置に依存させない）、移動は `DeferWindowPos` で
1 tick 1 バッチにまとめ、`UpdateLayeredWindow` は内容が変わったときだけ呼ぶようにしました。
移動の純コストは 33.2 → **3.0 ms/iter** です。

数十体までなら元々 tick に余裕があるため体感差はなく、効くのは 100 体規模まで増殖させた場合です。


## ディレクトリ構成

```text
conf/            アクション・ビヘイビア定義（Shimeji-ee 互換 XML）と設定
  actions.xml      アクション定義（set 専用ファイルが無い場合の共通定義）
  behaviors.xml    ビヘイビア（行動）定義と頻度（同上）
  Mascot.xsd       XML スキーマ（ドキュメント用。実行時の検証には使われない）
  settings.toml    設定ファイル（初回起動時に自動生成。詳細は「設定」節）
  lang/            UI 文言の辞書（en.toml / ja.toml）
  <SetName>/       set 専用の Actions.xml / Behavior.xml（任意・共通定義より優先）
img/
  Shimeji/         標準しめじ画像セット（shime1.png〜shime46.png + banner.bmp）
  KuroShimeji/     くろしめじ画像セット（同構成）
  <SetName>/conf/  set 専用 conf を画像側に置く場合の位置（最優先）
```

`conf/`・`img/` の資産は
[DalekCraft2/Shimeji-Desktop](https://github.com/DalekCraft2/Shimeji-Desktop)
（コミット `dea89528c10c066626a09609f0e742cbe6405a8d`）から取得し、
内容を一切改変せずに同梱しています。

## 設定（`conf/settings.toml`）

設定 GUI はありません。挙動の切り替えはトレイメニュー、または exe と同じフォルダの
`conf/settings.toml` を直接編集して行います（レジストリは使いません）。ファイルは起動時に
読み込まれ、無ければ既定値で自動生成されます。削除して再起動すれば既定に戻ります。

```toml
[general]
show_console = false        # true でログ表示用のコンソールウィンドウを確保する
language     = "en"         # UI 文言の言語。同梱は "en"（英語・既定） / "ja"（日本語）

[allowed]                   # トレイの Allowed Behaviours と同じ（true で許可）
breeding           = true   # 増殖（分裂）
transients         = true   # 特殊効果（一定時間で消える増殖個体）
transformation     = true   # 変身（スキン変更）
throwing           = true   # ウィンドウを投げる
sounds             = true   # 効果音
multiscreen        = true   # マルチモニタで複数の画面をまたいで動く
pin_dropped_window = false  # ドロップしたウィンドウを最前面に固定（既定 OFF）

[disabled_behaviors]        # 特定の Behavior を止める（任意・set 名 = ["Behavior 名", ...]）
# Shimeji = ["SitDown", "SplitIntoTwo"]

[imagesets.scale]           # 画像セットごとの拡大率（任意・既定 1.0）
# Shimeji = 0.5

[tint.sets.Shimeji]         # 出現を許可する色（任意・set ごと。書かなければ全色）
# colors = ["strawberry", "white"]   # 色 id は set の actions.xml の <TintPalette> が定義する

[interactive_windows]       # 反応するウィンドウ（タイトル部分一致。両方空 = どのウィンドウにも反応しない）
whitelist = []
blacklist = []
```

- 初回起動時に生成される `conf/settings.toml` には、上の説明がコメントとして入っています
  （トレイでトグルを切り替えて保存されても説明は残ります）。
- 反映タイミング: `[allowed]` と `[tint.sets]` はトレイ操作なら即時（ファイルを編集した場合は次回起動時）、
  それ以外（`language` / `[disabled_behaviors]` / `[imagesets.scale]` / `[interactive_windows]`）は
  次回起動時です。

## ビルド・実行

```powershell
cargo build --release   # リリースビルド
cargo run --release     # 実行（exe と同じ場所に conf/ と img/ が必要）
cargo test              # 単体テスト
```

現状: 主要挙動の実装は完了（`cargo test` 642/642 PASS、2026-09-23）。
実装済み機能の一覧は [`doc/feature-status.md`](doc/feature-status.md) を参照してください。

## 対応環境・既知の制限

- **対応 OS: Windows のみ**（Win32 API・windows-rs による直描きに依存）。macOS / Linux は非対応です。
- **長時間の連続起動は未検証**です（数時間〜日単位の連続稼働テストは未実施）。

## ライセンス

本プロジェクトは本家 Shimeji（Copyright (c) 2009-2011 Yuki Yamada、zlib/libpng ライセンス）
および Shimeji-ee / Shimeji-Desktop（Shimeji-ee Group、New BSD ライセンス。
Shimeji-ee の拡張は Kilkakon — https://kilkakon.com/shimeji/ — による）の派生物であり、
[DalekCraft2/Shimeji-Desktop](https://github.com/DalekCraft2/Shimeji-Desktop)
（コミット `dea89528c10c066626a09609f0e742cbe6405a8d`）のフォークから資産を同梱しています。

同梱画像セット `img/KuroShimeji/` の出所・ライセンスに関する注記を含め、
詳細は [LICENSE](LICENSE) を参照してください。
