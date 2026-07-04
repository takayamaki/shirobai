# shirobai

shirobaiは、[RuboCop gem](https://github.com/rubocop/rubocop)の一部ルールを
完全互換なRust実装で置き換えることでドロップインでの高速化の可能性を探る試験的gemです。

[English version: README.md](README.md)

> [!WARNING]
> このgemはexperimentalです。RuboCopとの互換性を可能な限り重視していますが、productionコードへ適用した際の不利益については保証しません。

## shirobaiの特徴と存在意義、理念
### RuboCopドロップインであること
formatter、linterの高速化というと全てを新しいインターフェイスで再実装しがちなイメージがありますが、shirobaiはあくまでRuboCopを主人として、
各ルールでASTのwalkなど重くなりがちな箇所をRustで再実装することでその高速化に貢献することを目的とします。

RuboCopの膨大なエコシステム、そして必要に応じて開発者自身がルールを書けるRuboCopのインターフェイスや理念に敬意を抱いており、敵対する意思はありません。

### RuboCop完全互換であること
上記の理念から、shirobaiはRuboCopの各copのspecで担保されている挙動を絶対の正典として扱います。

また、実用時の互換性を担保するために下記リポジトリそれぞれのRuboCop configを用いて実際にlintを行い、その結果が本家RuboCopと相違ないことをテストしています。

- RuboCop
- [Mastodon](https://github.com/mastodon/mastodon)
- [Discourse](https://github.com/discourse/discourse)
- [Redmine](https://github.com/redmine/redmine)
- [fluentd](https://github.com/fluent/fluentd)

また再実装時にspecで担保されているべきだがされていない疑いがある挙動については本家RuboCopへのcontributeもできたら…という展望がなくもないかもしれない。


### このgemの名前について
shirobaiは日本語話者なら当然わかるでしょう、白バイです。警官であるRuboCopが乗るだけで速くなる、をイメージして命名しました。

## 現状

- **76 cop** を Rust 実装済み（Lint / Layout / Metrics / Naming / Style）。
- **drop-in 完全互換**を実コーパスで検証済み。
  検証は `benches/parity_diff.sh` で行う。実装済み全 cop について offense
  の位置・メッセージ・autocorrect 後のバイトすべてが stock RuboCop と一致する。
  PENDING autocorrect は許容しない。完全互換に到達できない cop は ship しない方針。
- **実プロジェクトでの速度** — 実 CLI、各プロジェクトの `.rubocop.yml`、
  plugin gem 込み、5 round 中央値:

  | コーパス | files | offenses | stock | shirobai | 削減 |
  |---|---|---|---|---|---|
  | Mastodon | 3,206 | 0 | 116.25s | 90.57s | **-25.69s (-22.1%)** |
  | Discourse | 10,229 | 16 | 259.56s | 237.86s | **-21.70s (-8.4%)** |
  | Redmine | 1,058 | 2 | 56.73s | 43.24s | **-13.49s (-23.8%)** |
  | fluentd | 456 | 0 | 9.73s | 9.97s | +0.24s (+2.5%) |

  計測環境: GitHub Actions `ubuntu-latest`（4-vCPU 共有 runner）、
  shirobai は [`84b6906`](https://github.com/takayamaki/shirobai/commit/84b6906) 時点。
  各実行はまず stock と shirobai が **同じ offense 集合** を報告することを検証してから、
  同じコードを lint する中央値時間を測る。
  任意のコミットで再実行するには `gh workflow run bench.yml`
  （`.github/workflows/bench.yml`）。

  shirobai が置き換えるのは rubocop gem 本体の cop のみ。
  plugin の cop（rubocop-rails、rubocop-rspec 等）はそのまま動くため、
  plugin cop の比重が大きいプロジェクトほど削減率は小さくなる
  （Discourse は plugin 依存が大きい）。
  fluentd は config でほとんどの default cop が無効化されており、
  shirobai が置き換える対象がほとんど残らないため、
  native extension の固定ロードコストが削減分をわずかに上回る。

  RuboCop 自身も互換検証には使っているがベンチには含めていない。
  config が `rubocop-internal_affairs` / `rubocop-rake` を要求し、
  かつ rubocop gem 本体の cop がほとんど有効化されていないため。

## 前提条件

> [!IMPORTANT]
> shirobai のネイティブ拡張は Rust で書かれています。
> `bundle install` 時に `cargo build --release` が走るため、**Rust toolchain（stable, 1.75 以上）** がインストール先ホストに必要です。
> [rustup](https://rustup.rs/) 等で事前にインストールしてください。

| | |
|---|---|
| RuboCop | **`= 1.88.0` で hard pin** |
| Ruby | `>= 3.1` |
| Rust | `>= 1.75`（stable） |
| プラットフォーム | Linux / macOS（`cargo build --release` が通れば動く） |
| Ruby パーサ | `ruby-prism`（Latest 文法 ≈ Ruby 4.1） |

RuboCop の hard pin は意図的なもの。shirobai は cop の内部挙動をバイト単位
で写しているため、stock の minor 更新でも cop の挙動が微妙にずれうる。黙
って divergence を出すよりインストール時に失敗してほしいので、bump は手動・
意識的に行う。

### 既知の制約: `AllCops/TargetRubyVersion`

shirobai は常に prism の Latest 文法でパースする。実コーパスで影響が出るの
は **Layout/SpaceAroundKeyword** が Ruby 2.7 の `expr in pat`（ワンライン
パターンマッチ）を検出するケースだけで、他の実装済み cop は検証コーパスの
設定下では target version に依存しない。この 1 cop について厳密に TargetRuby
を効かせたいときは、設定でその cop だけ shirobai 差し替えを無効化すれば
stock がそのまま動く。

## インストール

Gemfile で `rubocop` の隣に追加する:

```ruby
gem "rubocop", "= 1.88.0"
gem "shirobai"
```

その後 `bundle install` を実行する。

## 使い方

`.rubocop.yml` に `require` を追加する:

```yaml
require:
  - shirobai
```

これだけ。`shirobai/inject.rb` が Rust 実装の各 cop を stock cop と同じ
badge で registry に登録するので、RuboCop 側の cop registry、設定解決、
disable コメント、`--only` / `--except`、`--auto-correct`、ResultCache 等
は何ひとつ変わらず動く。`require:` 以外の `.rubocop.yml` 変更は不要。

## 仕組み

```
┌───────────────────────────────────────────────────────────────────┐
│ RuboCop（Ruby フロントエンド）                                    │
│   Runner → Team → Commissioner → cop インスタンス（ファイル毎）   │
└───────────────────────────────────────────────────────────────────┘
                          │
                          │ Rust 実装の cop が
                          │ stock と同じ badge で登録される
                          ▼
┌───────────────────────────────────────────────────────────────────┐
│ lib/shirobai/cop/<dept>/<name>.rb（Ruby wrapper）                 │
│   - Rust のタプル結果を Parser::Source::Range・offense・           │
│     corrector 呼び出しに変換                                       │
│   - 非 ASCII offset には Shirobai::SourceOffsets を適用            │
│     （prism=byte / parser gem=char の単位差を吸収）                │
└───────────────────────────────────────────────────────────────────┘
                          │
                          │ Dispatch がファイル毎に 1 パス起動
                          ▼
┌───────────────────────────────────────────────────────────────────┐
│ crates/shirobai-core（Rust）                                      │
│   - prism ベースの shared walk: 1 回の AST 走査で全 cop 分の       │
│     解析結果をまとめて生成（rules/bundle.rs）                      │
│   - 各 cop は build_rule() で Rule 実装を公開し、standalone と     │
│     bundle で同一ロジックを駆動（コピー禁止、cargo test で同値担保）│
│ ext/shirobai（magnus ブリッジ）: check_all_bundle を Ruby に公開    │
└───────────────────────────────────────────────────────────────────┘
```

押さえどころ:

- **shared walk**: `Shirobai.check_all(src, token)` がファイル毎に 1 回の
  prism 走査を行い、全 Rust cop の解析結果を一括で生成する。cop を 1 個
  増やしても別個の全ファイル走査は発生しない。
- **同一ロジック・二系統ドライバ**: 各 Rust rule は `build_rule()` で公開
  され、standalone（per-cop fallback）と bundle（shared walk）は同じ実装
  を共有する。等価性は `cargo test` が守る。
- **badge 差し替えによる drop-in**: `inject.rb` の `registry.enlist(klass)`
  により Rust 実装の cop が stock cop と同じ registry スロットを占める。
  RuboCop 側からは stock cop と区別がつかない。

## リポジトリ構成

各ディレクトリに詳細を記した `README.md` がある。

| ディレクトリ | 内容 |
|---|---|
| `lib/shirobai/` | Ruby wrapper、Dispatch、SourceOffsets、inject |
| `crates/shirobai-core/` | Rust 解析コア（per-cop rule + shared walk） |
| `ext/shirobai/` | magnus ブリッジ（cdylib） |
| `benches/` | ベンチマークと parity オラクル |
| `spec/` | RSpec、vendor spec 取り込み、エッジケース parity |
| `vendor/rubocop/` | git submodule、1.88.0 を pin（vendor spec 用） |

## ビルドとテスト

```sh
bundle install
bundle exec rake compile          # cargo build --release + .so を lib/ にコピー
bundle exec rspec                 # Ruby 側: vendor spec + parity spec
cargo test                        # Rust 側: rule 等価性と単体テスト
cargo clippy --all-targets        # 新規警告ゼロをマージ基準とする
```

### Parity チェック（drop-in 互換検証）

まずテスト用コーパスを clone する:

```sh
bin/setup-corpora
```

Mastodon、Discourse、Redmine、fluentd を `.tmp/` に pin 済みコミットで clone する。
`rubocop_source` は `vendor/rubocop` へのシンボリックリンク（git で追跡済み）。

各コーパスに対して parity オラクルを実行する:

```sh
benches/parity_diff.sh .tmp/mastodon
benches/parity_diff.sh .tmp/discourse
benches/parity_diff.sh .tmp/redmine
benches/parity_diff.sh .tmp/fluentd
benches/parity_diff.sh .tmp/rubocop_source
```

各実行では実 `rubocop` CLI を 2 回走らせる
— 一度は `Gemfile.stock`（shirobai なし）、もう一度は `Gemfile.with_shirobai` で —
per-cop / per-offense (`path:line:column:message`) を diff する。
**5 コーパス全部で diff=0 がマージの必須条件**。

### 速度ベンチマーク

```sh
benches/run_e2e.sh .tmp/mastodon 3
```

Mastodon の `.rubocop.yml` を使って in-process で速度を計測する
（cop のオン/オフとパラメータを反映する。plugin gem のインストールは不要）。
各 round で 3 つのモードを実行する:

- **stock** — 全 default cop をそのまま実行
- **removed** — 実装済み cop を全部外した状態（速度の下限）
- **shirobai** — 実装済み cop を Rust 実装に差し替えた状態（実効速度）

スクリプトは compute/cpu/gc の中央値と net win をまとめて出力する。

## Claude Code エージェント向け

このリポジトリは Claude Code で開発されている。プロジェクトルールは
[`.claude/CLAUDE.md`](.claude/CLAUDE.md) を参照。

## ライセンス

MIT。[LICENSE.txt](LICENSE.txt) を参照。
