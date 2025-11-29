#!/bin/bash

echo "=== Fast-RM Performance Benchmark ==="
echo ""

# Build release version
echo "Building release version..."
cargo build --release 2>&1 | grep -E "Finished|Compiling fast-rm" || true
echo ""

FAST_RM="./target/release/fast-rm"

# Test 1: Small directory (100 files)
echo "Test 1: 100 files (flat structure)"
TEST_DIR=$(mktemp -d)
for i in {1..100}; do
    echo "test $i" > "$TEST_DIR/file_$i.txt"
done
echo "  Files created: 100"
TIME_START=$(python3 -c "import time; print(time.time())")
$FAST_RM "$TEST_DIR" 2>&1 | grep -E "Summary:|items"
TIME_END=$(python3 -c "import time; print(time.time())")
ELAPSED=$(python3 -c "print(f'{$TIME_END - $TIME_START:.3f}')")
echo "  Time: ${ELAPSED}s"
echo ""

# Test 2: Medium directory (1000 files)
echo "Test 2: 1,000 files (flat structure)"
TEST_DIR=$(mktemp -d)
for i in {1..1000}; do
    echo "test $i" > "$TEST_DIR/file_$i.txt"
done
echo "  Files created: 1,000"
TIME_START=$(python3 -c "import time; print(time.time())")
$FAST_RM "$TEST_DIR" 2>&1 | grep -E "Summary:|items"
TIME_END=$(python3 -c "import time; print(time.time())")
ELAPSED=$(python3 -c "print(f'{$TIME_END - $TIME_START:.3f}')")
echo "  Time: ${ELAPSED}s"
THROUGHPUT=$(python3 -c "print(f'{1000 / ($TIME_END - $TIME_START):.0f}')")
echo "  Throughput: ${THROUGHPUT} items/sec"
echo ""

# Test 3: Nested structure (depth=3, breadth=5)
echo "Test 3: Nested structure (depth=3, breadth=5)"
TEST_DIR=$(mktemp -d)
python3 -c "
import os
def create_nested(path, depth, breadth):
    if depth == 0:
        return 0
    count = 0
    for i in range(breadth):
        f = os.path.join(path, f'file_{i}.txt')
        open(f, 'w').write(f'test {i}')
        count += 1
    for i in range(breadth):
        d = os.path.join(path, f'dir_{i}')
        os.makedirs(d, exist_ok=True)
        count += 1
        count += create_nested(d, depth - 1, breadth)
    return count

total = create_nested('$TEST_DIR', 3, 5)
print(f'  Files+Dirs created: {total}')
"
TIME_START=$(python3 -c "import time; print(time.time())")
$FAST_RM "$TEST_DIR" 2>&1 | grep -E "Summary:|items"
TIME_END=$(python3 -c "import time; print(time.time())")
ELAPSED=$(python3 -c "print(f'{$TIME_END - $TIME_START:.3f}')")
echo "  Time: ${ELAPSED}s"
echo ""

# Test 4: Thread scaling (2000 files)
echo "Test 4: Thread scaling (2,000 files)"
for THREADS in 1 2 4 8; do
    TEST_DIR=$(mktemp -d)
    for i in {1..2000}; do
        echo "test $i" > "$TEST_DIR/file_$i.txt"
    done
    echo -n "  ${THREADS} threads: "
    TIME_START=$(python3 -c "import time; print(time.time())")
    $FAST_RM --scan-threads $THREADS --delete-threads $THREADS "$TEST_DIR" > /dev/null 2>&1
    TIME_END=$(python3 -c "import time; print(time.time())")
    ELAPSED=$(python3 -c "print(f'{$TIME_END - $TIME_START:.3f}')")
    THROUGHPUT=$(python3 -c "print(f'{2000 / ($TIME_END - $TIME_START):.0f}')")
    echo "${ELAPSED}s (${THROUGHPUT} items/sec)"
done
echo ""

# Test 5: Comparison with system rm (1000 files)
echo "Test 5: Comparison with system rm (1,000 files)"
TEST_DIR=$(mktemp -d)
for i in {1..1000}; do
    echo "test $i" > "$TEST_DIR/file_$i.txt"
done
echo -n "  fast-rm: "
TIME_START=$(python3 -c "import time; print(time.time())")
$FAST_RM "$TEST_DIR" > /dev/null 2>&1
TIME_END=$(python3 -c "import time; print(time.time())")
FAST_RM_TIME=$(python3 -c "print(f'{$TIME_END - $TIME_START:.3f}')")
echo "${FAST_RM_TIME}s"

TEST_DIR=$(mktemp -d)
for i in {1..1000}; do
    echo "test $i" > "$TEST_DIR/file_$i.txt"
done
echo -n "  system rm -rf: "
TIME_START=$(python3 -c "import time; print(time.time())")
rm -rf "$TEST_DIR"
TIME_END=$(python3 -c "import time; print(time.time())")
SYS_RM_TIME=$(python3 -c "print(f'{$TIME_END - $TIME_START:.3f}')")
echo "${SYS_RM_TIME}s"

SPEEDUP=$(python3 -c "print(f'{float('$SYS_RM_TIME') / float('$FAST_RM_TIME'):.2f}')")
echo "  Speedup: ${SPEEDUP}x"
echo ""

echo "=== Benchmark Complete ==="
