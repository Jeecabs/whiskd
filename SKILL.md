---
name: whiskd-process-manager
description: Use whiskd to start, monitor, log, and stop long-running local processes (dev servers, watchers, tunnels, background scripts) while keeping logs inspectable. Use whenever a command should keep running in the background.
---

# Whiskd Process Manager

CLI for running long-running local commands in the background with captured logs.
The commands below are all an agent needs. whiskd also has interactive TUI views
(`attach`, `top`, bare `whiskd <cmd>`) — they block the terminal and are for humans,
not agents. Ignore them.

## Default behaviour

- `whiskd start --name X <cmd>` is **non-blocking**: spawns the process detached and
  returns in ~300ms. Use it for everything long-running.
- If the command dies within those ~300ms, `start` prints the exit reason and last output
  and exits non-zero. Later crashes aren't caught, so **read `logs` (or `status`) right
  after starting** to confirm the process is actually up.
- Use plain shell for short commands that exit on their own.

## Commands

```sh
# Start (non-blocking). Always pass --name for anything you'll log or stop later.
whiskd start --name api node server.js
whiskd start --name web "npm run dev -- --host 0.0.0.0"

# Inspect
whiskd status              # table of all processes
whiskd status --json       # machine-readable: name, status, pid, cwd, uptime, cmd, log path
whiskd logs api            # last 40 lines  (whiskd logs api 100 for more)

# Stop
whiskd stop api
whiskd stop --all          # only when the user asked to stop everything

# Remove stopped-process state (not while you still need old logs)
whiskd clean
```

`logs` prints and exits (no follow) — poll it again for fresh output.

## Naming & restart

- Without `--name`, whiskd auto-derives one from the command (`npm run dev → dev`), which
  collides across runs. Always name important services.
- **To restart, `stop` first.** `start --name api` while `api` is still running fails
  with "already running" — it never replaces the running process.
- `--name` / `-n` must come **before** the command; a later `-n` belongs to the command
  (`whiskd start --name t tail -n 50 app.log`).

## Quoting

Separate args are fine for simple commands. Quote the whole command as one string when you
need shell operators — pipes, redirects, `&&`, env expansion:

```sh
whiskd start --name api node server.js
whiskd start --name app "FOO=bar npm run build && npm start"
```

## Install

```sh
mkdir -p ~/bin
curl -fsSL "https://github.com/Jeecabs/whiskd/releases/latest/download/whiskd-$(uname -m)-$(uname -s | tr A-Z a-z).tar.gz" | tar -xz -C ~/bin
whiskd --version     # or: cargo install --git https://github.com/Jeecabs/whiskd
```

macOS and Linux only. (Skill only: `npx skills add Jeecabs/whiskd`.)
