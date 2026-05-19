#!/bin/sh
export CS_URL=http://localhost:8888
export RUST_LOG=info
export VLT_GAME_PATH="/Users/amoura/pachinko/Projects/pachinko_game/pachinko_remake/pachinko_game_bingo/Export/macos/bin/PachinkoGameBingo.app/Contents/MacOS/PachinkoGameBingo"
export VLT_GAME_ARGS="--env=local --channel web --no-button-hub"
exec ./target/debug/pachinko_vlt_launcher "$@"
