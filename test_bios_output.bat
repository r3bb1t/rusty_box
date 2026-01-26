@echo off
REM Test script for BIOS output redirection
REM Usage: test_bios_output.bat

echo ================================================================
echo Test 1: BIOS output to file with quiet logs
echo ================================================================
echo.

set BIOS_OUTPUT_FILE=bios_messages.txt
set BIOS_QUIET_MODE=1
cargo run --release --example dlxlinux --features std

echo.
echo ================================================================
echo BIOS messages captured in bios_messages.txt:
echo ================================================================
type bios_messages.txt
echo.

echo ================================================================
echo Test 2: Normal mode (mixed output to console)
echo ================================================================
echo.

set BIOS_OUTPUT_FILE=
set BIOS_QUIET_MODE=
cargo run --release --example dlxlinux --features std

echo.
echo ================================================================
echo Tests complete!
echo ================================================================
