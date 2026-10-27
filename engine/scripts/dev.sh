#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
workspace_dir=$(cd -- "$project_dir/../.." && pwd)
if [[ ! -x "$workspace_dir/work/toolchains/cargo/bin/cargo" ]]; then
    workspace_dir=$(cd -- "$project_dir/../../.." && pwd)
fi
# В этой рабочей папке Rust установлен локально. В другом месте используем PATH.
if [[ -x "$workspace_dir/work/toolchains/cargo/bin/cargo" ]]; then
    export CARGO_HOME="$workspace_dir/work/toolchains/cargo"
    export RUSTUP_HOME="$workspace_dir/work/toolchains/rustup"
    export CARGO_TARGET_DIR="$workspace_dir/work/bevy-target"
    export PATH="$CARGO_HOME/bin:$PATH"
fi
cd -- "$project_dir"
exec "$@"
