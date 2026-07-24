# claude-statusline

Claude Code の statusline コマンド（`~/.claude/statusline-command.sh` の Rust 移植版）。

表示内容:

```
~/projects/foo (feat-x) | CI:✔ Rv:👀 | [xhigh] Opus 4.7 | 50K/200K (25%) | 5h:45%[████░░░░] →Mon 17:00  7d:12%[█░░░░░░░] →Wed 09:00 | [Max]
```

- カレントディレクトリ（`~` 置換・長いパスは短縮）
- git ブランチ
- PR の CI / レビュー状態（ブランチに PR がある場合のみ）
- 推論 effort（`effort.level`、対応モデルのみ）+ モデル名
- コンテキストウィンドウ使用量
- 5h / 7d レート制限: 色付きバー（緑 <70% / 黄 <90% / 赤 >=90%）とリセット時刻
- アカウント種別（`claude auth status` を session_id 単位でキャッシュ）

### PR の CI / レビュー状態

現在のブランチに PR がある場合、`gh pr checks` / `gh pr view` の結果を表示する（`gh` の認証が必要）。

- `CI:✔`（緑）: 全チェック pass / `CI:●n`（黄）: 実行中 n 件 / `CI:✘n`（赤）: 失敗 n 件
- `Rv:👀`（黄）: レビュー待ち / `Rv:✔`（緑）: 承認済み / `Rv:✘`（赤）: 変更要求
- 「CI は通っていて残りはレビューだけ」の状態は `CI:✔ Rv:👀` として見える
- レビュー承認と連動して pending のままになるチェックは CI 判定から除外する。除外パターンは環境変数 `STATUSLINE_CI_IGNORE`（カンマ区切りの部分一致、デフォルト `validate-review`）で変更可能
- 結果は `~/.cache/claude-statusline/` に (ディレクトリ, ブランチ) 単位で 60 秒キャッシュし、バックグラウンドで更新するため statusline の描画はブロックしない
- main / master ブランチでは表示しない

## Setup

GitHub Releases のバイナリを mise（ubi バックエンド）でインストール:

```sh
mise use -g ubi:hiyamamo/claude-statusline
```

`~/.claude/settings.json`:

```json
{
  "statusLine": {
    "type": "command",
    "command": "~/.local/share/mise/shims/claude-statusline"
  }
}
```

ソースからインストールする場合は `cargo install --path .`（バイナリは `~/.cargo/bin/claude-statusline`）。

## Release

`v*` タグを push すると GitHub Actions が macOS (arm64 / x86_64) と Linux (x86_64) のバイナリをビルドしてリリースに添付する。

```sh
git tag v0.1.0
git push origin v0.1.0
```

## Development

```sh
# ビルドと動作確認
cargo build --release
echo '{"workspace":{"current_dir":"'$PWD'"},"model":{"display_name":"Fable 5"},"rate_limits":{"five_hour":{"used_percentage":45,"resets_at":1752480000}}}' \
  | ./target/release/claude-statusline
```

Rust ツールチェインは mise で管理（`mise.toml`）。
