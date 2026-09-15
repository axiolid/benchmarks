#!/bin/bash
# Measure every area at each kernel revision.
#
# The probe source is held CONSTANT and only the kernel worktree moves, so
# a delta is attributable to the kernel change rather than to probe edits.
# Each arm forces a rebuild: the stale-binary trap already produced one
# bogus 37% figure this session.
set -u
unset CARGO_TARGET_DIR CARGO_HOME GIT_DIR GIT_WORK_TREE TMPDIR
export PATH=/home/friedrich/.cargo/bin:/usr/local/bin:/usr/bin:/bin
K=/mnt/backup/perf-hist/kernel
# The worktree is removed after each sweep, so recreate it here rather
# than assuming a previous run left one behind.
if [ ! -d $K ]; then
  cd /home/friedrich/projects/axiolid/kernel
  git worktree add --detach $K HEAD >/dev/null 2>&1
fi
P=/mnt/backup/perf-hist/probe
OUT=/mnt/backup/perf-hist/history.csv
AREAS='boolean audit measure levelset inspect heal genus decimate refine raymesh project decompose'
TIMEFORMAT=%R
echo 'rev,area,samples' > $OUT

for REV in 5e52dde a24f8a6 b47274d c8e150a; do
  cd $K && git checkout -q --detach $REV
  cd $P && cargo build --release >/dev/null 2>&1 || { echo "BUILD FAILED $REV" >&2; continue; }
  for A in $AREAS; do
    xs=''
    for i in 1 2 3 4 5 6 7; do
      t=$( { time taskset -c 3 ./target/release/perf-probe $A 0 >/dev/null 2>&1; } 2>&1 )
      xs="$xs $t"
    done
    echo "$REV,$A,$xs" >> $OUT
    echo "$REV $A$xs"
  done
done
