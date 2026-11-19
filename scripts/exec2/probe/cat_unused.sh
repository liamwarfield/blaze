#!/bin/sh
# Writes the content of every input that does not contain "unused" to $1 and the paths of the
# others to $2 (the unused inputs list).
set -eu
output_file="$1"; shift
unused_file="$1"; shift
output=""
unused=""
for input in "$@"; do
  if grep -q "unused" "$input"; then
    unused="${unused}${input}
"
  else
    output="${output}$(cat "$input") "
  fi
done
printf '%s\n' "$output" > "$output_file"
printf '%s' "$unused" > "$unused_file"
