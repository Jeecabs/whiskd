---
name: roo-process-manager
description: Use roo to run, monitor, attach to, log, and stop long-running local processes for agents and humans. Use when starting dev servers, watchers, tunnels, background scripts, or any command that should keep running while logs remain inspectable.
---

# Roo Process Manager

## Install / run

Install the skill:

```sh
npx skills add Jeecabs/roo
```

Install the CLI before use:

```sh
git clone https://github.com/Jeecabs/roo.git
cd roo
./install.sh
roo status
```

Do not run `roo` via `npx github:Jeecabs/roo ...`. `roo` manages long-running processes and needs a stable local binary/path for reliable monitoring, attach, and stop behavior.

## Agent rules

Prefer `roo` for long-running commands:

- dev servers: `npm run dev`, `pnpm dev`, `vite`, `next dev`
- watchers
- local APIs
- tunnels
- scripts where logs may matter later

Use plain shell only for short commands that exit promptly.

## Common commands

```sh
# Start named background process
roo start --name api node server.js
roo start --name dev npm run dev
roo start --name web "npm run dev -- --host 0.0.0.0"

# Inspect state
roo status
roo status --json
roo top
roo top --global

# Logs
roo logs dev
roo logs dev 100

# Attach
roo attach dev

# Stop
roo stop dev
roo stop --all

# Clean stopped state
roo clean
```

## Naming guidance

Always pass `--name` for important services. Stable names make later `logs`, `attach`, and `stop` safe.

## Quoting guidance

Separate args are fine for simple commands:

```sh
roo start --name api node server.js
```

Quote the whole command as one string when shell operators, env expansion, pipes, redirects, or `&&` are needed:

```sh
roo start --name app "FOO=bar npm run build && npm start"
```

## Attach safety

In attached TUI sessions:

- `d` detaches and leaves process running.
- `q` / `Ctrl+C` kills process and exits.
- `p` pauses output.

Be careful: quitting attached sessions kills the process unless detached.

## Smoke test

```sh
node --check roo
roo status --json
roo start --name roo-smoke "node -e 'setInterval(()=>console.log(Date.now()), 250)'"
roo logs roo-smoke 5
roo stop roo-smoke
```
