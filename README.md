
<p align="center">
  <img src="assets/whiskd-logo.png" alt="whiskd tabby cat mascot" width="220">
</p>

# whiskd

[![CI](https://github.com/Jeecabs/whiskd/actions/workflows/ci.yml/badge.svg)](https://github.com/Jeecabs/whiskd/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Jeecabs/whiskd)](https://github.com/Jeecabs/whiskd/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

A tiny process wrapper for agents and humans. Start long-running commands in the background, read their logs, attach a live view, and stop them cleanly. It's a single binary with no daemon and no config. macOS and Linux.

## Install

```sh
mkdir -p ~/bin
curl -fsSL "https://github.com/Jeecabs/whiskd/releases/latest/download/whiskd-$(uname -m)-$(uname -s | tr A-Z a-z).tar.gz" | tar -xz -C ~/bin
```

Or `cargo install --git https://github.com/Jeecabs/whiskd`, or clone and run `./install.sh`. Make sure `~/bin` is on your `PATH`.

Agent skill: `npx skills add Jeecabs/whiskd`

Upgrading from 4.x or earlier (Node): stop running processes with the old version first, then `npm uninstall -g whiskd`.

## Quick start

```sh
whiskd start --name api node server.js   # background, returns immediately
whiskd logs api                          # last 40 lines
whiskd attach api                        # live view (d = detach, q = kill)
whiskd stop api

whiskd npm run dev                       # foreground with a status bar
```

## Commands

| Command | What it does |
|---|---|
| `whiskd <cmd...>` | Run in the foreground with a status bar |
| `whiskd start [--name N] <cmd...>` | Run in the background |
| `whiskd status [--json]` | List processes started from this directory |
| `whiskd logs [name] [n]` | Print the last `n` lines (default 40) |
| `whiskd attach [name]` | Live output view of a running process |
| `whiskd top [--global]` | Dashboard of running processes (`-g`: all directories) |
| `whiskd stop [name \| --all]` | Stop one process, or all in this directory |
| `whiskd clean` | Remove stopped process state in this directory |

When only one process exists, the name can be omitted.

**Foreground / attach keys:** `d` detach · `p` pause · `q`/`Ctrl+C` kill (press again to force)

**Top keys:** `↑↓`/`jk` select · `Enter`/`a` attach · `s` stop · `l` logs · `g` global/local · `q` quit

## How it works

- **No daemon.** Each command reads state from disk and exits. Processes run in their own session, so closing the terminal or killing whiskd never takes them down.
- **State** lives in `/tmp/whiskd-<uid>/<cwd-hash>/<name>/` (`output.log`, `pid`, `cmd`, `started`, `cwd`). The directory is private to your user and scoped per directory, so a `web` process in one repo never collides with a `web` in another.
- **Names** come from `--name`, or are derived from the command (`npm run dev` → `dev`, `node server.js` → `server`, `go run .` → directory name). A derived name gets a `-2` suffix if taken. An explicit name that is already running is an error. `--name` must come before the command.
- **Commands** run via `/bin/sh -c`. A single quoted string is passed raw, so `"a && b"` works. Multiple arguments are quoted one by one, so the grouping your shell resolved survives.
- **Startup failures:** `start` waits ~300ms. If the command dies in that window, whiskd prints the reason and the last output, then exits with the command's code.
- **Stopping** sends SIGTERM to the whole process group and SIGKILL to anything still alive after 2s. First it checks that the PID still belongs to the process whiskd started, so a reused PID is never killed by mistake.
- **Logs** are capped at 10MB and get an exit footer (code or signal). A death discovered later is recorded as `exited (status unknown)`. `logs` and `top` strip ANSI colors; `attach` shows them raw. Stopped state is pruned after 7 days.

## Development

```sh
cargo test && ./test.sh
```

Pushing a `v*` tag builds release binaries.

## License

MIT
