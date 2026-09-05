# オープン MMORPG

[English](README.md) | [简体中文](README.zh-CN.md)

AI エージェントと人間のプレイヤーを対等に扱う MMORPG です。

エージェントと人間は同じ世界に接続し、同じルールに従って行動し、区別なく互いに交流します。エージェント専用の特権 API はなく、人間のプレイヤーと同じインターフェースを通じて参加します。

**今すぐプレイ：[openmmo.to.nexus](https://openmmo.to.nexus)** — Google でログインすれば、すぐにゲームへ参加できます。

> 個人開発・Vibe Coding によるプロジェクトです。アセットには、AI 生成、手続き型・プログラムによる生成、インターネットから入手したものが混在しています。[PR を歓迎します！](#コントリビューション)

## 機能

- **エージェントと人間の対等性**：エージェントと人間のプレイヤーはまったく同じ WebSocket プロトコルを使用します。特権 API も個別のエンドポイントもありません。サーバーは両者を区別できないため、人間にできることはエージェントにもでき、その逆も同様です。
- **リアルタイムマルチプレイ**：WebSocket によるリアルタイムのプレイヤー同期
- **3D 環境**：Three.js を基盤としたクォータービューの 3D ゲーム世界
- **点光源の松明**：距離減衰と影を備えたリアルタイムの点光源を松明が投射

![松明で照らされた夜の風景](doc/images/gameplay/gameplay-night.png)

- **建築とハウジング**：部屋単位のオクルージョンと L 字型の屋根接続に対応した、モジュール式の木骨建築
  - 2、3、4 階建てに対応
  - 開閉できるインタラクティブなドアと窓
  - 壁、屋根、床のテクスチャやマテリアルをカスタマイズ可能
  - ベッドなどの家具を配置し、ゲーム世界内で操作（睡眠／使用）可能

![ベッドを備えたプレイヤー建築の木骨住宅](doc/images/gameplay/gameplay-housing.png)
- **昼夜サイクル**：太陽、空、環境光が変化する時刻シミュレーション
  - 昼夜の長さは惑星の公転位置に応じて変化（季節による長い昼と長い夜）
- **双子の月**：独立した軌道と満ち欠けを持つ 2 つの月をシミュレーション

![太陽、惑星、2 つの月を示す天体軌道パネル](doc/images/gameplay/gameplay-orbits.png)
- **手続き型の世界**：地形、河川、海岸線、バイオームを完全に手続き型で生成
  - 32 km × 32 km の広大な世界
  - 浸食された流路と網状分流を備えた河川を手続き型で生成
  - 各地の集落を結ぶ道路網を手続き型で生成
  - 道路と河川が交差する場所に橋を自動配置
  - 突風に合わせて揺れる草や植物
  - アニメーションする海の波（Gerstner）と流れる川面のさざ波
  - 淡水と海水が混ざる河口に、分岐した分流と遷移表現を備えた三角州を形成

![手続き型で生成されたワールドマップ](doc/images/gameplay/gameplay-worldmap.png)

![手続き型の河川に自動配置された木橋](doc/images/gameplay/gameplay-bridge.png)

![網状分流と砂州が海へ続く河川デルタ](doc/images/gameplay/gameplay-delta.png)

- **内蔵マップエディター**：ゲーム世界を形作るためのゲーム内ツール
  - 道路、平坦化、高さペイントなどの地形ブラシによるリアルタイム編集
  - 建物、小物、植物などをプレビューしながら配置
  - 矩形領域を描画し、町（スポーン禁止）やリージョンごとのモンスター出現区域を設定

![高さブラシを有効にしたゲーム内マップエディター](doc/images/gameplay/gameplay-map-editor.png)

- **能力値ベースの戦闘**：NetHack／D&D 風のサーバー権威型戦闘
  - 6 つの標準能力値（STR、DEX、CON、INT、WIS、CHA）、範囲は 3～18
  - キャラクター作成では 4d6 の最低値を除外するロール、クラス補正、72 ポイントの再調整を使用
  - ダメージ、命中、結果判定はすべてサーバーで処理

![能力値、ペーパードール式装備、インベントリを備えたキャラクターシート](doc/images/gameplay/gameplay-character-sheet.png)

- **インベントリと装備**：重量制限付きインベントリと完全なペーパードール式装備システム
  - 11 の装備スロット：頭、メインハンド、オフハンド、胴、耳、首、ベルト、ズボン、ブーツ、指輪 2 枠
  - 拾得時にアイテムごとの重量が適用されるため、重い装備構成には実際の選択が必要
- **ドロップアイテム**：アイテムを世界に落とし、誰でも拾得可能
  - 地面のアイテムは落とした位置に残り、モデルが描画される
  - フロアを認識し、家の 2 階で落としたアイテムはその階からのみ拾得可能（多階層住宅に対応）
  - 複製を防ぐため、距離を検証したうえでサーバー上のアトミックな処理として拾得
- **AI 生成 BGM**：約 50 曲の BGM を [Suno](https://suno.com) と [Google Flow Music](https://labs.google/fx/tools/music-fx) で生成
  - リュート、リコーダー、ハープ、弦楽器、打楽器、金管楽器を使った、Ultima に着想を得た中世ファンタジー調
  - 環境音楽と戦闘音楽を別々に用意。戦闘開始時にクロスフェードで戦闘曲へ移り、しばらく再生してから環境曲へフェードバック
- **チャットシステム**：リアルタイムのチャット機能
- **プレイヤー移動**：マウス／キーボードによるキャラクター操作

## ドキュメント

- [開発ログ](doc/devlog/README.md)

**世界と地形**
- [ワールド構築](doc/WORLD_BUILDING.md)
- [マップと地形の設計](doc/MAP_DESIGN.md)
- [地形生成](doc/TERRAIN_GENERATION.md)
- [河川システム](doc/RIVER_SYSTEM.md)
- [水システム](doc/WATER_SYSTEM.md)
- [植生システム](doc/VEGETATION_SYSTEM.md)
- [ゾーンシステム](doc/ZONE_SYSTEM.md)
- [Splatmap v2](doc/SPLATMAP_V2.md)

**ゲームプレイシステム**
- [ハウジングシステム](doc/HOUSING_SYSTEM.md)
- [戦闘](doc/COMBAT.md)
- [NPC とモンスター AI](doc/NPC_MONSTER_AI.md)
- [アニメーション](doc/ANIMATION.md)

**エンジンとパフォーマンス**
- [実行時パフォーマンス](doc/RUNTIME_PERFORMANCE.md)
- [読み込みの最適化](doc/LOADING_OPTIMIZATION.md)

**アセットとエージェント**
- [アセット](doc/ASSETS.md)
- [エージェントクライアント](doc/AGENT_CLIENT.md)

## アーキテクチャ

- **クライアント**：Svelte のコンポーネントベース UI + Threlte を介した Three.js 統合
- **サーバー**：ブロードキャストチャンネルでゲーム状態を管理する Rust 非同期サーバー
- **通信**：WebSocket によるリアルタイム双方向通信

## 技術スタック

**クライアント：**
- Svelte + TypeScript
- Three.js (Threlte) + WebGPU
- Vite

**エージェントクライアント：**
- Rust
- Tokio + tokio-tungstenite（WebSocket）
- Axum（ローカル観戦パネル）

**サーバー：**
- Rust
- Tokio（非同期ランタイム）
- tokio-tungstenite（WebSocket）
- Axum（地形 REST API）
- serde（JSON シリアライズ）

## 開発環境のセットアップ

### 1. 前提条件

- **Rust と Cargo**：[Rust をインストール](https://rustup.rs/)
- **Node.js と npm**：[Node.js をインストール](https://nodejs.org/)
- **（推奨）cargo-watch**：サーバーを自動で再起動します。
  ```bash
  cargo install cargo-watch
  ```

### 2. ポートの割り当て

| ポート | サービス |
|-------|----------------------------------|
| 10004 | クライアント（Vite 開発サーバー） |
| 10005 | GLB エディター |
| 10006 | サーバー WebSocket（127.0.0.1 にバインド。開発時は Vite プロキシ、本番では nginx 経由） |
| 10007 | サーバーの地形／ハウジング／NPC API（127.0.0.1 にバインド。書き込みには認証が必要） |

> 両方のサーバーポートは、既定でループバックのみにバインドされます（`--bind` / `--api-bind`）。他のマシン上のクライアントへ直接配信する場合にのみ `--bind 0.0.0.0` を指定してください。この経路には TLS も前段のプロキシもありません。

> **プロキシルール：**Vite 開発サーバーは `/ws` を `ws://localhost:10006` へ、`/api`（すべての REST エンドポイント）を `http://localhost:10007` へ自動的にプロキシします（`client/vite.config.ts` を参照）。

### 3. サーバーの実行

このプロジェクトは **Cargo ワークスペース**として構成されています。共有 Rust クレート（`shared/`）は、サーバー、WASM 経由のクライアント、エージェントクライアントで使用されます。ゲームのソースデータは `data-src/` にあり、Cargo ビルド中に `data/` 内の生成済み JSON へ変換されます。サーバークレート（`server/`）、共有クレート、ソースデータの変更時だけサーバーを再ビルドするには、**ルートディレクトリ**で次の監視コマンドを実行します。

```bash
cargo watch -w server -w shared -w data-src -x "run -p onlinerpg-server"
```

サーバーは既定でポート 10006 をリッスンします。地形／ハウジング／NPC REST API は、127.0.0.1 にバインドされたポート 10007（ゲームポート + 1）で自動的に起動します（`--api-bind` で変更可能）。読み取りは公開されています。書き込み（PUT/POST/DELETE）には bearer token が必要です。ローカルスクリプト用の NPC token、またはメールアドレスが `ADMIN_EMAILS` / `--admin-emails`（カンマ区切り）に含まれる Google ID token を利用できます。マップエディターはログイン中のユーザーの token を自動的に送信します。

WebSocket と地形 API のプロキシは Vite 開発サーバーが処理するため（`client/vite.config.ts` を参照）、別途 socat や SSL プロキシを用意する必要はありません。

**Google ログイン**：ブラウザでのログインには Google OAuth を使用します。同じ Web クライアント ID をサーバー（`GOOGLE_CLIENT_ID` 環境変数 / `--google-client-id`）とクライアント（`VITE_GOOGLE_CLIENT_ID`、手順 4 を参照）の両方へ渡してください。指定しなくてもサーバーは動作しますが、ブラウザからのログインを拒否します。NPC／ボット用 token は初回実行時に `data/npc_token` へ自動生成されます。`NPC_AUTH_TOKEN` / `--npc-token` で上書きできます（16 文字以上）。

別の人のマシンで動作するエージェントクライアントは、自身の Google アカウントでデバイスフローを使ってログインします。そのため、「TV and Limited Input」種類の 2 つ目の OAuth クライアントが必要です（ヘッドレスクライアントでは Web 用のものを使用できません）。そのクライアント ID を `GOOGLE_CLI_CLIENT_ID` / `--google-cli-client-id` で渡してください。サーバーはどちらのクライアントからの token も受け入れます。[doc/REMOTE_AGENT_CLIENT.md](doc/REMOTE_AGENT_CLIENT.md) を参照してください。


### 4. クライアントの実行

```bash
cd client
cp .env.example .env.local   # then set VITE_GOOGLE_CLIENT_ID (required for login)
npm install
npm run dev -- --port 10004
```

### 5. エージェントクライアントの実行

`agent-client/data/config.toml` を編集して正しいポート番号を設定し、次を実行します。

```bash
cd agent-client
cargo watch -i "data/npcs/**/memory.txt" -i "data/npcs/**/favor.json" -x run
```

### 6. 共有コード変更時の WASM 自動再ビルド（推奨）
`shared` ライブラリ内の Rust コード変更をすぐにブラウザへ反映するには、別のターミナルで次のコマンドを実行します。

```bash
# Run from the root directory
cargo watch -w shared -s "npm run build:wasm --prefix client"
```

### 7. GLB エディターの実行

```bash
cd tools/glb-editor
npm install
npm run dev -- --port 10005
```

## 本番環境へのデプロイ

本番環境では 2 つのバイナリを systemd ユニット（`tools/systemd/`）として実行し、クライアントのビルド成果物を `/var/www/openmmo` から静的配信します。

| ユニット | バイナリ | Syslog 識別子 |
|------|--------|-------------------|
| `openmmo-server` | `onlinerpg-server` | `openmmo` |
| `openmmo-agent-client` | `agent-client` | `openmmo-agent` |

本番ホスト上で `tools/deploy-prod.sh` を実行してデプロイします。このスクリプトは master をプルし、両方のバイナリとクライアントバンドルをビルドし、静的ファイルを公開してから両方のユニットを再起動します。

ホストのセットアップスクリプトはありません。本番の nginx は手動管理の `/etc/nginx/sites-available/openmmo` で、キャッシュ規則の基準である `docker/nginx.conf.template` と揃えて保守します。特に `/models/` は `Cache-Control: no-cache` で配信する必要があります — オブジェクトカタログと GLB は固定パスで取得されるため、時間ベースの有効期限では古い `catalog.json` が新しい家具をエラーなしに隠してしまいます。

サーバーは systemd の `SIGTERM` を適切に処理します。接続中のプレイヤーへ再起動通知を表示し、リスナーと定期タスクを停止し、進行中の一括保存を待機して、接続中の全キャラクター、インベントリ、ワールド時計を永続化してから終了します。`systemctl restart` はこの終了処理を待ってから新しいバイナリを起動します。

管理者キャラクターは `/notice <message>` で同じバナーを手動表示できます（これはゲーム内のライブバナーであり、`data/announcements/` が提供するログイン画面のお知らせではありません）。メッセージなしの `/notice` でバナーを消去できます。バナーが有効な間に参加したプレイヤーにも、参加時に通知されます。

SSH 経由では、接続の切断によってビルドが途中で終了しないよう、セッションから切り離して実行します。

```bash
ssh prod 'setsid nohup bash ~/work/OnlineRPG/tools/deploy-prod.sh > ~/deploy-latest.log 2>&1 < /dev/null &'
ssh prod 'tail -f ~/deploy-latest.log'   # follow; ends at "==> deployed <commit>"
```

フォアグラウンド実行中に接続が切れるとビルド全体が無駄になりますが、不完全なデプロイ状態にはなりません。スクリプトは最初にすべてをビルドし、最後にだけ稼働中のファイルを更新するためです（`rsync` で Web ルートへ同期してから再起動）。それ以前に中断した場合は、対応の取れた古いバンドルと古いサーバープロセスがそのまま残ります。

### ログ

両方のユニットは journald（`StandardOutput=journal`）へ出力し、個別のログファイルはありません。

```bash
journalctl -u openmmo-server -f              # follow the game server
journalctl -u openmmo-agent-client -f        # follow the NPC agent client
journalctl -u openmmo-server -n 200 --no-pager   # recent history
journalctl -u openmmo-server --since "1 hour ago" -p err   # errors only
```

両方のバイナリは `RUST_LOG` から subscriber を構成し、未設定または解析できない場合は `info` が既定値です。モンスターの出現／消滅、戦闘のダイス／HP／XP 判定など、操作ごとの詳細は、高負荷時のログ量を抑えるため `debug` になっています。確認するにはログレベルを上げてください。

```bash
sudo systemctl edit openmmo-server   # or write /etc/openmmo/server.env
# [Service]
# Environment=RUST_LOG=debug
sudo systemctl restart openmmo-server
```

各ユニットは `EnvironmentFile=-/etc/openmmo/{server,agent-client}.env` を読み取るため、そこに `RUST_LOG` を設定できます。systemd ユニットはシェル環境を継承しない点に注意してください。ターミナルで `RUST_LOG` を export しても、そのターミナルから直接起動したバイナリにしか影響しません。

移動に関する警告（移動先の拒否、ウェイポイントキューの上限、移動の阻害）は意図的に `warn` のままです。これらはサーバーとクライアントのステップ検証が一致しないときに発生するため、通常動作ではなくバグの兆候です。

## コントリビューション

コントリビューションを歓迎します！作業の選び方、CI が実行するチェック、PR の慣例については [CONTRIBUTING.md](CONTRIBUTING.md) を参照してください。初めてプルリクエストを開くと、ボットが PR 上のコメントで [Contributor License Agreement](CLA.md) への署名を求めます。コントリビューションをマージするには CLA への署名が必要です。

バイナリアセット（3D モデル・音楽・効果音）のコントリビューションは [doc/ASSETS.md](doc/ASSETS.md) を参照してください — git ではなく Hugging Face データセットへの PR で行います。
