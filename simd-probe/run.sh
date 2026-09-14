#!/bin/bash
# Compile the SAME source at baseline x86-64 versus target-cpu=native and
# compare like for like.
#
# Separate target dirs: sharing one makes cargo rebuild on every flag
# flip and risks a stale binary being measured.
#
# Runs are INTERLEAVED (base, native, base, native, ...) rather than all
# of one then all of the other, so thermal drift or a background process
# hits both arms roughly equally instead of biasing one.
set -e
cd "$(dirname "$0")"
unset CARGO_TARGET_DIR CARGO_HOME GIT_DIR GIT_WORK_TREE TMPDIR
export PATH=/home/friedrich/.cargo/bin:/usr/local/bin:/usr/bin:/bin

echo "building baseline (x86-64, SSE2)..."
CARGO_TARGET_DIR=/mnt/backup/build-cache/simd-base \
  RUSTFLAGS="-C target-cpu=x86-64" \
  cargo build --release -q

echo "building native (AVX-512)..."
CARGO_TARGET_DIR=/mnt/backup/build-cache/simd-native \
  RUSTFLAGS="-C target-cpu=native" \
  cargo build --release -q

BASE=/mnt/backup/build-cache/simd-base/release/simd-probe
NATIVE=/mnt/backup/build-cache/simd-native/release/simd-probe

# Confirm the flag actually changed the emitted code. If both binaries
# are identical the comparison is meaningless, and that has to be caught
# here rather than misread as "SIMD does not help".
echo "baseline sha : $(sha256sum "$BASE"  | cut -c1-16)"
echo "native   sha : $(sha256sum "$NATIVE" | cut -c1-16)"
echo "avx512 insns in baseline: $(objdump -d "$BASE"   | grep -c 'zmm' || true)"
echo "avx512 insns in native  : $(objdump -d "$NATIVE" | grep -c 'zmm' || true)"
echo

# Pinned to one core: migration between cores is a large part of the
# run-to-run spread, and the first pass showed +-10% noise, wide enough
# to swallow any real effect.
for i in 1 2 3 4 5 6 7; do
  taskset -c 3 "$BASE"   | sed "s/^/base,/"   > "/tmp/simd_base_$i.csv"
  taskset -c 3 "$NATIVE" | sed "s/^/native,/" > "/tmp/simd_native_$i.csv"
done
cat /tmp/simd_base_*.csv /tmp/simd_native_*.csv
