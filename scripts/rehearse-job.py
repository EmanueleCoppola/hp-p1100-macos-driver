#!/usr/bin/env python3
"""Rehearse the printing pipeline the way CUPS will drive it, without a printer.

This is the closest thing to the real integration that can be done before the hardware
arrives, and it exercises the exact boundary CUPS uses:

    Apple's cgpdftoraster  ->  our filter  ->  job bytes
    (cupsfilter, native)       (p1102-filter)   (judged by the reference JBIG decoder)

Every piece of that is real: the raster comes from the same filter CUPS would run, the job
is produced by our actual filter binary with the argv shape CUPS uses, and the result is
decoded by the reference implementation of the standard rather than by our own code.

The filter is run from its native build when there is one - that is the artifact that would be
installed - and from the wasm build otherwise, which links with Rust's own bundled linker and
needs no SDK at all. See docs/testing.md.

Usage:
  scripts/rehearse-job.py [--options "Duplex=DuplexNoTumble"] [--text FILE] [--keep DIR]

Exit status 0 when the rehearsal passes, 1 when it does not.
"""

import argparse
import importlib.util
import os
import shutil
import struct
import subprocess
import sys
import tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.dirname(HERE)
NATIVE_FILTER = os.path.join(REPO, 'target/release/p1102-filter')
WASM_FILTER = os.path.join(REPO, 'target/wasm32-wasip1/debug/p1102-filter.wasm')
PPD = '/etc/cups/ppd/HP_LaserJet_Professional_P1102.ppd'


def load(name):
    """Import one of the sibling tools so its logic is reused rather than duplicated."""
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def filter_command(extra_env=None):
    """How to run the filter: the real native binary if it is built, else the wasm build.

    The native build is the artifact that would be installed, so it is preferred. The wasm
    build exists for machines where native linking is unavailable; see docs/testing.md.
    """
    if os.path.exists(NATIVE_FILTER):
        return [NATIVE_FILTER], 'native'
    return wasi_command(extra_env), 'wasm'


def wasi_command(extra_env=None):
    """Return the command prefix that runs a wasm module, or fail with advice.

    wasmtime does not inherit the host environment, so variables the filter reads have to
    be passed explicitly with --env. node's WASI takes them from process.env instead.
    """
    extra_env = extra_env or {}
    wt = os.environ.get('P1102_WASMTIME') or shutil.which('wasmtime')
    if wt:
        cmd = [wt, 'run', '--dir', '/']
        for key, value in extra_env.items():
            cmd += ['--env', '%s=%s' % (key, value)]
        return cmd
    node = shutil.which('node')
    if node:
        return ['node', os.path.join(HERE, 'wasi-run.mjs')]
    raise SystemExit('need wasmtime or node to run the wasm filter; see docs/testing.md')


def make_raster(text, out_path):
    if not os.path.exists(PPD):
        raise SystemExit('cannot find the printer PPD at ' + PPD)
    with open(out_path, 'wb') as dst:
        proc = subprocess.run(
            ['/usr/sbin/cupsfilter', '-m', 'application/vnd.cups-raster', '-p', PPD, text],
            stdout=dst, stderr=subprocess.PIPE)
    if proc.returncode != 0:
        raise SystemExit('cupsfilter failed:\n' + proc.stderr.decode('utf-8', 'replace'))
    return os.path.getsize(out_path)


class Lever(Exception):
    """An environment override that did not do what the bring-up notes claim."""


# What each PPD option must do to the job. The filter maps these names - CUPS passes the PPD
# spelling, not the IPP one - and the mapping is unit-tested and compared against the reference
# driver separately; this checks it once more through the whole binary, at the boundary where a
# name that no longer matches would silently do nothing.
OPTION_EFFECTS = [
    # Simplex is announced once, on the document; a duplex mode is announced on the page, the
    # image and the page end, which is what the reference driver does as well.
    ('Duplex=None', {'doc': {'DMDUPLEX': 1}}),
    ('Duplex=DuplexNoTumble', {'page': {'DMDUPLEX': 2}, 'image_end': {'DMDUPLEX': 2},
                               'end': {'DMDUPLEX': 2}}),
    ('Duplex=DuplexTumble', {'page': {'DMDUPLEX': 3}, 'image_end': {'DMDUPLEX': 3},
                             'end': {'DMDUPLEX': 3}}),
    ('InputSlot=Manual', {'page': {'DMDEFAULTSOURCE': 4}}),
    ('EconoMode=True', {'items': {'ECONOMODE': 1}, 'pjl': ['@PJL SET ECONOMODE=ON']}),
    ('pjlDensity=1', {'pjl': ['@PJL SET DENSITY=1']}),
    ('pjlDensity=5', {'pjl': ['@PJL SET DENSITY=5']}),
    ('pjlJamRecovery=True', {'pjl': ['@PJL SET JAMRECOVERY=AUTO']}),
    ('pjlJamRecovery=False', {'pjl': ['@PJL SET JAMRECOVERY=OFF']}),
]


def check_options(options, job, zc):
    """Assert the effect of every PPD option named on the command line, in the real job."""
    lines = []
    view = zc.semantic_view(job)

    def items_of(op):
        for kind, event in view['events']:
            if kind == 'chunk' and event['op'] == op:
                return event['items']
        return {}

    scopes = {'doc': items_of('START_DOC'), 'page': items_of('START_PAGE'),
              'image_end': items_of('END_JBIG'), 'end': items_of('END_PAGE')}
    for option, expected in OPTION_EFFECTS:
        if option not in options.split():
            continue
        for scope in ('doc', 'page', 'image_end', 'end'):
            for name, value in expected.get(scope, {}).items():
                got = scopes[scope].get(name)
                # The semantic view renders some items as labels ("2 (long-edge)"), so compare
                # the leading number: readable whatever a tool adds after it.
                got_number = str(got).split()[0] if got is not None else None
                if got_number != str(value):
                    raise Lever('%s should set %s to %s on the %s chunk, but the job has %r'
                                % (option, name, value, scope, got))
                lines.append('%s: %s carries %s=%s' % (option, scope, name, value))
        for needle in expected.get('pjl', []):
            if needle.encode() not in job:
                raise Lever('%s should put %r in the prologue' % (option, needle))
            lines.append('%s: the prologue says %s' % (option, needle))
    return lines


def check_levers(raster_path, options, work, zc):
    """Run the environment overrides the bring-up notes tell you to reach for.

    Each is a diagnostic someone will type on the first hardware session, so each is run here
    and its effect checked: a lever that quietly did nothing would waste that session, and
    until now only P1102_RAW had ever been exercised.
    """
    lines = []

    def job_with(overrides, name):
        path = os.path.join(work, 'lever-%s.zjs' % name)
        code, stderr, _ = run_filter(raster_path, options, path, env_overrides=overrides)
        if code != 0:
            raise Lever('the filter exited %d with %s: %s'
                        % (code, ' '.join('%s=%s' % kv for kv in overrides.items()), stderr))
        return open(path, 'rb').read()

    job = job_with({'P1102_NO_PJL': '1'}, 'no-pjl')
    if zc.semantic_view(job)['prologue'] or b'@PJL' in job or not job.startswith(zc.SYNC):
        raise Lever('P1102_NO_PJL=1 did not remove the wrapper: the job should start at the '
                    'JZJZ signature with no @PJL lines at all')
    lines.append('P1102_NO_PJL=1: the job starts at the signature, with no wrapper')

    job = job_with({'P1102_DENSITY': '5'}, 'density')
    if b'@PJL SET DENSITY=5\n' not in job:
        raise Lever('P1102_DENSITY=5 did not change the PJL density line')
    lines.append('P1102_DENSITY=5: the prologue carries DENSITY=5')

    job = job_with({'P1102_ECONOMODE': '1'}, 'econo')
    pages = [e[1]['items'] for e in zc.semantic_view(job)['events']
             if e[0] == 'chunk' and e[1]['op'] == 'START_PAGE']
    if not pages or pages[0].get('ECONOMODE') != 1:
        raise Lever('P1102_ECONOMODE=1 did not set the ECONOMODE item on the page')
    if b'@PJL SET ECONOMODE=ON\n' not in job:
        raise Lever('P1102_ECONOMODE=1 did not set the PJL line')
    lines.append('P1102_ECONOMODE=1: the page item and the PJL line both say so')
    lines.append('P1102_RAW=1: covered by the payload check when the rehearsal runs --raw')

    # CUPS hands a filter the queue's PPD in PPD and sends only the options the job named, so
    # the filter is expected to take the rest from the file. On this printer the visible one is
    # the media source: the PPD's default is Manual (4) where our built-in default is 1.
    if os.path.exists(PPD):
        items = {}
        for kind, event in zc.semantic_view(job_with({'PPD': PPD}, 'ppd-defaults'))['events']:
            if kind == 'chunk' and event['op'] == 'START_PAGE':
                items = event['items']
                break
        got = str(items.get('DMDEFAULTSOURCE')).split()[0]
        if got != '4':
            raise Lever('with PPD set, the media source should be the PPD default (Manual, 4), '
                        'but the job has %r' % items.get('DMDEFAULTSOURCE'))
        lines.append('PPD: the file\'s own default media source (Manual, 4) is used when the '
                     'command line says nothing')
    return lines


def ppd_media_type_codes(path):
    """The MediaType choices a PPD declares, and the cupsMediaType code each one sets."""
    codes = {}
    with open(path, encoding='latin1') as f:
        for line in f:
            if not line.startswith('*MediaType '):
                continue
            head, _, value = line.partition(':')
            choice = head.split(' ')[1].split('/')[0]
            _, _, after = value.partition('cupsMediaType')
            digits = ''
            for ch in after.lstrip():
                if not ch.isdigit():
                    break
                digits += ch
            if choice and digits:
                codes[choice] = int(digits)
    return codes


def check_media_types(raster_path, work, cr, zc):
    """Every MediaType this PPD declares arrives as the code the file declares.

    The code travels PPD -> PostScript page device -> raster header (cupsMediaType) ->
    ZJI_DMMEDIATYPE. scripts/reference-compare.py checks the whole path against the reference
    driver for three choices; this checks the header-to-item step for all of them, by patching
    the code into a small page cut out of the real raster so that the sweep costs nothing.
    """
    if not os.path.exists(PPD):
        return []
    codes = ppd_media_type_codes(PPD)
    if not codes:
        raise Lever('the PPD declares no MediaType choices, so the sweep would prove nothing')

    base = open(raster_path, 'rb').read()
    off = next(cr.walk(base))[1]
    width, height = 128, 8
    page = bytearray(base[:off + cr.PAGE_HEADER_LEN + width * height])
    struct.pack_into('<II', page, off + cr.CUPS_WIDTH, width, height)
    struct.pack_into('<I', page, off + cr.CUPS_BYTES_PER_LINE, width)

    for choice in sorted(codes, key=lambda c: codes[c]):
        value = codes[choice]
        data = bytearray(page)
        struct.pack_into('<I', data, off + cr.CUPS_MEDIA_TYPE, value)
        raster = os.path.join(work, 'media-%d.raster' % value)
        with open(raster, 'wb') as f:
            f.write(bytes(data))
        out = os.path.join(work, 'media-%d.zjs' % value)
        code, stderr, _ = run_filter(raster, '', out)
        if code != 0:
            raise Lever('the filter exited %d on MediaType %s (code %d): %s'
                        % (code, choice, value, stderr))
        items = {}
        for kind, event in zc.semantic_view(open(out, 'rb').read())['events']:
            if kind == 'chunk' and event['op'] == 'START_PAGE':
                items = event['items']
                break
        got = str(items.get('DMMEDIATYPE')).split()[0]
        if got != str(value):
            raise Lever('MediaType %s should reach the printer as %s, the job says %r'
                        % (choice, value, items.get('DMMEDIATYPE')))
    return ['all %d MediaType choices in the installed PPD arrive as the code it declares: %s'
            % (len(codes), ' '.join('%d=%s' % (codes[c], c)
                                    for c in sorted(codes, key=lambda c: codes[c])))]


# The option groups the filter consumes, and the ones whose effect CUPS has already put into
# the raster page header by the time the filter runs. docs/differences.md item 10 has the
# reasoning; this is the check that keeps the claim true. An argument can be made for a group
# being in either list, but it cannot be in neither: that would be a setting the queue offers
# and the printer never hears about, which is what the media source was before the PPD's own
# default was read.
FILTER_OPTIONS = ('Duplex', 'InputSlot', 'pjlDensity', 'pjlJamRecovery', 'EconoMode')
RASTER_OPTIONS = ('PageSize', 'PageRegion', 'MediaType', 'Resolutions')


def ppd_option_groups(path):
    """The option groups a PPD declares, in the order it declares them."""
    groups = []
    with open(path, encoding='latin1') as f:
        for line in f:
            if not line.startswith('*OpenUI'):
                continue
            rest = line[len('*OpenUI'):].strip()
            name = rest.lstrip('*').split('/')[0].strip()
            if name and name not in groups:
                groups.append(name)
    return groups


def check_ppd_option_coverage(ppd_path=None):
    """No option the queue offers can silently do nothing.

    Regenerating our PPD from a different HP PPD is the realistic way to break this - another
    bundle in the family offers other slots and other groups - so the check runs against
    whatever PPD is installed rather than against a list kept here.
    """
    path = ppd_path or PPD
    if not os.path.exists(path):
        return []
    groups = ppd_option_groups(path)
    if not groups:
        raise Lever('%s declares no *OpenUI groups, so this check would prove nothing' % path)
    unhandled = [g for g in groups if g not in FILTER_OPTIONS and g not in RASTER_OPTIONS]
    if unhandled:
        raise Lever('%s offers %s, and neither the filter nor the raster header carries them: '
                    'a job that sets one would be accepted and ignored'
                    % (path, ', '.join(unhandled)))
    return ['%d option groups in %s, every one of them mapped by the filter (%s) or carried by '
            'the raster header (%s)'
            % (len(groups), path, ', '.join(g for g in groups if g in FILTER_OPTIONS),
               ', '.join(g for g in groups if g in RASTER_OPTIONS))]


def run_filter(raster_path, options, out_path, raw=False, env_overrides=None):
    overrides = {'P1102_RAW': '1'} if raw else {}
    if env_overrides:
        overrides = dict(overrides, **env_overrides)
    prefix, route = filter_command(overrides)
    if route == 'native':
        cmd = prefix + ['1', 'manu', 'rehearsal', '1', options, raster_path]
    else:
        cmd = prefix + [WASM_FILTER, '1', 'manu', 'rehearsal', '1', options, raster_path]
    env = dict(os.environ)
    env.update(overrides)
    with open(out_path, 'wb') as dst:
        proc = subprocess.run(cmd, stdout=dst, stderr=subprocess.PIPE, env=env)
    return proc.returncode, proc.stderr.decode('utf-8', 'replace').strip(), route

def run_filter_on_stdin(raster_path, options, out_path, raw=False):
    """The same job, with the raster on stdin and no input file argument.

    CUPS passes argv[6] for a filter chain, but the filter also accepts its input on stdin
    (it is how the tool is used by hand), and the two must produce the same bytes - a
    difference there would mean the streaming reader depends on it being seekable.
    """
    overrides = {'P1102_RAW': '1'} if raw else {}
    prefix, route = filter_command(overrides)
    cmd = prefix if route == 'native' else prefix + [WASM_FILTER]
    cmd = cmd + ['1', 'manu', 'rehearsal', '1', options]
    env = dict(os.environ)
    env.update(overrides)
    with open(raster_path, 'rb') as src, open(out_path, 'wb') as dst:
        proc = subprocess.run(cmd, stdin=src, stdout=dst, stderr=subprocess.PIPE, env=env)
    return proc.returncode, proc.stderr.decode('utf-8', 'replace').strip()


def expected_pages(raster_bytes, cr):
    """The page bitmaps an independent computation says the raster represents.

    Covers the two depths the filter converts: 8 bits per pixel grey, thresholded, and
    1 bit per pixel, copied through.
    """
    out = []
    for _, off, h, payload in cr.walk(raster_bytes):
        width = (h['width'] + 127) & ~127
        stride = width // 8
        bpl = h['bytes_per_line']
        px = raster_bytes[off + cr.PAGE_HEADER_LEN:off + cr.PAGE_HEADER_LEN + payload]
        bm = bytearray(stride * h['height'])
        ink = 0
        depth = h['bits_per_pixel']
        if depth == 8:
            for y in range(h['height']):
                src = px[y * bpl:(y + 1) * bpl]
                dst = y * stride
                for x in range(h['width']):
                    if src[x] < 128:
                        bm[dst + (x >> 3)] |= 0x80 >> (x & 7)
                        ink += 1
        elif depth == 1:
            copy = min(bpl, stride)
            for y in range(h['height']):
                row = px[y * bpl:y * bpl + copy]
                bm[y * stride:y * stride + copy] = row
                ink += sum(bin(b).count('1') for b in row)
        else:
            raise SystemExit('the expected-bitmap computation does not handle %d bpp' % depth)
        out.append((width, h['height'], bytes(bm), ink))
    return out


def page_images(chunks):
    """Each page's compressed image as a complete BIE, in page order."""
    images = []
    i = 0
    while i < len(chunks):
        if chunks[i]['type'] == 4:
            bih = chunks[i]['payload']
            parts = []
            i += 1
            while i < len(chunks) and chunks[i]['type'] == 5:
                parts.append(chunks[i]['payload'])
                i += 1
            images.append(bih + b''.join(parts).rstrip(b'\x00'))
            continue
        i += 1
    return images


def raw_images(chunks):
    """Each page's uncompressed image payload, in page order."""
    return [c['payload'] for c in chunks if c['type'] == 8]



def main(argv):
    ap = argparse.ArgumentParser(add_help=True,
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 description=__doc__)
    ap.add_argument('--options', default='',
                    help='CUPS options string, exactly as cupsd would pass it')
    ap.add_argument('--text', default=os.path.join(REPO, 'fixtures/inputs/text-1p.txt'))
    ap.add_argument('--raster', help='rehearse an existing CUPS raster file instead of '
                                     'generating one with cupsfilter')
    ap.add_argument('--keep', help='keep the intermediate files in this directory')
    ap.add_argument('--raw', action='store_true',
                    help='rehearse the uncompressed ZJT_RAW_IMAGE path instead')
    args = ap.parse_args(argv)

    if not (os.path.exists(NATIVE_FILTER) or os.path.exists(WASM_FILTER)):
        return fail('build the filter first:\n'
                    '    scripts/build-native.sh\n'
                    '  or, where native linking is unavailable:\n'
                    '    cargo build --target wasm32-wasip1 -p p1102-filter --offline')

    cr = load('cupsraster')
    zc = load('zjs-compare')

    work = args.keep or tempfile.mkdtemp(prefix='p1102-rehearsal-')
    os.makedirs(work, exist_ok=True)
    raster_path = os.path.join(work, 'page.raster')
    job_path = os.path.join(work, 'job.zjs')

    print('rehearsing the CUPS -> filter boundary')
    print('  options: %r' % args.options)
    if args.raster:
        raster_path = args.raster
        print('  1. rehearsing the captured raster %s (%d bytes)'
              % (raster_path, os.path.getsize(raster_path)))
    else:
        size = make_raster(args.text, raster_path)
        print('  1. cgpdftoraster (Apple, native) produced a %d byte CUPS raster' % size)

    code, stderr, route = run_filter(raster_path, args.options, job_path, raw=args.raw)
    if code != 0:
        return fail('the filter exited %d\n%s' % (code, stderr))
    job = open(job_path, 'rb').read()
    print('  2. our %s filter produced a %d byte job' % (route, len(job)))
    for line in stderr.splitlines():
        print('       %s' % line)

    # The other input route, checked against this one: the same raster on stdin, with no file
    # argument. Compared semantically because the JOBATTR stamp can differ by a second.
    stdin_path = os.path.join(work, 'stdin.zjs')
    code, stdin_stderr = run_filter_on_stdin(raster_path, args.options, stdin_path,
                                             raw=args.raw)
    if code != 0:
        return fail('the filter exited %d when the raster arrives on stdin\n%s'
                    % (code, stdin_stderr))
    diffs = zc.compare(zc.semantic_view(job),
                       zc.semantic_view(open(stdin_path, 'rb').read()), True)
    if diffs:
        return fail('the job differs when the raster arrives on stdin: %s' % diffs)
    print('       the same job comes out with the raster on stdin')

    # And that each option CUPS would pass actually did something, checked on this job.
    try:
        for line in check_options(args.options, job, zc):
            print('       %s' % line)
    except Lever as exc:
        return fail(str(exc))

    _, stream, _ = zc.split_framing(job)
    chunks = zc.parse_chunks(stream)
    ops = [zc.opcode_name(c['type']) for c in chunks]
    print('  3. chunk sequence: %s' % ' '.join(ops))

    if args.raw:
        images = raw_images(chunks)
        kind = 'raw'
        if not images:
            return fail('no ZJT_RAW_IMAGE chunk in the job; the filter was not put in raw mode')
    else:
        images = page_images(chunks)
        kind = 'compressed'

    pages = expected_pages(open(raster_path, 'rb').read(), cr)
    if len(images) != len(pages):
        return fail('the job carries %d %s image(s) but the raster has %d page(s)'
                    % (len(images), kind, len(pages)))
    print('  4. the job carries %d %s page image(s) for %d raster page(s)'
          % (len(images), kind, len(pages)))

    for n, ((width, height, expected, ink), image) in enumerate(zip(pages, images), start=1):
        if args.raw:
            if image != expected:
                return fail('page %d: the raw image does not match the bitmap the raster '
                            'describes' % n)
            print('  5. page %d: raw payload equals the expected %dx%d bitmap (%d ink pixels)'
                  % (n, width, height, ink))
            continue
        bie_n = os.path.join(work, 'image-%d.jbg' % n)
        dec_n = os.path.join(work, 'decoded-%d.pbm' % n)
        exp_n = os.path.join(work, 'expected-%d.pbm' % n)
        with open(bie_n, 'wb') as f:
            f.write(image)
        rc = subprocess.run([os.path.join(HERE, 'jbig-oracle.sh'), 'decode', bie_n, dec_n])
        if rc.returncode != 0:
            return fail('page %d: the reference JBIG decoder rejected our image' % n)
        with open(exp_n, 'wb') as f:
            f.write(b'P4\n%d %d\n' % (width, height) + expected)
        rc = subprocess.run([sys.executable, os.path.join(HERE, 'pbm-compare.py'),
                             exp_n, dec_n], stdout=subprocess.PIPE)
        print('  5. page %d: reference decoder vs the independently computed bitmap: %s'
              % (n, rc.stdout.decode().strip()))
        if rc.returncode != 0:
            return fail('page %d: the decoded page does not match the raster' % n)

    print()
    print('  6. bring-up levers, the overrides the hardware notes tell you to reach for:')
    try:
        for line in check_levers(raster_path, args.options, work, zc):
            print('       %s' % line)
    except Lever as exc:
        return fail(str(exc))

    print()
    print('  7. media type codes, every choice the installed PPD declares:')
    try:
        for line in check_media_types(raster_path, work, cr, zc):
            print('       %s' % line)
    except Lever as exc:
        return fail(str(exc))

    print()
    print('  8. PPD option coverage, so no setting is accepted and ignored:')
    try:
        for line in check_ppd_option_coverage():
            print('       %s' % line)
    except Lever as exc:
        return fail(str(exc))

    # The filter is not a friendly CLI - CUPS always passes seven arguments - but a typo when
    # running it by hand should not look like a failed job, which is what it did before the
    # guard existed: half a job on stdout and then an error about the raster.
    prefix, route = filter_command()
    base = prefix if route == 'native' else prefix + [WASM_FILTER]
    for argv, want_code in ((['--help'], 0), ([], 2)):
        proc = subprocess.run(base + argv, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                              stdin=subprocess.DEVNULL)
        # What must never appear is a job: neither the PJL exit sequence nor the stream
        # signature. --help may write its usage text there, which is what help is for.
        wrote_job = proc.stdout.startswith(b'\x1b%-12345X') or proc.stdout.startswith(zc.SYNC)
        if proc.returncode != want_code or wrote_job:
            return fail('the filter with argv %r exited %d and wrote %d bytes to stdout '
                        '(a job: %s); expected exit %d and no job'
                        % (argv, proc.returncode, len(proc.stdout), wrote_job, want_code))
        if argv == ['--help'] and not proc.stdout.startswith(b'usage:'):
            return fail('--help should print its usage text on stdout, got %r'
                        % proc.stdout[:40])
    print('       invocation guard: --help exits 0 and a short argv exits 2, neither writes a job')
    print()
    print('rehearsal passed%s' % ('' if args.keep else ' (temporary files removed)'))
    if args.keep:
        print('intermediates kept in %s' % work)
    else:
        # only ever remove the scratch directory we created: with --raster, raster_path is
        # the caller's own file and must survive
        for name in os.listdir(work):
            os.unlink(os.path.join(work, name))
        os.rmdir(work)
    return 0


def fail(message):
    print('REHEARSAL FAILED: %s' % message, file=sys.stderr)
    return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
