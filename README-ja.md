# Cranberry

Cranberry は、Prometheus HTTP API 経由で Prometheus のメトリクスを参照するための Rust 製 TUI ダッシュボードです。
Loki のログを別画面で追いかけることもできます。

## 実行

```bash
cargo run
```

`cranberry.toml` が存在する場合は、自動的に読み込まれます。

コマンドラインから Prometheus の base URL を上書きすることもできます。

```bash
cargo run -- http://127.0.0.1:9090
```

別の設定ファイルを指定する場合は次のように実行します。

```bash
cargo run -- --config /path/to/cranberry.toml
```

## 設定

`cranberry.toml.sample` の例:

```toml
[prometheus]
base_url = "http://127.0.0.1:9090"

[loki]
base_url = "http://127.0.0.1:3100"
host_label = "host"
log_label = "job"
poll_secs = 1
lookback_secs = 1200

[display]
max_metrics = 20
initial_metric = "up"
refresh_secs = 15

[logging]
path = "cranberry.log"
level = "info"
```

利用できる設定項目:

- `prometheus.base_url`: Prometheus サーバーの base URL。例: `http://127.0.0.1:9090`
- `display.max_metrics`: ターゲットとテキストフィルタ適用後に表示するメトリクス数の上限
- `display.initial_metric`: 起動時に最初に選択するメトリクス名
- `display.refresh_secs`: 自動更新間隔（秒）
- `loki.base_url`: Loki サーバーの base URL。デフォルトは `http://127.0.0.1:3100`
- `loki.host_label`: ホスト選択に使うラベル名。デフォルトは `host`
- `loki.log_label`: ログ種別選択に使うラベル名。デフォルトは `job`
- `loki.poll_secs`: ログ更新のポーリング間隔（秒）。デフォルトは `1`
- `loki.lookback_secs`: ログ画面を開いたときの初期取得範囲（秒）。デフォルトは `1200`
- `logging.path`: ログファイルのパス。デフォルトは `cranberry.log`
- `logging.level`: ログレベル。`trace`、`debug`、`info`、`warn`、`error` のいずれか。デフォルトは `info`

`prometheus.base_url` を省略した場合、Cranberry は組み込みのサンプルメトリクスで起動します。

## 操作

- `q`: 終了
- `j` / `k`: 選択を移動
- `[` / `]`: ターゲットを切り替え
- `t`: ターゲットピッカーを開く
- `l`: Loki ログビューアを開く
- `/`: メトリクスフィルタ入力を開く
- `r`: 即時リロード
- ログ画面で `Tab` / `h` / `l`: ホスト一覧とログ一覧のフォーカスを切り替え
- `Esc`: ターゲットピッカー、フィルタ入力、ログ画面を閉じる
- `Enter`: ターゲットピッカーの選択を適用、またはフィルタ入力を閉じる
- `Backspace`: フィルタ入力で 1 文字削除
- `Ctrl-U`: フィルタ入力をクリア
