#!/usr/bin/env python3
"""Compare two ZjStream jobs semantically rather than byte for byte.

The plan asks for exactly this: "they do not necessarily have to be byte-identical if
fields such as IDs/timestamps differ, but the semantic printer commands/raster data should
match. Where differences exist, explain them."

So this tool separates two kinds of difference:

  Transport detail, normalised away because it is not a printer command:
    - the PJL JOBATTR line, which carries a timestamp (use --ignore-jobattr)
    - how the compressed page image is divided between JBIG_BID chunks. A chunk boundary
      is a 64 KiB transport detail; the bi-level image entity is what the printer decodes.

  Semantic content, which must match and is compared by name:
    - every other PJL setting line
    - the ordered sequence of chunks, and the item set of each chunk by tag name
    - each page image: its BIH fields and its compressed body

Exit status is 0 when the two jobs are semantically identical, 1 when they differ, so it
can be used as a check.

Typical use, comparing HP's output with ours once a capture exists:

    scripts/zjs-compare.py captured-from-hp.zjs ours.zjs --ignore-jobattr

Usage:
  scripts/zjs-compare.py <a.zjs> <b.zjs> [--ignore-jobattr] [--show-normalised]
"""

import argparse
import hashlib
import struct
import sys

SYNC = b'JZJZ'
UEL = b'\x1b%-12345X'
CHUNK_HEADER = 16
JBIG_BIH_LEN = 20
CHUNK_SIGNATURE = 0x5A5A

OPCODES = {
    0: 'START_DOC', 1: 'END_DOC', 2: 'START_PAGE', 3: 'END_PAGE',
    4: 'JBIG_BIH', 5: 'JBIG_BID', 6: 'END_JBIG', 7: 'SIGNATURE',
    8: 'RAW_IMAGE', 9: 'START_PLANE', 10: 'END_PLANE',
    11: '2600N_PAUSE', 12: '2600N',
}

ITEM_NAMES = {
    0: 'PAGECOUNT', 1: 'DMCOLLATE', 2: 'DMDUPLEX', 3: 'DMPAPER', 4: 'DMCOPIES',
    5: 'DMDEFAULTSOURCE', 6: 'DMMEDIATYPE', 7: 'NBIE', 8: 'RESOLUTION_X',
    9: 'RESOLUTION_Y', 10: 'OFFSET_X', 11: 'OFFSET_Y', 12: 'RASTER_X',
    13: 'RASTER_Y', 14: 'COLLATE', 15: 'QUANTITY', 16: 'VIDEO_BPP',
    17: 'VIDEO_X', 18: 'VIDEO_Y', 19: 'INTERLACE', 20: 'PLANE', 21: 'PALETTE',
    22: 'RET', 23: 'ECONOMODE',
}

DUPLEX_MODES = {1: 'off', 2: 'long-edge', 3: 'short-edge', 4: 'manual-long', 5: 'manual-short'}

ITEM_TYPE_UINT32 = 1
ITEM_TYPE_STRING = 3


def opcode_name(value):
    return OPCODES.get(value, 'OP_%d' % value)


def item_name(tag):
    return ITEM_NAMES.get(tag, 'TAG_0x%04x' % tag)


def _lines(blob):
    # The prologue ends with a NUL and then the PJL UEL with no trailing newline, so split
    # on NUL as well to keep that last line readable.
    out = []
    for part in blob.replace(b'\x00', b'\n').split(b'\n'):
        if part.strip():
            out.append(part)
    return out


def split_framing(data):
    """Split a job into (prologue lines, chunk stream, epilogue lines)."""
    at = data.find(SYNC)
    if at < 0:
        raise SystemExit('no JZJZ signature found; is this a ZjStream job?')
    prologue = _lines(data[:at])
    rest = data[at:]
    end = rest.find(UEL + b'@PJL EOJ')
    if end < 0:
        return prologue, rest, []
    return prologue, rest[:end], _lines(rest[end:])


def parse_chunks(stream):
    pos = len(SYNC)
    chunks = []
    while pos < len(stream):
        if pos + CHUNK_HEADER > len(stream):
            raise SystemExit('truncated chunk header at %d' % pos)
        size, ty, items, reserved, sig = struct.unpack_from('>IIIHH', stream, pos)
        if sig != CHUNK_SIGNATURE:
            raise SystemExit('bad chunk signature %#06x at %d' % (sig, pos))
        if size < CHUNK_HEADER:
            raise SystemExit('chunk size %d too small at %d' % (size, pos))
        if pos + size > len(stream):
            raise SystemExit('chunk at %d overruns the stream' % pos)
        ip = pos + CHUNK_HEADER
        item_list = []
        for _ in range(items):
            if ip + 8 > pos + size:
                raise SystemExit('item header overruns chunk at %d' % ip)
            isize, tag, itype, param = struct.unpack_from('>IHBB', stream, ip)
            if isize < 8 or ip + isize > pos + size:
                raise SystemExit('item size %d invalid at %d' % (isize, ip))
            item_list.append((tag, itype, param, stream[ip + 8:ip + isize]))
            ip += isize
        chunks.append({
            'type': ty,
            'items': item_list,
            'reserved': reserved,
            'payload': stream[ip:pos + size],
        })
        pos += size
    if pos != len(stream):
        raise SystemExit('chunk stream does not end cleanly')
    return chunks


def items_map(item_list):
    out = {}
    for tag, itype, param, payload in item_list:
        name = item_name(tag)
        if itype == ITEM_TYPE_UINT32 and len(payload) == 4:
            value = struct.unpack('>I', payload)[0]
            if tag == 2:
                value = '%d (%s)' % (value, DUPLEX_MODES.get(value, '?'))
        elif itype == ITEM_TYPE_STRING:
            value = payload.split(b'\x00')[0].decode('latin1')
        else:
            value = '<%d bytes type %d>' % (len(payload), itype)
        if param:
            name += ' param=%d' % param
        out[name] = value
    return out


def bih_fields(payload):
    if len(payload) != JBIG_BIH_LEN:
        return {'malformed_bih': len(payload)}
    return {
        'DL': payload[0],
        'D': payload[1],
        'planes': payload[2],
        'XD': struct.unpack('>I', payload[4:8])[0],
        'YD': struct.unpack('>I', payload[8:12])[0],
        'L0': struct.unpack('>I', payload[12:16])[0],
        'MX': payload[16],
        'MY': payload[17],
        'order': payload[18],
        'options': payload[19],
    }


def semantic_view(data):
    """Normalise a job into an ordered list of semantic events."""
    prologue, stream, epilogue = split_framing(data)
    chunks = parse_chunks(stream)
    events = []
    i = 0
    while i < len(chunks):
        chunk = chunks[i]
        ty = chunk['type']

        if ty == 4:  # JBIG_BIH: absorb the following BID chunks into one image
            bih = chunk['payload']
            parts = []
            i += 1
            while i < len(chunks) and chunks[i]['type'] == 5:
                parts.append(chunks[i]['payload'])
                i += 1
            body = b''.join(parts)
            # the last BID chunk is zero padded to a 4-byte boundary; the reference
            # implementation already trims trailing zeros from the body itself, so
            # trailing zeros here are transport padding and not image data
            trimmed = body.rstrip(b'\x00')
            events.append(('image', {
                'bih': bih_fields(bih),
                'body_bytes': len(trimmed),
                'body_sha': hashlib.sha256(trimmed).hexdigest()[:16],
            }))
            continue

        if ty == 8:  # RAW_IMAGE
            payload = chunk['payload'].rstrip(b'\x00')
            events.append(('raw_image', {
                'bytes': len(payload),
                'sha': hashlib.sha256(payload).hexdigest()[:16],
            }))
            i += 1
            continue

        events.append(('chunk', {
            'op': opcode_name(ty),
            'items': items_map(chunk['items']),
            'reserved': chunk['reserved'],
            'item_count': len(chunk['items']),
        }))
        i += 1

    return {
        'prologue': prologue,
        'events': events,
        'epilogue': epilogue,
        'raw_bytes': len(data),
    }


def describe_events(view):
    out = []
    for kind, body in view['events']:
        if kind == 'chunk':
            items = ', '.join('%s=%s' % kv for kv in sorted(body['items'].items()))
            out.append('%-11s items=%d{%s}' % (body['op'], body['item_count'], items))
        elif kind == 'image':
            bih = body['bih']
            out.append('%-11s %s body=%d sha=%s'
                       % ('IMAGE', bih, body['body_bytes'], body['body_sha']))
        else:
            out.append('%-11s bytes=%d sha=%s' % ('RAW_IMAGE', body['bytes'], body['sha']))
    return out


def compare(a, b, ignore_jobattr):
    """Return a list of human-readable differences."""
    diffs = []

    def drop_jobattr(lines):
        if not ignore_jobattr:
            return lines
        return [ln for ln in lines if b'JOBATTR' not in ln]

    pa, pb = drop_jobattr(a['prologue']), drop_jobattr(b['prologue'])
    if pa != pb:
        diffs.append('PJL prologue differs:')
        only_a = [l for l in pa if l not in pb]
        only_b = [l for l in pb if l not in pa]
        for l in only_a:
            diffs.append('    only in a: %s' % l.decode('latin1'))
        for l in only_b:
            diffs.append('    only in b: %s' % l.decode('latin1'))
        if not only_a and not only_b:
            diffs.append('    same lines in a different order')

    if a['epilogue'] != b['epilogue']:
        diffs.append('PJL epilogue differs: %r vs %r'
                     % (a['epilogue'], b['epilogue']))

    ea, eb = a['events'], b['events']
    if len(ea) != len(eb):
        diffs.append('different number of semantic events: %d vs %d'
                     % (len(ea), len(eb)))
    for idx, (x, y) in enumerate(zip(ea, eb)):
        if x[0] != y[0]:
            diffs.append('event %d: %s vs %s' % (idx, x[0], y[0]))
            continue
        kind, bx, by = x[0], x[1], y[1]
        if kind == 'chunk':
            if bx['op'] != by['op']:
                diffs.append('event %d: chunk %s vs %s' % (idx, bx['op'], by['op']))
                continue
            for key in sorted(set(bx['items']) | set(by['items'])):
                va, vb = bx['items'].get(key), by['items'].get(key)
                if va != vb:
                    why = ''
                    if va is None:
                        why = '  (only b sends it)'
                    elif vb is None:
                        why = '  (only a sends it)'
                    diffs.append('event %d (%s): %s  a=%r b=%r%s'
                                 % (idx, bx['op'], key, va, vb, why))
            if bx['reserved'] != by['reserved']:
                # `reserved` is derived: it is 12 x the item count. Report it only when it
                # does not follow the item count in one of the jobs, otherwise adding an
                # option would produce a second, redundant difference line.
                derived = (bx['reserved'] == 12 * bx['item_count']
                           and by['reserved'] == 12 * by['item_count'])
                if not derived:
                    diffs.append('event %d (%s): reserved  a=%#06x b=%#06x'
                                 % (idx, bx['op'], bx['reserved'], by['reserved']))
        else:
            for key in sorted(set(bx) | set(by)):
                if bx.get(key) != by.get(key):
                    if key == 'bih' and isinstance(bx.get(key), dict):
                        for f in sorted(set(bx[key]) | set(by[key])):
                            if bx[key].get(f) != by[key].get(f):
                                diffs.append('event %d (%s): BIH %s  a=%r b=%r'
                                             % (idx, kind, f, bx[key].get(f), by[key].get(f)))
                    else:
                        diffs.append('event %d (%s): %s  a=%r b=%r'
                                     % (idx, kind, key, bx.get(key), by.get(key)))
    return diffs


def main(argv):
    ap = argparse.ArgumentParser(add_help=True,
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 description=__doc__)
    ap.add_argument('a')
    ap.add_argument('b')
    ap.add_argument('--ignore-jobattr', action='store_true',
                    help='ignore the JOBATTR line, which carries a timestamp')
    ap.add_argument('--show-normalised', action='store_true',
                    help='print the normalised semantic view of both jobs')
    args = ap.parse_args(argv)

    view_a = semantic_view(open(args.a, 'rb').read())
    view_b = semantic_view(open(args.b, 'rb').read())

    if args.show_normalised:
        print('--- %s (%d bytes) ---' % (args.a, view_a['raw_bytes']))
        for line in describe_events(view_a):
            print('  ' + line)
        print('--- %s (%d bytes) ---' % (args.b, view_b['raw_bytes']))
        for line in describe_events(view_b):
            print('  ' + line)
        print()

    diffs = compare(view_a, view_b, args.ignore_jobattr)

    print('%s (%d bytes) vs %s (%d bytes)' % (args.a, view_a['raw_bytes'], args.b, view_b['raw_bytes']))
    print('  events: %d vs %d' % (len(view_a['events']), len(view_b['events'])))
    if not diffs:
        print('  SEMANTICALLY IDENTICAL')
        if view_a['raw_bytes'] != view_b['raw_bytes']:
            print('  (byte sizes differ, but only in transport detail)')
        return 0
    print('  %d semantic difference(s):' % len(diffs))
    for d in diffs:
        print('  - ' + d)
    return 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
