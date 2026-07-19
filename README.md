# claude-statusline

Claude Code の statusline コマンド（`~/.claude/statusline-command.sh` の Rust 移植版）。

表示内容:

```
~/projects/foo (main) | [Max] Fable 5 | 50K/200K (25%) | 5h:45%[████░░░░] →Mon 17:00  7d:12%[█░░░░░░░] →Wed 09:00
```

- カレントディレクトリ（`~` 置換・長いパスは短縮）
- git ブランチ
- アカウント種別（`claude auth status` を session_id 単位でキャッシュ）+ モデル名
- コンテキストウィンドウ使用量
- 5h / 7d レート制限: 色付きバー（緑 <70% / 黄 <90% / 赤 >=90%）とリセット時刻

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
