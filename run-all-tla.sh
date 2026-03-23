#! /usr/bin/env bash

set -eu

cd -- "$( dirname -- "${BASH_SOURCE[0]}" )"

script=$(cat <<'EOF'
set -eu

# This is probably significantly more than enough memory for the below specs.
export JAVA_TOOL_OPTIONS='-Xmx29G'

W=$(nproc)
if [[ "$W" -gt 12 ]]; then
  W=12
fi

# basically set -v but doesn't print shellHook stuff.
run() {
  printf '%q ' "$@"
  printf '\n'
  "$@"
}

# `-lncheck final` defers liveness checking to the end and makes each check
# finish faster.
#
# There are multiple cfgs so that we can run multiple models on the "frontier"
# of reasonable run time. Non-liveness models can enable symmetry, which lets
# us run more complex scenarios (more processes, signal depth, iterations).

#run tlc -workers "$W" -lncheck final -config tla/futex.cfg tla/futex.tla
#run tlc -workers "$W" -lncheck final -config tla/futex-liveness1.cfg tla/futex.tla
#run tlc -workers "$W" -lncheck final -config tla/futex-liveness2.cfg tla/futex.tla
#run tlc -workers "$W" -lncheck final -config tla/futex-liveness3.cfg tla/futex.tla

run tlc -workers "$W" -lncheck final -config tla/pipe.cfg tla/pipe.tla
run tlc -workers "$W" -lncheck final -config tla/pipe2.cfg tla/pipe.tla
# The futex liveness configs work well for pipe.tla too.
run tlc -workers "$W" -lncheck final -config tla/futex-liveness1.cfg tla/pipe.tla
run tlc -workers "$W" -lncheck final -config tla/futex-liveness2.cfg tla/pipe.tla
run tlc -workers "$W" -lncheck final -config tla/futex-liveness3.cfg tla/pipe.tla
EOF
)

nix-shell --run "$script"
