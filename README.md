
<p align="center">
  <img src="assets/whiskd-logo.png" alt="whiskd tabby cat mascot" width="220">
</p>

# whiskd

[![CI](https://github.com/Jeecabs/whiskd/actions/workflows/ci.yml/badge.svg)](https://github.com/Jeecabs/whiskd/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Jeecabs/whiskd)](https://github.com/Jeecabs/whiskd/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

whiskd is a tiny, fast process wrapper for agents and humans. Start long-running commands in the background, read their logs later, attach a live terminal view, and stop them cleanly. It needs no daemon and no config file.

It's a single ~450KB Rust binary with one dependency (`libc`). It works with any command: Node, Python, Go, Rust, shell scripts, tunnels, watchers.

## Why whiskd

- **Built for agents**: `whiskd start` returns in ~300ms and exits non-zero if the command crashes on boot. `status --json` gives machine-readable state, and `logs` prints and exits.
- **No daemon**: each command reads state from disk and exits. Processes run in their own session, so closing the terminal or killing whiskd never takes them down.
- **Fast**: about 5ms per command and under 2MB of memory (see [Performance](#performance)).
- **Safe by default**: state lives in a private per-user directory (`0700`, files `0600`), with checks against symlinks and wrong owners. Before killing, whiskd checks that a PID still belongs to the process it started, so a reused PID is never killed by mistake. Escape sequences in logs are scrubbed before display.
- **Human-friendly TUI**: a status bar for foreground runs and `attach`, plus a `top` dashboard across every project on the machine.

## Features

- Background (`start`) and foreground runs, auto-named from the command or named with `--name`
- `attach` / detach from a live output view with a pinned status bar, pause, and kill
- `top`: live dashboard of running processes, local or global, with attach, stop and log views
- Per-process logs capped at 10MB, with an exit footer (code or signal) written on death
- Deaths discovered after the fact, even when nobody was watching
- Tree kill: SIGTERM the whole process group, SIGKILL whatever survives after 2s
- Names are scoped per directory, so `web` in two repos never collide
- Stopped state is pruned after 7 days

## Install

Requires macOS or Linux.

### Prebuilt binary

```sh
mkdir -p ~/bin
curl -fsSL "https://github.com/Jeecabs/whiskd/releases/latest/download/whiskd-$(uname -m)-$(uname -s | tr A-Z a-z).tar.gz" | tar -xz -C ~/bin
whiskd --version
```

Binaries cover macOS (arm64, x86_64) and Linux (x86_64, aarch64; statically linked). Make sure `~/bin` is on your `PATH`.

### Cargo

```sh
cargo install --git https://github.com/Jeecabs/whiskd
```

### Build from source

```sh
git clone https://github.com/Jeecabs/whiskd.git
cd whiskd
./install.sh    # cargo build --release, installs to ~/bin/whiskd
```

### Agent skill

```sh
npx skills add Jeecabs/whiskd
```

This installs [`SKILL.md`](SKILL.md), which teaches coding agents the non-interactive subset of the CLI.

## Quick start

```sh
whiskd start --name api node server.js     # background, returns immediately
whiskd logs api                            # last 40 lines
whiskd status                              # everything in this directory
whiskd attach api                          # live view (d = detach, q = kill)
whiskd stop api
```

Run in the foreground with a status bar:

```sh
whiskd npm run dev
```

## Commands

| Command | What it does |
|---|---|
| `whiskd <cmd...>` | Run in the foreground with the TUI, auto-named |
| `whiskd start [--name N] <cmd...>` | Run in the background |
| `whiskd status [--json]` | List processes started from this directory |
| `whiskd logs [name] [n]` | Print the last `n` lines (default 40) |
| `whiskd attach [name]` | Live output view of a running process |
| `whiskd top [--global]` | Dashboard of running processes (`-g` for all directories) |
| `whiskd stop [name \| --all]` | Stop one process, or all in this directory |
| `whiskd clean` | Remove stopped process state in this directory |

When only one process exists, the name can be omitted.

`status --json` fields: `name`, `cwd`, `status`, `pid`, `started`, `cmd`, `log`, `uptime`.

### TUI keys (foreground / attach)

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Kill the process and exit (press again to force) |
| `d` | Detach and leave it running |
| `p` | Pause/unpause output |

### Top keys

| Key | Action |
|-----|--------|
| `↑`/`↓`/`j`/`k` | Select process (scroll in the log view) |
| `Enter` / `a` | Attach to the selected process (`q`/`Esc` returns to top) |
| `s` | Stop the selected process |
| `l` | View logs of the selected process |
| `g` | Toggle global/local view |
| `q` / `Ctrl+C` | Quit top |

## Naming

Names are derived from the command when `--name` / `-n` isn't given:

| Command | Name |
|---------|------|
| `npm run dev` / `yarn dev` / `bun run dev` | `dev` |
| `node --watch server.js` | `server` |
| `python3 app.py` | `app` |
| `go run .` | directory name |

Auto-derived names get a `-2` suffix if taken. An explicit `--name` that is already running is an error, because agents rely on stable names for `logs` and `stop`. The flag must come **before** the command, so a later `-n` belongs to the command (`whiskd start --name t tail -n 50 app.log`).

## Command quoting

Multi-argument commands are quoted per argument, so the grouping your shell resolved survives, and env prefixes like `FOO='x y' cmd` stay assignments. A single-argument command string is passed raw to `/bin/sh -c`, so shell operators work:

```sh
whiskd start "npm run build && npm start"   # shell operators work
whiskd printf '%s\n' 'a b' c                # arguments survive intact
```

## Startup failures

`whiskd start` waits ~300ms. If the command exits in that window, whiskd prints the exit reason and the last 10 lines of output, then exits with the command's code (128+signal if it was killed). Later crashes are recorded in the log footer and shown by `status`.

## Logs and state

State lives in `/tmp/whiskd-<uid>/<cwd-hash>/<name>/`: `output.log`, `pid`, `cmd`, `started`, `cwd`. The directory is private to your user. `/tmp` is used on purpose instead of `$TMPDIR`, which on macOS differs between GUI and SSH sessions.

- Log files keep ANSI colors. `logs` and top's log view strip them; `attach` and the foreground TUI show them raw. Foreground runs set `FORCE_COLOR=1` and `start` sets `FORCE_COLOR=0`.
- Logs are capped at 10MB. The cap is enforced on exit, on every whiskd command, and continuously while `top` or `attach` is open.
- When whiskd discovers a death after the fact (the usual case for `start`), it appends an `exited (status unknown)` footer.

## Performance

Mean of 30 runs per command on an Apple M4 Pro, with two managed processes running:

| Command | whiskd 5 (Rust) | whiskd 4 (Node 24) |
|---|---|---|
| `status` | 5.4 ms | 26.2 ms |
| `status --json` | 5.4 ms | 26.1 ms |
| `logs api` | 5.3 ms | 26.2 ms |
| Peak memory (`status`) | 1.8 MB | 51 MB |

`start` takes ~300ms in both versions. That's the deliberate window for catching boot crashes, not overhead.

## Upgrading from 4.x and earlier (Node)

whiskd 5 is a Rust rewrite with the same commands and keys. Its state layout differs, so processes started by an older version aren't visible to it. Stop them with the old binary first. If you installed the old version through npm, remove it with `npm uninstall -g whiskd`.

## Development

```sh
cargo test          # unit tests for quoting, naming, parsing
./test.sh           # integration checks in a throwaway directory
cargo clippy -- -D warnings
```

Releases are built by GitHub Actions when a `v*` tag is pushed.

## License

MIT
