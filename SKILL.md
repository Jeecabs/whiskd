---
name: whiskd-process-manager
description: Use whiskd to run, monitor, attach to, log, and stop long-running local processes for agents and humans. Use when starting dev servers, watchers, tunnels, background scripts, or any command that should keep running while logs remain inspectable.
---

# Whiskd Process Manager

## Install / run

Install the CLI before use:

```sh
npm install -g github:Jeecabs/whiskd
whiskd status
```

After npm publish, prefer:

```sh
npm install -g whiskd
```

Install this skill separately if needed:

```sh
npx skills add Jeecabs/whiskd
```

Local checkout install:

```sh
git clone https://github.com/Jeecabs/whiskd.git
cd whiskd
./install.sh
whiskd status
```

Do not run `whiskd` via `npx github:Jeecabs/whiskd ...`. `whiskd` manages long-running processes and needs a stable local binary/path for reliable monitoring, attach, and stop behavior.

## Agent rules

Prefer `whiskd` for long-running commands:

- dev servers: `npm run dev`, `pnpm dev`, `vite`, `next dev`
- watchers
- local APIs
- tunnels
- scripts where logs may matter later

Use plain shell only for short commands that exit promptly.

## Common commands

```sh
# Start named background process
whiskd start --name api node server.js
whiskd start --name dev npm run dev
whiskd start --name web "npm run dev -- --host 0.0.0.0"

# Inspect state
whiskd status
whiskd status --json
whiskd top
whiskd top --global

# Logs
whiskd logs dev
whiskd logs dev 100

# Attach
whiskd attach dev

# Stop
whiskd stop dev
whiskd stop --all

# Clean stopped state
whiskd clean
```

## Naming guidance

Always pass `--name` for important services. Stable names make later `logs`, `attach`, and `stop` safe.

## Quoting guidance

Separate args are fine for simple commands:

```sh
whiskd start --name api node server.js
```

Quote the whole command as one string when shell operators, env expansion, pipes, redirects, or `&&` are needed:

```sh
whiskd start --name app "FOO=bar npm run build && npm start"
```

## Attach safety

In attached TUI sessions:

- `d` detaches and leaves process running.
- `q` / `Ctrl+C` kills process and exits.
- `p` pauses output.

Be careful: quitting attached sessions kills the process unless detached.

## Smoke test

```sh
node --check whiskd
whiskd status --json
whiskd start --name whiskd-smoke "node -e 'setInterval(()=>console.log(Date.now()), 250)'"
whiskd logs whiskd-smoke 5
whiskd stop whiskd-smoke
```
