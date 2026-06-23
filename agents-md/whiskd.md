<!-- agents-md: target=root priority=100 title="Whiskd process manager agent guide" -->

# Whiskd process manager agent guide

## Project purpose

`whiskd` is a lightweight process wrapper for agents and humans. Use it to start, monitor, attach to, log, and stop long-running local commands like dev servers, watchers, tunnels, and scripts.

GitHub repo: `https://github.com/Jeecabs/whiskd`

Core files:

- `whiskd` — Node.js CLI executable.
- `package.json` — package metadata for local checks; do not recommend running `whiskd` through `npx`.
- `README.md` — human usage docs.
- `install.sh` — copies `whiskd` to `~/bin`.

## Install / invoke from agents

Install the CLI before use:

```sh
npm install -g github:Jeecabs/whiskd
whiskd status
```

After npm publish, prefer:

```sh
npm install -g whiskd
```

Optional: install the agent skill with skills.sh:

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

## Agent workflow

Prefer `whiskd` for any command that may keep running or need logs later.

Use plain shell only for short commands that exit promptly.

### Start a background process

```sh
whiskd start --name api node server.js
whiskd start --name dev npm run dev
whiskd start --name web "npm run dev -- --host 0.0.0.0"
```

Rules:

- Always pass `--name` for important services. Stable names make later `logs`, `attach`, and `stop` safe.
- Quote the whole command as one string when shell operators, env expansion, pipes, redirects, or `&&` are required.
- For simple commands, separate args are fine: `whiskd start --name api node server.js`.

### Check process state

```sh
whiskd status
whiskd status --json
whiskd top
whiskd top --global
```

Use `whiskd status --json` when an agent needs machine-readable state. It includes names, status, cwd, pid, uptime, command, and log paths.

### Read logs

```sh
whiskd logs
whiskd logs api
whiskd logs api 100
```

Log guidance:

- Check logs after starting a process.
- Use explicit names when more than one process may exist.
- Use larger line counts only when needed.
- Logs live under `/tmp/whiskd-<uid>/.../output.log` and are capped at 10MB.

### Attach or inspect live output

```sh
whiskd attach api
whiskd top
```

TUI keys:

- `d` detaches and leaves the process running.
- `q` / `Ctrl+C` kills the attached process and exits.
- `p` pauses output.

Be careful: quitting an attached session kills the process unless detached.

### Stop processes

```sh
whiskd stop api
whiskd stop --all
```

Use `whiskd stop --all` only when the user clearly asked to stop every whiskd-managed process in the current context.

### Clean stale state

```sh
whiskd clean
```

Use after stopped processes no longer matter. Do not clean while debugging historical logs unless asked.

## Repo development checks

After changing the `whiskd` CLI, run focused smoke tests:

```sh
node --check whiskd
whiskd status --json
whiskd start --name whiskd-smoke "node -e 'setInterval(()=>console.log(Date.now()), 250)'"
whiskd logs whiskd-smoke 5
whiskd stop whiskd-smoke
```

If testing foreground/attach behavior, avoid leaving orphan processes:

```sh
whiskd start --name attach-smoke "node -e 'setInterval(()=>console.log("tick"), 500)'"
whiskd attach attach-smoke
whiskd stop attach-smoke
```

## Implementation notes

- State is per-user under `/tmp/whiskd-<uid>` with private permissions.
- `whiskd start` stores process metadata and captures output in `output.log`.
- Foreground `whiskd <cmd>` also spawns a managed process and attaches to it.
- Auto-naming derives names from common commands, but agents should still prefer explicit `--name`.
- Multi-argument commands are shell-quoted per arg. A single command string is passed raw to the shell.

## agents-md maintenance

This file is an `agents-md` source fragment. Do not hand-edit generated `AGENTS.md`.

Regenerate agent instructions with:

```sh
npx agents-md compose
```

Optional automation:

```sh
npx agents-md setup:compose-before-commit
```

Useful source syntax:

```md
<!-- agents-md: target=root priority=100 title="Section title" -->
<!-- agents-md: target=nearest -->
<!-- agents-md: target=../some-dir -->
<!-- agents-md: import=@./shared.md -->
```

Fragments discovered by default:

- `**/agents-md/**/*.md`
- `**/*.agents.md`

Generated targets are `AGENTS.md` files. Edit fragments, then run compose.
