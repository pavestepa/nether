#!/bin/bash
# Сравнение производительности Nether и Rust на эквивалентных программах.
# Использование: ./run.sh [повторов=5]
# Nether собирается как `nether build --release --overflow-checks=off`, Rust — rustc -C opt-level=3.
set -euo pipefail
cd "$(dirname "$0")"
RUNS=${1:-5}
ROOT=$(cd .. && pwd)
cargo build -q --release -p nether --manifest-path "$ROOT/Cargo.toml"
NETHER="$ROOT/target/release/nether"
OUT=out
mkdir -p "$OUT"
now() { python3 -c 'import time;print(time.perf_counter())'; }
best_of() { # <бинарник> -> минимальное время и код выхода
  local bin=$1 best=999999 code=0 t0 t1 dt
  for _ in $(seq "$RUNS"); do
    t0=$(now); set +e; "$bin"; code=$?; set -e; t1=$(now)
    dt=$(python3 -c "print($t1-$t0)")
    best=$(python3 -c "print(min($best,$dt))")
  done
  echo "$best $code"
}
printf '%-14s %10s %10s %8s  %s\n' benchmark rust_s nether_s ratio result
for src in bench/*.nr; do
  name=$(basename "$src" .nr)
  rustc -C opt-level=3 -C debuginfo=0 "bench/$name.rs" -o "$OUT/$name-rust" 2>/dev/null
  "$NETHER" build "$src" --release --overflow-checks=off -o "$OUT/$name-nether" >/dev/null
  read -r tr cr < <(best_of "$OUT/$name-rust")
  read -r tn cn < <(best_of "$OUT/$name-nether")
  status=ok; [ "$cr" = "$cn" ] || status="MISMATCH rust=$cr nether=$cn"
  printf '%-14s %10.3f %10.3f %7.2fx  %s (exit %s)\n' "$name" "$tr" "$tn" "$(python3 -c "print($tn/$tr)")" "$status" "$cr"
done
