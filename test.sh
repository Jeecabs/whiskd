#!/bin/bash
# Smoke/regression checks. Runs in a throwaway cwd so real state is untouched.
set -u
W="node $(cd "$(dirname "$0")" && pwd)/whiskd"
T=$(cd "$(mktemp -d)" && pwd -P); mkdir -p "$T/web"; cd "$T"
fail=0
check() { if eval "$2"; then echo "ok   $1"; else echo "FAIL $1"; fail=1; fi; }
trap '$W stop --all >/dev/null 2>&1; $W clean >/dev/null; (cd web && $W stop --all >/dev/null 2>&1; $W clean >/dev/null); rm -rf "$T"' EXIT

node --check "${W#node }" || exit 1

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

exit $fail
