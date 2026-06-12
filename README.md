# roo

Lightweight process wrapper with a TUI status bar. Run, background, attach, and manage long-running commands with automatic log capture.

## Install

```sh
./install.sh
```

Copies `roo` to `~/bin`. Make sure `~/bin` is in your `PATH`:

```sh
export PATH=$HOME/bin:$PATH
```

Requires Node.js.

## Usage

```sh
# Run with TUI (foreground)
roo npm run dev

# Run in background
roo start npm run dev

# Named process
roo start --name api node server.js

# Check what's running
roo status
roo status --json

# Live dashboard
roo top
roo top --global    # all directories

# View logs (last 40 lines by default)
roo logs
roo logs api 100

# Attach to a background process
roo attach api

# Stop
roo stop api
roo stop --all

# Clean up stopped process dirs
roo clean
```

## TUI keys (foreground/attach)

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Kill process and exit |
| `d` | Detach (leave running in background) |
| `p` | Pause/unpause output |

## Top keys

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Quit top |
| `↑`/`↓`/`j`/`k` | Select process |
| `Enter` / `a` | Attach to selected process |
| `s` | Stop selected process |
| `l` | View logs of selected process |
| `g` | Toggle global/local view |

## Auto-naming

Process names are derived from commands automatically:

| Command | Name |
|---------|------|
| `npm run dev` | `dev` |
| `node server.js` | `server` |
| `python app.py` | `app` |

Use `--name` / `-n` to override.

## Logs

Logs are stored in `/tmp/roo-<uid>/<cwd>/<name>/output.log` (a private, per-user directory with `0700` permissions; state files and logs are `0600`).

Log files keep ANSI color codes; `roo logs` and top's log pane strip them for display, while `attach` and the foreground TUI show them raw (it's a live terminal view). The foreground TUI runs commands with `FORCE_COLOR=1`; `roo start` uses `FORCE_COLOR=0`.

Logs are capped at 10MB — enforced when a process exits and whenever any roo command runs, and checked continuously while `top` or `attach` are open. Old process dirs are pruned after 7 days. When roo discovers a death after the fact (the usual case for `roo start`), it appends an `exited (status unknown)` footer to the log.

## Command quoting

Multi-argument commands are quoted per-argument, so `roo echo 'a  b'` preserves the grouping your shell resolved, and env prefixes like `FOO='x y' cmd` stay assignments. A single-argument command string is passed raw to the shell:

```sh
roo start "npm run build && npm start"   # shell operators work
roo printf '%s\n' 'a b' c                # arguments survive intact
```

## Changed in 2.0

- **State moved** from the shared `/tmp/roo` to per-user `/tmp/roo-<uid>` (also reflected in `status --json`'s `log` field). Processes started by 1.x are invisible to 2.x: stop them with the old binary (or `kill` them manually), then `rm -rf /tmp/roo`.
- **Foreground mode spawns like `start`** and attaches to its own process. The child owns its log file, so detaching — or roo itself being killed — leaves it running and attachable. Trade-offs: stderr is no longer tinted red, output is displayed with up to ~100ms latency, and log files now retain ANSI (stripped on display).
- **Multi-arg commands are shell-quoted per-arg** (see above). `roo echo a && echo b` no longer leaks `&&` to the inner shell — quote the whole thing as one argument if you want shell semantics.
