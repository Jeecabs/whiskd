#!/bin/bash
# Integration checks against the release build. Runs in a throwaway cwd so
# real state is untouched.
set -u
ROOT=$(cd "$(dirname "$0")" && pwd)
cargo build --release --quiet --manifest-path "$ROOT/Cargo.toml" || exit 1
W="$ROOT/target/release/whiskd"
T=$(cd "$(mktemp -d)" && pwd -P); mkdir -p "$T/web"; cd "$T"
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
trap '$W stop --all >/dev/null 2>&1; $W clean >/dev/null; (cd web && $W stop --all >/dev/null 2>&1; $W clean >/dev/null); rm -rf "$T"' EXIT

$W start tail -n 3 /etc/hosts >/dev/null 2>&1
check "-n after the command belongs to the command" '$W status | grep -q "cmd=\"tail -n 3 /etc/hosts\""'

$W start --name api "sleep 30" >/dev/null
check "explicit name clash errors" '! $W start --name api "sleep 30" >/dev/null 2>&1'
check "attach without a tty exits" '! $W attach api </dev/null >/dev/null 2>&1'

check "instant failure exits non-zero" '! $W start --name bad nonexistent_cmd_xyz >/dev/null 2>&1'

check "status --json has cwd" '$W status --json | grep -q "\"cwd\": \"$PWD\""'

$W start --name web "sleep 30" >/dev/null
(cd web && $W start --name inner "sleep 30" >/dev/null)
$W stop web >/dev/null; $W clean >/dev/null
check "clean leaves a subdir's procs alone" '(cd web && $W status | grep -q "name=inner  status=running")'

$W start bun run dev >/dev/null 2>&1
check "bun run dev is named dev" '$W status | grep -q "name=dev "'

$W start --name quick "echo hi" >/dev/null 2>&1
check "quick successful exit reports code 0" '$W logs quick | grep -q "exited with code 0"'

$W start --name sig "kill -TERM \$\$" >/dev/null 2>&1; code=$?
check "boot killed by signal exits 128+15" '[ $code = 143 ]'

$W start --name cnt "seq 1 100; sleep 30" >/dev/null; sleep 0.3
check "logs n returns exactly n lines" '[ "$($W logs cnt 5 | wc -l | tr -d " ")" = 5 ]'
check "logs | head does not panic" '$W logs cnt 50 | head -1 >/dev/null 2>&1 && [ -z "$($W logs cnt 50 2>&1 | head -1 | grep panicked)" ]'

$W stop cnt >/dev/null
check "stop writes footer" '$W logs cnt 1 | grep -q "\[whiskd:cnt\] stopped"'

exit $fail
