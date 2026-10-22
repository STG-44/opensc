#!/usr/bin/env bash
set -euo pipefail
project_dir=$(cd -- "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
game_dir=${1:-${SHADOW_GAME_DIR:-}}
if [[ -z "$game_dir" && -f "$project_dir/local/game-dir.txt" ]]; then
    game_dir=$(<"$project_dir/local/game-dir.txt")
fi
if [[ -z "$game_dir" ]]; then
    printf 'Укажи каталог оригинала: ./scripts/demo-game.sh "/путь/к/игре"\n' >&2
    exit 1
fi
args=(--game-dir "$game_dir")
if [[ -n ${2:-} ]]; then args+=(--overrides "$2"); fi
if (( $# > 2 )); then args+=("${@:3}"); fi
exec "$project_dir/scripts/dev.sh" cargo run --locked -p shadow-model-viewer --bin shadow-game -- "${args[@]}"
