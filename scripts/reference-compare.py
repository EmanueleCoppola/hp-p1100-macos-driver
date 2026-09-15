#!/usr/bin/env python3
"""Compare our job against the reference driver's job, for the same page and options.

Why this exists: HP's filter is x86_64 and cannot run on this machine, so the closest
available statement of "what a working driver sends this printer" is foo2zjs, which
supports the P1102 and which scripts/foo2zjs-oracle.sh builds locally. This runs both
implementations over the same page bitmaps with the same print options and reports every
difference, in items rather than byte offsets where it can.

The page bitmap is computed independently of the Rust code (the same way
scripts/rehearse-job.py does it), so the comparison isolates the job encoding rather than
mixing in the raster conversion.

One scope limit, and it is asserted rather than assumed: the reference reads 1-bit PBM pages,
so the page both implementations see is bilevel. That covers the job encoding - items, chunk
framing, geometry, the compressed image - and says nothing about the 8 bits per pixel to 1 bit
conversion, which this harness performs itself with the same threshold the filter uses
(docs/differences.md item 11). A fixture with grey in it would quietly turn the case into a
comparison of our thresholding against itself, so assert_bilevel() refuses one instead.

Pass condition: the two jobs are byte-identical once the job-attribute line is removed,
which carries the clock. Anything else is a real difference and is printed.

Usage:
  scripts/foo2zjs-oracle.sh fetch          # once, builds the reference
  scripts/reference-compare.py [--only SUBSTRING] [--keep DIR] [--filter PATH]
"""

import argparse
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
NATIVE_FILTER = os.path.join(REPO, 'target/release/p1102-filter')
WASM_FILTER = os.path.join(REPO, 'target/wasm32-wasip1/debug/p1102-filter.wasm')
ORACLE = os.path.join(HERE, 'foo2zjs-oracle.sh')
PPD = '/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd'

# DMPAPER codes the reference takes on its command line, for the page sizes the PPD offers.
PAPER_CODES = {'Letter': 1, 'Legal': 5, 'Executive': 7, 'A4': 9, 'A5': 11, 'B5': 13}

# label, cupsfilter -o options, our filter's argv[5], the reference's flags, fixture
CASES = [
    ('letter, defaults', [], '', [], 'text-1p.txt'),
    ('a4', ['PageSize=A4'], '', ['-p', '9'], 'text-1p.txt'),
    ('econo mode', ['EconoMode=True'], 'EconoMode=True', ['-t'], 'text-1p.txt'),
    ('density 5', ['pjlDensity=5'], 'pjlDensity=5', ['-T', '5'], 'text-1p.txt'),
    ('manual feed', ['InputSlot=Manual'], 'InputSlot=Manual', ['-s', '4'], 'text-1p.txt'),
    ('three copies', ['copies=3'], 'copies=3', ['-n', '3'], 'text-1p.txt'),
    ('two pages', [], '', [], 'text-2p.txt'),
    # Page sizes other than Letter and A4: the raster's page size name decides DMPAPER, so
    # each of these tests one entry of that mapping against the code the reference uses for
    # the same page size. B5JIS, EnvDL, A5, the 16K sizes and 8.5x13Custom all used to fall
    # through the table and be sent as Letter.
    ('a5', ['PageSize=A5'], '', ['-p', '11'], 'text-1p.txt'),
    ('legal', ['PageSize=Legal'], '', ['-p', '5'], 'text-1p.txt'),
    ('executive', ['PageSize=Executive'], '', ['-p', '7'], 'text-1p.txt'),
    ('a6', ['PageSize=A6'], '', ['-p', '70'], 'text-1p.txt'),
    ('b5 jis', ['PageSize=B5JIS'], '', ['-p', '13'], 'text-1p.txt'),
    ('envelope #10', ['PageSize=Env10'], '', ['-p', '20'], 'text-1p.txt'),
    ('envelope dl', ['PageSize=EnvDL'], '', ['-p', '27'], 'text-1p.txt'),
    ('envelope c5', ['PageSize=EnvC5'], '', ['-p', '28'], 'text-1p.txt'),
    ('envelope iso b5', ['PageSize=EnvISOB5'], '', ['-p', '34'], 'text-1p.txt'),
    ('monarch', ['PageSize=Monarch'], '', ['-p', '37'], 'text-1p.txt'),
    ('postcard', ['PageSize=Postcard'], '', ['-p', '43'], 'text-1p.txt'),
    ('16k 197x273', ['PageSize=16K197x273mm'], '', ['-p', '257'], 'text-1p.txt'),
    ('16k 184x260', ['PageSize=16K184x260mm'], '', ['-p', '263'], 'text-1p.txt'),
    ('16k 195x270', ['PageSize=16K195x270mm'], '', ['-p', '264'], 'text-1p.txt'),
    ('8.5x13 custom', ['PageSize=8.5x13Custom'], '', ['-p', '258'], 'text-1p.txt'),
    ('double postcard', ['PageSize=DoublePostcard'], '', ['-p', '82'], 'text-1p.txt'),
    # A page size the PPD does not list arrives as "Custom", which has its own code.
    ('custom size', ['media=Custom.300x400'], '', ['-p', '256'], 'text-1p.txt'),
    # The other resolution the PPD offers: CUPS puts 600x600 in the raster for both, so the
    # job should be the same as the default case, and it is worth knowing if it is not.
    ('600x600 resolution', ['Resolutions=600x600dpi'], '', ['-p', '1'], 'text-1p.txt'),
    # Orientation: the chain rotates the page *inside* the raster, so both sides receive the
    # already-rotated bitmap and neither driver has to do anything with it. Worth a case
    # because it is a thing users do and nothing else here covers it.
    ('landscape', ['landscape'], '', [], 'text-1p.txt'),
    ('upside down', ['orientation-requested=3'], '', [], 'text-1p.txt'),
    # N-up is the same kind of thing: CUPS lays the pages out before rasterising them.
    ('two pages per sheet', ['number-up=2'], '', [], 'text-2p.txt'),
]

# Not compared here: the media code. It reaches our filter in the raster header's
# cupsMediaType field, which the PPD's MediaType option sets through the PostScript page
# device - something cupsfilter cannot reproduce (its raster always says Paper/1), while
# the reference takes the code as a flag. The mapping itself is covered by a unit test in
# crates/p1102-job; what stays unverified is whether the printer wants HP's codes.
#
# Also not compared here: duplex. The reference reorders duplex pages host-side, rotating
# every second one and padding an odd page count with a blank sheet, while we emit
# ZJI_DMDUPLEX on the document, page and image records and leave the ordering to the
# printer - which is the architecture HP's own driver sits in. The two jobs therefore
# differ by construction on any duplex job, and only a two-sided print settles which the
# P1102 wants; see docs/differences.md.


def load(name):
    """Import a sibling tool so its logic is reused rather than duplicated."""
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def filter_command():
    if os.path.exists(NATIVE_FILTER):
        return [NATIVE_FILTER]
    if not os.path.exists(WASM_FILTER):
        raise SystemExit('build the filter first: ./scripts/build-native.sh, or the wasm\n'
                         'build for this machine: cargo build --target wasm32-wasip1 \\\n'
                         '  -p p1102-filter --offline')
    wt = os.environ.get('P1102_WASMTIME') or shutil.which('wasmtime')
    if wt:
        return [wt, 'run', '--dir', '/', WASM_FILTER]
    return ['node', os.path.join(HERE, 'wasi-run.mjs'), WASM_FILTER]


def make_raster(fixture, options, dst):
    cmd = ['/usr/sbin/cupsfilter', '-m', 'application/vnd.cups-raster', '-p', PPD]
    for opt in options:
        cmd += ['-o', opt]
    cmd.append(os.path.join(REPO, 'fixtures/inputs', fixture))
    with open(dst, 'wb') as out:
        proc = subprocess.run(cmd, stdout=out, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise SystemExit('cupsfilter failed: %s' % proc.stderr.decode('utf-8', 'replace'))
    return os.path.getsize(dst)


def our_job(cmd, raster, options, dst):
    # CUPS gives a filter the queue's PPD in the environment and sends only the options the
    # job named, so this is what a real job looks like: the filter takes the rest from the file.
    env = dict(os.environ)
    env['PPD'] = PPD
    with open(dst, 'wb') as out:
        proc = subprocess.run(cmd + ['1', 'manu', 'compare', '1', options, raster],
                              stdout=out, stderr=subprocess.PIPE, env=env)
    if proc.returncode != 0:
        raise SystemExit('our filter failed: %s' % proc.stderr.decode('utf-8', 'replace'))
    return os.path.getsize(dst)


def reference_job(pages, header, flags, work):
    """Feed the reference the same bitmaps, at the true (unaligned) raster width.

    Our bitmaps carry the 128-pixel aligned stride; the reference wants the page's real
    width and does the alignment itself, which is exactly the behaviour being compared.
    """
    paths = []
    for n, (width, aligned_width, height, bitmap) in enumerate(pages):
        stride = aligned_width // 8
        row_bytes = (width + 7) // 8
        path = os.path.join(work, 'page-%d.pbm' % n)
        with open(path, 'wb') as f:
            f.write(b'P4\n%d %d\n' % (width, height))
            for y in range(height):
                f.write(bitmap[y * stride:y * stride + row_bytes])
        paths.append(path)

    rx, ry = header['resolution']
    page = header['page_size']
    gx = int(round(page[0] * rx / 72.0))
    gy = int(round(page[1] * ry / 72.0))
    code = PAPER_CODES.get(header['page_size_name'])
    # `-s 4` is the media source this printer's PPD defaults to, which is what the filter reads
    # out of the file when the job names nothing; the reference takes it as a flag instead.
    cmd = [ORACLE, '-z2', '-P', '-L0', '-r%dx%d' % (rx, ry), '-g%dx%d' % (gx, gy), '-s', '4']
    if code is not None:
        cmd += ['-p', str(code)]
    cmd += flags + paths
    out = os.path.join(work, 'reference.zjs')
    with open(out, 'wb') as dst:
        proc = subprocess.run(cmd, stdout=dst, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise SystemExit('the reference failed: %s' % proc.stderr.decode('utf-8', 'replace'))
    return open(out, 'rb').read()


# The whole line is what the pass condition removes, but its shape is checked: both jobs must
# carry a fourteen-digit stamp, so a format regression cannot hide behind the removal. The
# values differ because the two runs happen at different instants - and because ours is UTC
# while the reference's is local time, which is recorded in docs/differences.md.
JOBATTR = re.compile(rb'@PJL SET JOBATTR="JobAttr4=(\d{14})"\x00')


def without_timestamp(blob):
    return JOBATTR.sub(b'', blob)


def assert_bilevel(data, cr, label):
    """Refuse a fixture with grey in it, because the reference is fed a 1-bit page.

    The bitmap handed to the reference below is thresholded here, at the same cut our filter
    uses, so a case built from a raster with intermediate levels compares our conversion with
    our own conversion rather than with the reference's. That is a silent loss of coverage, so
    it is an error rather than a note: see docs/differences.md item 11.
    """
    for _, off, header, payload in cr.walk(data):
        if header['bits_per_pixel'] != 8:
            continue
        start = off + cr.PAGE_HEADER_LEN
        levels = sorted(set(data[start:start + payload]) - {0, 255})
        if levels:
            raise SystemExit(
                '%s: the raster carries %d intermediate grey level(s) (first: %s), and the '
                'reference is fed a bilevel page, so this case would no longer compare the '
                'grey conversion with the reference - see docs/differences.md item 11'
                % (label, len(levels), ', '.join(str(v) for v in levels[:4])))


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--only', help='run only cases whose label contains this')
    ap.add_argument('--keep', help='keep the generated rasters and jobs here')
    ap.add_argument('--filter', help='filter binary to run instead of the built default')
    args = ap.parse_args(argv)

    probe = subprocess.run([ORACLE, 'which'], stdout=subprocess.PIPE)
    if not os.path.exists(probe.stdout.decode().strip()):
        raise SystemExit('build the reference first: scripts/foo2zjs-oracle.sh fetch')

    cmd = [args.filter] if args.filter else filter_command()
    cr = load('cupsraster')
    rehearse = load('rehearse-job')

    work = args.keep or tempfile.mkdtemp(prefix='p1102-reference-')
    os.makedirs(work, exist_ok=True)
    print('comparing against the reference driver (foo2zjs)')
    print('work directory: %s' % work)
    print()

    failures = 0
    stamps = None
    for label, cups_opts, options, flags, fixture in CASES:
        if args.only and args.only not in label:
            continue
        case_dir = os.path.join(work, re.sub(r'[^a-z0-9]+', '-', label.lower()).strip('-'))
        os.makedirs(case_dir, exist_ok=True)
        raster = os.path.join(case_dir, 'page.raster')
        size = make_raster(fixture, cups_opts, raster)
        data = open(raster, 'rb').read()
        assert_bilevel(data, cr, label)
        page_headers = [h for _, _, h, _ in cr.walk(data)]
        ours_path = os.path.join(case_dir, 'ours.zjs')
        ours_size = our_job(cmd, raster, options, ours_path)
        pages = [(hdr['width'], aligned, hdr['height'], bitmap)
                 for (aligned, _, bitmap, _), (_, _, hdr, _)
                 in zip(rehearse.expected_pages(data, cr), cr.walk(data))]
        reference = reference_job(pages, page_headers[0], flags, case_dir)
        ours = open(ours_path, 'rb').read()

        stamp = JOBATTR.search(ours)
        other = JOBATTR.search(reference)
        if not stamp or not other:
            raise SystemExit('%s: job-attribute stamp missing or malformed (ours %r, reference %r)'
                             % (label, stamp.group(1) if stamp else None,
                                other.group(1) if other else None))
        if stamps is None:
            stamps = (stamp.group(1).decode(), other.group(1).decode())
        same = without_timestamp(ours) == without_timestamp(reference)
        print('%-18s %-6s %d pages, %d raster bytes, ours %d bytes, reference %d bytes'
              % (label, 'PASS' if same else 'FAIL', len(pages), size, ours_size,
                 len(reference)))
        if not same:
            failures += 1
            diff = subprocess.run([sys.executable, os.path.join(HERE, 'zjs-compare.py'),
                                   ours_path, os.path.join(case_dir, 'reference.zjs'),
                                   '--ignore-jobattr'], stdout=subprocess.PIPE)
            for line in diff.stdout.decode('utf-8', 'replace').splitlines():
                print('    %s' % line)

    print()
    if failures:
        print('%d case(s) differ' % failures)
        print('  its scratch directory is kept for inspection: %s' % work)
        return 1
    if stamps:
        print('the timestamp line is the only difference: ours %s (UTC), the reference\'s %s,'
              % stamps)
        print('  the same fourteen-digit format on a different clock basis (differences.md 7)')
    print('every case is byte-identical to the reference once the timestamp is removed'
          if not args.keep else 'every case matched; intermediates kept in %s' % work)
    # Each case leaves a raster of tens of megabytes behind, so a run that passes cleans up
    # after itself; a failing run keeps the directory for the reader to look at.
    if not args.keep:
        shutil.rmtree(work, ignore_errors=True)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
