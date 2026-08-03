#!/bin/bash
set -e

# Build the project
cargo build --release

# Setup a dummy workspace
rm -rf test_workspace
mkdir -p test_workspace
echo "Creating dummy files..."
for i in {1..2000}; do
  touch test_workspace/file_$i.txt
done
for i in {1..2000}; do
  mkdir -p test_workspace/dir_$i
  touch test_workspace/dir_$i/file.txt
done

# Run the command and measure time
echo "Running clean..."
time target/release/kaji clean -w test_workspace --yes
