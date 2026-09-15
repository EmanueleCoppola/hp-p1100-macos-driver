#!/usr/bin/env python3
"""Check the whole two-bit path against the reference encoder.

Why this exists: a two-bit page is ours to produce - HP's tone tables screened the driver's way,
then two JBIG planes - and the only runnable implementation of the compression is the reference
one. So the same page is built twice and the two jobs are compared: once through
'p1100 raster-to-zjs --2bit --tone-table', and once through 'hp-tone-table.py --convert' and
'pbmtojbg -b', wrapped by 'p1100 wrap --bits 2'. That compares our conversion, our two-plane
encoder and our framing with the reference's in one run. It is what caught the aligned columns
being padded black instead of white, which is the kind of thing only an end-to-end check sees.

Needs the installed HP driver for the tone tables and the JBIG oracle for the reference encoder.

Usage:
  scripts/jbig-oracle.sh fetch      # once
  scripts/two-bit-compare.py [--verbose]

Exit status 0 when every page matches, 1 when one does not.
"""

import argparse
import importlib.util
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
PY = sys.executable
CLI = os.path.join(REPO, 'target/release/p1100')
ORACLE = os.path.join(HERE, 'jbig-oracle.sh')

# The parameters the reference encoder is given for a two-plane image: binary words rather than
# Gray code (-b), one resolution layer, L0 128 (-s), no adaptive template (-m 0), order 3 (-o),
# and options 0x1c (-p 28), which is what it picks for a multi-plane image - no LRLTWO.
PLANE_OPTIONS = 28

PAGES = ((16, 8), (16, 130), (129, 40), (300, 260))


def load(name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--verbose', action='store_true')
    args = ap.parse_args(argv)

    tone = load('hp-tone-table')
    if not os.path.exists(tone.HP_BINARY):
        print('no HP driver at %s: nothing to screen with, so nothing to check'
              % tone.HP_BINARY)
        return 0
    if not os.path.exists(CLI):
        raise SystemExit('build the CLI first: scripts/build-native.sh')
    work = tempfile.mkdtemp(prefix='p1100-two-bit-')
    driver, offsets = tone.symbol_offsets(tone.HP_BINARY)
    size = tone.entry_size()[0]
    table_name = '_sdtahs10'
    table = driver[offsets[table_name]:offsets[table_name] + size * 256]
    table_path = os.path.join(work, 'tone.bin')
    with open(table_path, 'wb') as f:
        f.write(table)

    failures = 0
    for width, height in PAGES:
        rows = [[(x * 17 + y * 5) % 256 for x in range(width)] for y in range(height)]
        raster = tone.tiny_raster(os.path.join(work, 'page.raster'), width, height, rows)
        ours = os.path.join(work, 'ours.zjs')
        subprocess.run([CLI, 'raster-to-zjs', raster, ours, '--2bit', '--tone-table', table_path],
                       check=True, stderr=subprocess.DEVNULL)

        pgm = tone.convert(raster, table, work, stem='ref')[0][0]
        bie = os.path.join(work, 'ref.jbg')
        subprocess.run([ORACLE, 'encode', pgm, bie, '-b', '-p', str(PLANE_OPTIONS)], check=True,
                       stdout=subprocess.DEVNULL)
        blob = open(bie, 'rb').read()
        open(os.path.join(work, 'ref.bih'), 'wb').write(blob[:20])
        open(os.path.join(work, 'ref.bid'), 'wb').write(blob[20:])
        ref = os.path.join(work, 'ref.zjs')
        subprocess.run([CLI, 'wrap', '--bih', os.path.join(work, 'ref.bih'), '--bid',
                        os.path.join(work, 'ref.bid'), '--bits', '2', '--width', str(width),
                        '--height', str(height), '-o', ref], check=True,
                       stderr=subprocess.DEVNULL)

        diff = subprocess.run([PY, os.path.join(HERE, 'zjs-compare.py'), ours, ref,
                               '--ignore-jobattr'], stdout=subprocess.PIPE)
        same = b'SEMANTICALLY IDENTICAL' in diff.stdout
        ours_size = os.path.getsize(ours)
        ref_size = os.path.getsize(ref)
        if same:
            if args.verbose:
                print('  ok  %4dx%-4d  %d bytes each' % (width, height, ours_size))
        else:
            failures += 1
            print('  MISMATCH %4dx%-4d  ours %d bytes, reference %d bytes'
                  % (width, height, ours_size, ref_size))
            print(diff.stdout.decode('utf-8', 'replace'))

    print('%d two-bit page(s), %d mismatch(es)' % (len(PAGES), failures))
    if failures:
        print('  the scratch directory is kept: %s' % work)
        return 1
    print('our screening, our two-plane encoder and our framing agree with the reference encoder')
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
