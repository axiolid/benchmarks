#!/bin/bash
# Final history run: 7 reps per (revision, area), median reported, plus
# min/max so the page can show a noise band. Single runs on a shared box
# cannot distinguish a 5% win from scheduler jitter.
set -u
unset CARGO_TARGET_DIR CARGO_HOME GIT_DIR GIT_WORK_TREE TMPDIR
export PATH=/home/friedrich/.cargo/bin:/usr/local/bin:/usr/bin:/bin
K=/mnt/backup/perf-hist/kernel
P=/mnt/backup/perf-hist/probe
OUT=/mnt/backup/perf-hist/history.csv
AREAS='boolean audit measure levelset inspect heal genus decimate refine raymesh project decompose'
TIMEFORMAT=%R
echo 'rev,area,samples' > $OUT
for REV in 5e52dde a24f8a6 b47274d; do
  cd $K && git checkout -q --detach $REV
  cd $P && cargo build --release >/dev/null 2>&1 || continue
  for A in $AREAS; do
    s=''
    for i in 1 2 3 4 5 6 7; do
      t=$( { time taskset -c 3 ./target/release/perf-probe $A 0 >/dev/null 2>&1; } 2>&1 )
      s="$s $t"
    done
    echo "$REV,$A,$s" >> $OUT
    echo "$REV $A done"
  done
done
