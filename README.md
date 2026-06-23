# whiskd

<p align="center">
  <img src="assets/whiskd-logo.png" alt="whiskd tabby cat mascot" width="220">
</p>

Lightweight process wrapper with a TUI status bar. Run, background, attach, and manage long-running commands with automatic log capture.

## Install

Install the agent skill with skills.sh:

```sh
npx skills add Jeecabs/whiskd
```

Install the CLI locally to `~/bin`:

```sh
git clone https://github.com/Jeecabs/whiskd.git
cd whiskd
./install.sh
```

Make sure `~/bin` is in your `PATH`:

```sh
export PATH=$HOME/bin:$PATH
```

Requires Node.js 18+.

Do not run `whiskd` via `npx github:Jeecabs/whiskd ...`. `whiskd` manages long-running processes and needs a stable local binary/path for reliable monitoring, attach, and stop behavior.

## Usage

```sh
# Run with TUI (foreground)
whiskd npm run dev

# Run in background
whiskd start npm run dev

# Named process
whiskd start --name api node server.js

# Check what's running
whiskd status
whiskd status --json

# Live dashboard
whiskd top
whiskd top --global    # all directories

# View logs (last 40 lines by default)
whiskd logs
whiskd logs api 100

# Attach to a background process
whiskd attach api

# Stop
whiskd stop api
whiskd stop --all

# Clean up stopped process dirs
whiskd clean
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

Logs are stored in `/tmp/whiskd-<uid>/<cwd>/<name>/output.log` (a private, per-user directory with `0700` permissions; state files and logs are `0600`).

Log files keep ANSI color codes; `whiskd logs` and top's log pane strip them for display, while `attach` and the foreground TUI show them raw (it's a live terminal view). The foreground TUI runs commands with `FORCE_COLOR=1`; `whiskd start` uses `FORCE_COLOR=0`.

Logs are capped at 10MB — enforced when a process exits and whenever any whiskd command runs, and checked continuously while `top` or `attach` are open. Old process dirs are pruned after 7 days. When whiskd discovers a death after the fact (the usual case for `whiskd start`), it appends an `exited (status unknown)` footer to the log.

## Command quoting

Multi-argument commands are quoted per-argument, so `whiskd echo 'a  b'` preserves the grouping your shell resolved, and env prefixes like `FOO='x y' cmd` stay assignments. A single-argument command string is passed raw to the shell:

```sh
whiskd start "npm run build && npm start"   # shell operators work
whiskd printf '%s\n' 'a b' c                # arguments survive intact
```

## Changed in 3.0

- **Renamed to `whiskd`** across the package, CLI binary, docs, agent skill, log prefixes, and state directory.
- **State lives under `/tmp/whiskd-<uid>`**. Existing process state under older names is not read.

## Changed in 2.0

- **Foreground mode spawns like `start`** and attaches to its own process. The child owns its log file, so detaching — or whiskd itself being killed — leaves it running and attachable. Trade-offs: stderr is no longer tinted red, output is displayed with up to ~100ms latency, and log files now retain ANSI (stripped on display).
- **Multi-arg commands are shell-quoted per-arg** (see above). `whiskd echo a && echo b` no longer leaks `&&` to the inner shell — quote the whole thing as one argument if you want shell semantics.
