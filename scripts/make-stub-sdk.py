#!/usr/bin/env python3
"""Build a link-only copy of the SDK whose stubs rust-lld can parse.

Why this is needed: macOS 27's SDK describes some libraries with an architecture token,
arm64e.x1, that the LLVM bundled with the current Rust toolchain does not know. rust-lld
therefore rejects those .tbd files as malformed ("unknown architecture"), so libSystem, libc
and libm cannot be resolved and any link fails. Apple's own ld64 understands the token, which
is why clang works and rust-lld does not.

This makes a copy of the SDK's stub directory in which those tokens are removed, and leaves
the real SDK untouched. Nothing is installed and nothing outside the output directory is
written.

What it changes and what it does not: only the arm64e.x1 entries in target lists are removed.
For an arm64 link those entries never apply anyway, so the symbol sets our build sees are the
same ones Apple's linker would use. The runtime library is the real libSystem from the OS;
these files only describe it at link time.

The supported way to build is to accept the Xcode licence and let Apple's toolchain do this.
This exists so the project can still be built and verified without it - see docs/testing.md.

Usage:
  scripts/make-stub-sdk.py [--out DIR] [--sdk DIR] [--quiet]

Defaults: --out ~/.cache/p1005-stub-sdk, --sdk /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk
"""

import argparse
import os
import re
import shutil
import sys

DEFAULT_SDK = '/Library/Developer/CommandLineTools/SDKs/MacOSX.sdk'
DEFAULT_OUT = os.path.expanduser('~/.cache/p1005-stub-sdk')

# The token that breaks rust-lld, and any list of targets it appears in.
BAD_TOKEN = 'arm64e.x1-'
LIST = re.compile(r'\[([^\[\]]*)\]', re.S)


def clean(match):
    """Drop arm64e.x1 entries from one bracketed list, keeping the rest in order."""
    items = [x.strip() for x in match.group(1).split(',')]
    items = [x for x in items if x and not x.startswith(BAD_TOKEN)]
    return '[' + ', '.join(items) + ']'


def build(sdk, out, quiet):
    src_lib = os.path.join(sdk, 'usr', 'lib')
    if not os.path.isdir(src_lib):
        raise SystemExit('no usr/lib under %s; is that an SDK?' % sdk)
    dst_lib = os.path.join(out, 'usr', 'lib')
    shutil.rmtree(dst_lib, ignore_errors=True)

    rewritten = symlinked = 0
    for dirpath, _dirnames, filenames in os.walk(src_lib):
        rel = os.path.relpath(dirpath, src_lib)
        dest_dir = dst_lib if rel == '.' else os.path.join(dst_lib, rel)
        os.makedirs(dest_dir, exist_ok=True)
        for name in filenames:
            src = os.path.join(dirpath, name)
            dst = os.path.join(dest_dir, name)
            if name.endswith('.tbd'):
                with open(src) as f:
                    text = f.read()
                if BAD_TOKEN in text:
                    text = LIST.sub(clean, text)
                with open(dst, 'w') as f:
                    f.write(text)
                rewritten += 1
            else:
                # headers, crt1.o and friends: link the originals rather than copying them
                if os.path.lexists(dst):
                    os.unlink(dst)
                os.symlink(src, dst)
                symlinked += 1

    if not quiet:
        left = 0
        for dirpath, _d, filenames in os.walk(dst_lib):
            for name in filenames:
                if name.endswith('.tbd'):
                    with open(os.path.join(dirpath, name)) as f:
                        left += f.read().count(BAD_TOKEN)
        print('stub SDK: %d tbd files written, %d entries symlinked, %d bad tokens left'
              % (rewritten, symlinked, left))
    return out


def main(argv):
    ap = argparse.ArgumentParser(add_help=True,
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 description=__doc__)
    ap.add_argument('--sdk', default=DEFAULT_SDK)
    ap.add_argument('--out', default=DEFAULT_OUT)
    ap.add_argument('--quiet', action='store_true')
    args = ap.parse_args(argv)
    print(build(args.sdk, args.out, args.quiet))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))

