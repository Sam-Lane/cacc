# cacc

Tongue in cheek named tool for swapping between multiple Claude Code accounts.
A small Rust take on [claude-swap](https://github.com/realiti4/claude-swap).

## Install

```bash
brew install sam-lane/tap/cacc
```

## Usage

```bash
cacc add main        # save the account Claude Code is logged in to
cacc login work      # log in to another account and save it
cacc list            # saved accounts, * marks the active one
cacc switch          # pick an account with fzf (numbered prompt without fzf)
cacc set work        # switch by name, email or number
cacc usage           # 5-hour / 7-day usage for every account
```

Restart running Claude Code sessions after switching.

## Where things are stored

- macOS: tokens in the Keychain (service `cacc`)
- Linux: `~/.config/cacc/creds/`, mode 0600
- Account list: `~/.config/cacc/accounts.json`

`cacc usage` uses Anthropic's undocumented OAuth usage endpoint, so it may break without notice.
