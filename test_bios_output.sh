#!/bin/bash
# Test script for BIOS output redirection
# Usage: ./test_bios_output.sh

echo "================================================================"
echo "Test 1: BIOS output to file with quiet logs"
echo "================================================================"
echo ""

BIOS_OUTPUT_FILE=bios_messages.txt BIOS_QUIET_MODE=1 \
  cargo run --release --example dlxlinux --features std

echo ""
echo "================================================================"
echo "BIOS messages captured in bios_messages.txt:"
echo "================================================================"
cat bios_messages.txt
echo ""

echo "================================================================"
echo "Test 2: Normal mode (mixed output to console)"
echo "================================================================"
echo ""

cargo run --release --example dlxlinux --features std 2>&1 | head -100

echo ""
echo "================================================================"
echo "Tests complete!"
echo "================================================================"
