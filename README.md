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

Logs are stored in `/tmp/roo-<uid>/<cwd>/<name>/output.log` (a private, per-user directory with `0700` permissions). ANSI codes are stripped from log files. Logs are capped at 10MB and old process dirs are pruned after 7 days.

### Migrating from 1.x

1.x stored state in the shared `/tmp/roo` tree, which 2.x no longer reads — processes started by an old roo are invisible to the new one. Stop them with the old binary (or `kill` them manually), then remove the old tree:

```sh
rm -rf /tmp/roo
```
