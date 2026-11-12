#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
out="$root/target/inquiry"
mkdir -p "$out"
cc=${CC:-cc}
"$cc" -std=c11 -Wall -Wextra -Werror -O2 \
    "$root/native/inquiry/main.c" -o "$out/p1100-inquire" \
    -framework IOKit -framework CoreFoundation -lcups
"$cc" -std=c11 -Wall -Wextra -Werror -O1 -g \
    -fsanitize=address,undefined "$root/native/inquiry/test.c" -o "$out/inquiry-tests"
"$out/inquiry-tests"
"$cc" -std=c11 -Wall -Wextra -Werror -O1 -g -fsanitize=address,undefined \
    "$root/native/inquiry/platform-test.c" -o "$out/platform-tests" \
    -framework IOKit -framework CoreFoundation -lcups
"$out/platform-tests"
"$out/p1100-inquire" --help
