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

## TUI keys

| Key | Action |
|-----|--------|
| `q` / `Ctrl+C` | Kill process and exit |
| `d` | Detach (leave running in background) |

## Auto-naming

Process names are derived from commands automatically:

| Command | Name |
|---------|------|
| `npm run dev` | `dev` |
| `node server.js` | `server` |
| `python app.py` | `app` |

Use `--name` / `-n` to override.

## Logs

Logs are stored in `/tmp/roo/<cwd>/<name>/output.log`. ANSI codes are stripped from log files. Logs are capped at 10MB and old process dirs are pruned after 7 days.
