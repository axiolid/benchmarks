#!/bin/bash
# Three ways to cast the same 2000 rays at the same mesh, one
# revision. Not a history: a CHOICE the caller makes.
set -u
unset CARGO_TARGET_DIR CARGO_HOME GIT_DIR GIT_WORK_TREE TMPDIR
export PATH=/home/friedrich/.cargo/bin:/usr/local/bin:/usr/bin:/bin
cd /home/friedrich/projects/axiolid/benchmarks/perf-probe
OUT=/mnt/backup/perf-hist/rayvariants.csv
echo "variant,samples" > $OUT
for v in raymesh facaderay handleray; do
  s=""
  for i in 1 2 3 4 5 6 7; do
    t0=$(date +%s%N)
    taskset -c 3 ./target/release/perf-probe $v 0 >/dev/null 2>&1
    t1=$(date +%s%N)
    s="$s $(( (t1 - t0) / 1000000 ))"
  done
  echo "$v,$s" >> $OUT
  echo "$v done"
done
