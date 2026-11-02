# BevyConsoleDevTools 設計書

- 日付: 2026-11-02
- 対象: サブプロジェクト① devtools crate 本体
- 対象 Bevy: 0.20 系

## 1. 目的

AI（Claude）が **Remote Control / 画面操作ツールを使わず、Bash などの一般的なコマンド操作だけで**
Bevy ゲームの「画面操作」「画面収録」「デバッグ」を行えるようにする、汎用の Bevy プラグイン crate。

### 成功条件

- 台本ファイル 1 つを渡して起動するだけで、操作・撮影・状態出力が行われ、自動終了する（バッチ型）。
- ゲームを起動したまま `curl` で 1 命令ずつ操作し、結果（画像パス・状態）を受け取れる（対話型）。
- 同じ台本は毎回同じ結果になる（フレーム単位の決定性）。
- 例: CCC の「ノードドラッグ中に Edge が暴れる」問題を、AI が Bash のみで再現・撮影・数値検証できる。

### 非目標（今回やらないこと）

- ウィンドウ無し（オフスクリーン）撮影 → サブプロジェクト③。ただし差し替え可能な構造にしておく。
- CCC の Bevy 0.14 → 0.20 移行と組み込み → サブプロジェクト②（別リポジトリ・別仕様）。
- 複数 Bevy バージョンの同時サポート。crate の 0.x を Bevy の 0.y に対応させ、Bevy 更新時に追従する。

## 2. 全体構成

### 組み込み

```rust
#[cfg(feature = "devtools")]
app.add_plugins(ConsoleDevToolsPlugin::default());
```

ゲーム側の feature で有効化し、リリースビルドには含めない。

### 起動モード

| モード | 起動方法 | 動作 |
|---|---|---|
| バッチ | `BCDT_SCRIPT=<台本.jsonl> BCDT_OUT=<出力dir> cargo run --features devtools` | ゲーム内で台本をフレーム単位に実行し、出力を書いて自動終了 |
| 対話 | 環境変数なしで起動 | BRP（Bevy Remote Protocol, 既定 `localhost:15702`）で待ち受け |

専用 CLI は作らない（環境変数と `curl` で足りるため）。

### モジュール

| モジュール | 責務 |
|---|---|
| `command` | 操作命令の型 `DevCommand`（serde で JSON 相互変換）。バッチと対話で共有 |
| `input` | 命令を Bevy の入力メッセージ（マウスボタン・キー・カーソル位置）として注入。**OS のカーソルは動かさない** |
| `capture` | 画面撮影。現状はウィンドウのスクリーンショット。オフスクリーン化はこのモジュールの差し替えで行う |
| `clock` | 1 フレーム = 固定 1/60 秒で時間を進める（実時間に依存しない）。pause / step も担当 |
| `script` | バッチ実行器。台本をフレーム単位で順に実行 |
| `remote` | BRP に独自メソッド `devtools.<cmd>` を登録 |
| `dump` | 指定した Resource / Component をリフレクションで JSON 出力 |

### 最大のリスク: カーソル位置の注入

多くのゲーム（CCC 含む）は `window.cursor_position()` を直接読む。この値は通常 winit（OS）由来で、
**OS カーソルを動かさずに書き換えられるかは Bevy 0.20 で検証が必要**。
実装計画の最初のタスクとしてこれを検証し、不可能ならゲーム側でカーソル取得を差し替える方式
（devtools が提供するカーソル Resource を読む）を組み込み手順として文書化する。

## 3. 操作命令と台本

### 座標系

ウィンドウの論理ピクセル、原点は左上（`cursor_position()` と同じ）。

### 命令一覧

| 種類 | 命令 | 消費フレーム |
|---|---|---|
| 入力 | `move {x,y}` / `down {button}` / `up {button}` | 0 |
| 入力 | `click {x,y,button?}` | 1 |
| 入力 | `drag {from,to,frames,button?}`（直線補間） | frames |
| 入力 | `key {key, action: press/release/tap}` | 0（tap は 1） |
| 時間 | `wait {frames}` | frames |
| 時間 | `pause` / `resume` / `step {frames}` | ― |
| 撮影 | `screenshot {name?}` | 0 |
| 撮影 | `record {every, frames}` | frames（0 なら「開始のみ」） |
| デバッグ | `dump {resources?, components?}` | 0 |
| デバッグ | `mark {label}` | 0 |
| 終了 | `quit` | ― |

- `button` の既定値は `left`。
- `record` の `frames:0` は「以後 `every` フレームごとに撮影し続ける」。`frames>0` はその間だけ撮影しつつ時間を進める。
- `dump` の型名は Bevy の型レジストリに登録された型パス（`Reflect` 必須）。

### 台本形式（JSONL）

1 行 1 命令。`#` で始まる行と空行は無視。

```jsonl
# ノードをドラッグして配線の暴れを撮る
{"cmd":"wait","frames":30}
{"cmd":"record","every":2,"frames":0}
{"cmd":"drag","from":[300,200],"to":[520,260],"frames":90}
{"cmd":"dump","components":["ccc::EdgeComponent"]}
{"cmd":"quit"}
```

実行規則: 消費 0 フレームの命令は同一フレーム内で連続実行し、消費フレームを持つ命令でそのフレーム数だけ進む。

### 出力（`BCDT_OUT/`）

| パス | 内容 |
|---|---|
| `frames/<frame:06>.png` | 撮影画像（`screenshot` の `name` 指定時は `frames/<name>.png`） |
| `events.jsonl` | 実行した命令ごとに 1 行: `{frame, cmd, result?, error?}` |
| `dump/<frame:06>.json` | 状態出力 |
| 終了コード | 0 = 成功 / 1 = 実行時エラー / 2 = 台本の書式エラー |

### 対話モード

同じ命令を BRP メソッド `devtools.<cmd>` として送る。応答は `{frame, result}`。

```bash
curl -s localhost:15702 -d '{"jsonrpc":"2.0","method":"devtools.click","params":{"x":300,"y":200},"id":1}'
```

状態の閲覧は BRP 標準の `world.query` 等をそのまま使う。

## 4. 決定性（時間制御）

- devtools 有効時、時間の更新を「1 フレーム = 固定 1/60 秒」にする（実時間に依存しない）。
- 対話モードの `pause` / `step` で、HTTP の通信タイミングに依らずフレーム単位で検証できる。

## 5. エラー処理

AI が使う前提のため、曖昧に続行せず速やかに失敗させる。

- **台本の書式エラー**: 起動直後に「行番号と内容」を表示し、1 フレームも実行せず終了コード 2。
- **実行時エラー**（未登録の型を `dump`、撮影失敗など）: `events.jsonl` に記録し、バッチでは終了コード 1。
- **終了し忘れ**: 台本末尾で自動終了。加えて `BCDT_MAX_FRAMES`（既定 36000 ≒ 10 分@60fps）超過で強制終了（終了コード 1）。
- **対話モード**: JSON-RPC のエラー応答で返し、ゲームは止めない。

## 6. テスト

- **単体（ウィンドウ不要）**: 命令のパース、`drag` の補間、台本スケジューラのフレーム計算。
- **結合**: `examples/demo.rs`（四角をドラッグで動かす最小アプリ）を台本で実行し、
  PNG の出力と `dump` 上の四角の移動を検証。ウィンドウが必要なため手元（macOS）のみで実行し、
  CI では除外（`#[ignore]`）。オフスクリーン撮影（③）完成後に CI へ。

## 7. 今後のサブプロジェクト

| 順 | 内容 | 場所 |
|---|---|---|
| ② | CCC を Bevy 0.14 → 0.20 へ移行し devtools を組み込む | CCC |
| ③ | ウィンドウ無し（オフスクリーン）撮影 | BevyConsoleDevTools |
