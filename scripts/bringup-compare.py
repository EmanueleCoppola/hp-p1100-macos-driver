#!/usr/bin/env python3
"""Compare the bring-up tool's jobs with the shipped implementation's, and both with CUPS.

scripts/make-test-job.py is what a first hardware test uses: it draws a pattern, compresses it
with the reference JBIG encoder, and frames a job around it, so a printer that rejects that job
is rejecting our framing rather than our compressor. That makes a defect in it expensive - it
would read as "the printer does not accept our protocol", which is the one thing the bring-up is
supposed to settle. Its framing is Python, mirroring crates/p1102-protocol, and until this script
existed nothing was comparing the two.

  make-test-job.py    -> job A     Python framing, reference-compressed image
  p1102 wrap          -> job B     the Rust framing, the same image, byte for byte
  p1102 test-page     -> job C     the Rust drawing and framing, same patterns

All three have to be the same job: identical where that is possible, and semantically identical
(scripts/zjs-compare.py, ignoring the timestamp line) where the clock makes the bytes differ.
Job B is given its geometry, paper code and image from job A, so what that comparison tests is the
framing - chunk sequence, item sets, item order, padding - and not the numbers. Job C draws its
own bitmaps, so comparing it with job A tests the drawing as well: it is how the CLI's page table
was found to carry the same wrong Legal height the Python one did.

The numbers are the other half. What a test page claims about the page - the paper code, the
raster geometry, the resolution - is compared against the job the filter builds from a real CUPS
raster of that same page size, which is the raster cgpdftoraster produces for a document. That is
where the table in make-test-job.py was found to be giving Legal the paper's height, 8100 px,
rather than the imageable area's 8200 px that a real job carries.

Usage:
  scripts/bringup-compare.py [--keep DIR]
"""

import argparse
import filecmp
import importlib.util
import os
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
CLI = os.path.join(REPO, 'target/release/p1102')
FILTER = os.path.join(REPO, 'target/release/p1102-filter')
MAKE_JOB = os.path.join(HERE, 'make-test-job.py')
ORACLE = os.path.join(HERE, 'jbig-oracle.sh')
PPD = '/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd'
FIXTURE = os.path.join(REPO, 'fixtures/inputs/text-1p.txt')

# Every pattern on the page a hardware session starts from, and one cheap pattern on the other
# page sizes, where what is being added is the paper code and the geometry rather than the
# picture. gray and checker are the two that take seconds: they are noisy, so the reference
# encoder has real work to do.
PATTERNS = ['blank', 'square', 'hline', 'vline', 'text123', 'border', 'gray', 'checker']
PAGES = ['letter', 'a4', 'legal']

# The items that describe the page rather than the picture.
PAGE_ITEMS = ['DMPAPER', 'RASTER_X', 'RASTER_Y', 'RESOLUTION_X', 'RESOLUTION_Y']


def load(name):
    """Import one of the sibling tools so its logic is reused rather than duplicated."""
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(cmd, **kw):
    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, **kw)
    if proc.returncode != 0:
        raise SystemExit('%s failed:\n%s'
                         % (' '.join(cmd), proc.stdout.decode('utf-8', 'replace')))
    return proc.stdout.decode('utf-8', 'replace')


def page_items(path, zc):
    """The START_PAGE item map of a one-page job, by item name."""
    with open(path, 'rb') as f:
        view = zc.semantic_view(f.read())
    for kind, event in view['events']:
        if kind == 'chunk' and event['op'] == 'START_PAGE':
            return event['items']
    raise SystemExit('no START_PAGE in %s' % path)


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--keep', help='keep the jobs in this directory')
    args = ap.parse_args(argv)

    for tool, how in ((CLI, 'the CLI'), (FILTER, 'the filter')):
        if not os.path.exists(tool):
            raise SystemExit('%s is not built: scripts/build-native.sh' % how)
    zc = load('zjs-compare')

    work = args.keep or tempfile.mkdtemp(prefix='p1102-bringup-')
    failures = 0

    print('the bring-up job against the shipped framing, for the same page image')
    cases = [(page, pattern) for page in PAGES
             for pattern in (PATTERNS if page == 'letter' else ['square'])]
    for page, pattern in cases:
        pbm = os.path.join(work, '%s-%s.pbm' % (page, pattern))
        ours = os.path.join(work, '%s-%s.ours.zjs' % (page, pattern))
        run([sys.executable, MAKE_JOB, '--pattern', pattern, '--page', page,
             '--keep-pbm', pbm, '-o', ours, '--verify'])

        # The same image, from the same reference encoder, framed by the Rust side instead.
        jbg = pbm + '.jbg'
        run([ORACLE, 'encode', pbm, jbg])
        with open(jbg, 'rb') as f:
            bie = f.read()
        bih = os.path.join(work, 'page.bih')
        bid = os.path.join(work, 'page.bid')
        with open(bih, 'wb') as f:
            f.write(bie[:zc.JBIG_BIH_LEN])
        with open(bid, 'wb') as f:
            f.write(bie[zc.JBIG_BIH_LEN:])

        items = page_items(ours, zc)
        wrapped = os.path.join(work, '%s-%s.wrap.zjs' % (page, pattern))
        run([CLI, 'wrap', '--bih', bih, '--bid', bid,
             '--width', str(items['RASTER_X']), '--height', str(items['RASTER_Y']),
             '--resolution', str(items['RESOLUTION_X']),
             '--paper-code', str(items['DMPAPER']), '-o', wrapped])

        # The third implementation: the CLI draws the same patterns in Rust and frames them
        # itself, so it has to produce job A rather than merely an equivalent job.
        cli_job = os.path.join(work, '%s-%s.cli.zjs' % (page, pattern))
        run([CLI, 'test-page', pattern, '--page', page, '-o', cli_job])

        shown = []
        for name, other in (('wrap', wrapped), ('test-page', cli_job)):
            if filecmp.cmp(ours, other, shallow=False):
                shown.append('%s identical' % name)
                continue
            proc = subprocess.run([sys.executable, os.path.join(HERE, 'zjs-compare.py'), ours,
                                   other, '--ignore-jobattr'],
                                  stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if proc.returncode == 0:
                shown.append('%s the same job, modulo the clock' % name)
                continue
            shown.append('%s DIFFERENT' % name)
            failures += 1
            print(proc.stdout.decode('utf-8', 'replace'), file=sys.stderr)
        print('  %-10s %-8s %s' % (pattern, page, ', '.join(shown)))

    print('the page each size claims, against the raster CUPS produces for it')
    for page in PAGES:
        raster = os.path.join(work, '%s.raster' % page)
        with open(raster, 'wb') as out:
            proc = subprocess.run(['/usr/sbin/cupsfilter', '-m', 'application/vnd.cups-raster',
                                   '-p', PPD, '-o', 'PageSize=' + page.title(), FIXTURE],
                                  stdout=out, stderr=subprocess.PIPE)
        if proc.returncode != 0:
            raise SystemExit('cupsfilter failed for %s:\n%s'
                             % (page, proc.stderr.decode('utf-8', 'replace')))
        real = os.path.join(work, '%s.real.zjs' % page)
        with open(real, 'wb') as out:
            proc = subprocess.run([FILTER, '1', 'manu', 'bringup', '1', '', raster],
                                  stdout=out, stderr=subprocess.PIPE)
        if proc.returncode != 0:
            raise SystemExit('the filter failed on the %s raster:\n%s'
                             % (page, proc.stderr.decode('utf-8', 'replace')))

        test_job = os.path.join(work, '%s-test.zjs' % page)
        run([sys.executable, MAKE_JOB, '--pattern', 'square', '--page', page, '-o', test_job])
        from_tool = page_items(test_job, zc)
        from_raster = page_items(real, zc)
        wrong = [name for name in PAGE_ITEMS if from_tool.get(name) != from_raster.get(name)]
        if wrong:
            failures += 1
            print('  %-10s %-8s DIFFERENT' % ('page', page), file=sys.stderr)
            for name in wrong:
                print('    %s: the test page says %s, a real page says %s'
                      % (name, from_tool.get(name), from_raster.get(name)), file=sys.stderr)
            continue
        print('  %-10s %-8s %s' % ('page', page,
                                   ' '.join('%s=%s' % (n, from_raster[n]) for n in PAGE_ITEMS)))

    if failures:
        print('%d comparison(s) failed' % failures, file=sys.stderr)
        return 1
    print('the bring-up tool and the shipped implementation build the same jobs, and the pages'
          ' they describe are the pages CUPS rasterises.')
    if args.keep:
        print('kept in %s' % work)
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
