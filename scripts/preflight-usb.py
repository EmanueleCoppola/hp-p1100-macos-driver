#!/usr/bin/env python3
"""Read-only pre-flight report for the first time the P1102 is attached.

Nothing in this repository has been validated on hardware, so the first attachment is
also the moment when a wrong assumption costs the most. This script answers, in one
read-only run and with no privileges, the questions that decide what to do next:

  - is a USB Printer-class device attached, and is it Hewlett-Packard 03f0:...?
  - does its serial number match the one the CUPS queue's device URI asks for?
  - which interfaces does it present, and how many endpoints do they have?
  - do the filters the queue names exist, and can this machine run them at all?

`system_profiler SPUSBDataType` is not used: it returns nothing on this machine.
`ioreg -a` returns the registry as a plist, which is exact, where the text form
truncates long values and repeats matched personalities.

Run it either side of a power cycle, or around a firmware-upload attempt, and diff the
JSON dumps to see whether the printer re-enumerates with a different identity:

    PY=/opt/homebrew/bin/python3.13          # /usr/bin/python3 is gated, see README
    $PY scripts/preflight-usb.py --json > /tmp/before.json
    # power cycle the printer here
    $PY scripts/preflight-usb.py --json > /tmp/after.json
    diff /tmp/before.json /tmp/after.json

Usage:
  scripts/preflight-usb.py [--json] [--all]

  --json  machine-readable dump of everything collected (for the diff above)
  --all   list every USB device, not just printers and hubs

Exit status: 0 when a printer-class device is attached, 1 when none is, 2 on usage or
registry failure. The serial comparison is reported, not returned.
"""

import argparse
import contextlib
import importlib.util
import io
import json
import os
import plistlib
import subprocess
import sys
import urllib.parse

HERE = os.path.dirname(os.path.abspath(__file__))

HP_VENDOR_ID = 0x03F0
PRINTER_CLASS = 7  # bDeviceClass / bInterfaceClass for USB Printer
HUB_CLASS = 9

SPEEDS = {1: 'low (1.5 Mb/s)', 2: 'full (12 Mb/s)', 3: 'high (480 Mb/s)',
          4: 'super (5 Gb/s)', 5: 'super+ (10 Gb/s)'}


def load(name):
    """Import a sibling tool so its logic is reused rather than duplicated."""
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + '.py'))
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


def run(cmd):
    proc = subprocess.run(cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    return (proc.returncode, proc.stdout.decode('utf-8', 'replace'),
            proc.stderr.decode('utf-8', 'replace'))


def usb_devices(source=None):
    """Every USB device, each with the interfaces that belong to it.

    Interfaces are siblings of their device in the registry, not children, so they are
    matched back by locationID, which both carry. Endpoints are not registry entries at
    all: only their count (bNumEndpoints) is exposed, which is why the report shows that
    rather than inventing addresses.

    `source` is a saved `ioreg -a` dump, for reviewing a machine that is not this one;
    without it the live registry is read.
    """
    if source is not None:
        tree = plistlib.loads(source)
    else:
        rc, out, err = run(['ioreg', '-r', '-c', 'IOUSBHostDevice', '-l', '-w0', '-a'])
        if rc != 0 or not out.strip():
            raise SystemExit('ioreg failed: %s' % (err.strip() or 'no output'))
        tree = plistlib.loads(out.encode('utf-8'))

    devices = {}
    interfaces = {}

    def walk(node):
        eid = node.get('IORegistryEntryID')
        if eid is not None:
            if 'bDeviceClass' in node and 'idVendor' in node:
                devices[eid] = node
            elif 'bInterfaceClass' in node:
                interfaces.setdefault(node.get('locationID'), []).append(node)
        for child in node.get('IORegistryEntryChildren') or []:
            walk(child)

    for root in tree:
        walk(root)

    found = []
    for eid, node in devices.items():
        found.append({
            'name': node.get('USB Product Name') or node.get('kUSBProductString')
                    or node.get('IORegistryEntryName'),
            'vendor_name': node.get('USB Vendor Name') or node.get('kUSBVendorString'),
            'vid': node.get('idVendor'),
            'pid': node.get('idProduct'),
            'serial': node.get('USB Serial Number') or node.get('bUSB Serial Number'),
            'bcdUSB': node.get('bcdUSB'),
            'class': node.get('bDeviceClass'),
            'subclass': node.get('bDeviceSubClass'),
            'protocol': node.get('bDeviceProtocol'),
            'speed': node.get('USBSpeed'),
            'link_bps': node.get('UsbLinkSpeed'),
            'location': node.get('locationID'),
            'entry_id': eid,
            'exclusive_owner': node.get('UsbExclusiveOwner'),
            'interfaces': [{
                'class': i.get('bInterfaceClass'),
                'subclass': i.get('bInterfaceSubClass'),
                'protocol': i.get('bInterfaceProtocol'),
                'endpoints': i.get('bNumEndpoints'),
                'driver': i.get('IOObjectClass'),
            } for i in interfaces.get(node.get('locationID'), [])],
        })
    found.sort(key=lambda d: (d['vid'] or 0, d['pid'] or 0, d['location'] or 0))
    return found


def is_printer(dev):
    return (dev['class'] == PRINTER_CLASS
            or any(i['class'] == PRINTER_CLASS for i in dev['interfaces'])
            or dev['vid'] == HP_VENDOR_ID)


def queue_names_device(queue, device):
    """Does this queue's device URI name this device?

    The serial comparison would otherwise fire for every USB queue on the machine: a Samsung
    or Epson queue's serial is always going to differ from an HP printer's, and reporting that
    as a mismatch is noise at exactly the moment someone is reading the report to decide what
    to do next.
    """
    uri = urllib.parse.unquote(queue['uri']).lower()
    product = ' '.join((device['name'] or '').lower().split())
    return bool(product) and product in uri


def arch_of(path):
    """Architectures in a Mach-O file, or a short reason there are none to report."""
    try:
        with open(path, 'rb') as f:
            data = f.read()
    except OSError:
        return None
    macho = load('macho')
    archs = []
    for arch, offset, _ in macho.slices(data):
        mach = macho.parse_mach(data, offset, arch != 'thin')
        if not mach:
            return 'not a Mach-O'
        archs.append(mach['arch'])
    return '+'.join(archs) if archs else 'empty'


def ppd_of(queue):
    path = '/etc/cups/ppd/%s.ppd' % queue
    info = {'ppd': path, 'filters': [], 'device_ids': []}
    if not os.path.exists(path):
        info['ppd_missing'] = True
        return info
    with open(path, 'r', errors='replace') as f:
        text = f.read()
    for line in text.splitlines():
        if line.startswith('*cupsFilter:') or line.startswith('*cupsFilter2:'):
            parts = line.split()
            if parts:
                info['filters'].append(parts[-1].strip('"'))
        elif line.startswith('*1284DeviceID:'):
            # A series PPD carries one of these per model it covers - hp1100.ppd has six,
            # P1102 through P1109 - and CUPS picks the model by matching the device's own
            # 1284 identity against them. All of them are reported, not just the last.
            info['device_ids'].append(line.split(':', 1)[1].strip().strip('"'))
    for path in info['filters']:
        info.setdefault('filter_arch', {})[path] = (
            arch_of(path) if os.path.exists(path) else 'missing')
    return info


def cups_queues():
    """Every CUPS queue backed by a USB device, with the PPD that will drive it."""
    rc, out, _ = run(['lpstat', '-v'])
    if rc != 0:
        return []
    queues = []
    for line in out.splitlines():
        if not line.startswith('device for '):
            continue
        name, sep, uri = line[len('device for '):].partition(':')
        if not sep:
            continue
        uri = uri.strip()
        if not uri.startswith('usb://'):
            continue
        entry = {'queue': name.strip(), 'uri': uri}
        entry['serial'] = urllib.parse.parse_qs(
            urllib.parse.urlparse(uri).query).get('serial', [None])[0]
        entry.update(ppd_of(entry['queue']))
        queues.append(entry)
    return queues


def human(devices, queues, show_all):
    printers = [d for d in devices if is_printer(d)]

    print('P1102 pre-flight (read-only)')
    print()
    print('CUPS queues on a USB device:')
    if not queues:
        print('  none')
    for q in queues:
        print('  %s' % q['queue'])
        print('    device   %s' % q['uri'])
        print('    wants    serial %s' % (q['serial'] or '(none in the URI)'))
        print('    ppd      %s%s' % (q['ppd'], ' (missing)' if q.get('ppd_missing') else ''))
        if q['device_ids']:
            print('    id       1284DeviceID of the PPD (%d models):' % len(q['device_ids']))
            for did in q['device_ids']:
                print('             %s' % did)
        for path, arch in sorted(q.get('filter_arch', {}).items()):
            note = ''
            if arch == 'missing':
                note = '  <- missing'
            elif arch and 'arm64' not in arch:
                note = '  <- Intel-only, cannot run here'
            print('    filter   %-8s %s%s' % (arch or '?', path, note))
    print()

    print('USB devices: %d attached' % len(devices))
    for d in devices:
        hub = d['class'] == HUB_CLASS
        if not show_all and not is_printer(d) and not hub:
            continue
        print('  %s  %04x:%04x%s' % (d['name'] or '(no product string)',
                                      d['vid'] or 0, d['pid'] or 0,
                                      '  [hub]' if hub else ''))
        if hub and not show_all:
            continue
        print('    vendor   %s' % (d['vendor_name'] or '(none)'))
        print('    serial   %s' % (d['serial'] or '(none)'))
        print('    class    %d/%d/%d   usb %s   %s' % (
            d['class'] or 0, d['subclass'] or 0, d['protocol'] or 0,
            '%x.%02x' % ((d['bcdUSB'] or 0) >> 8, (d['bcdUSB'] or 0) & 0xFF)
            if d['bcdUSB'] else '?',
            SPEEDS.get(d['speed'], 'speed %s' % d['speed'])))
        print('    location %s   entry %s' % (hex(d['location'] or 0), d['entry_id']))
        if d['exclusive_owner']:
            print('    claimed exclusively by %s  <- nothing else can open it'
                  % d['exclusive_owner'])
        for i in d['interfaces']:
            kind = 'Printer' if i['class'] == PRINTER_CLASS else 'class %s' % i['class']
            print('    iface    %s/%s/%s  %s endpoint(s)  %s' % (
                i['class'], i['subclass'], i['protocol'], i['endpoints'], kind))
    if not show_all:
        hidden = [d for d in devices if not is_printer(d) and d['class'] != HUB_CLASS]
        if hidden:
            print('  (%d other device(s) not shown; use --all)' % len(hidden))
    print()

    print('Verdict:')
    if not printers:
        print('  no USB Printer-class device and no Hewlett-Packard device is attached.')
        print('  Attach the P1102 (and switch it on) and run this again.')
    for d in printers:
        print('  attached: %s  %04x:%04x' % (d['name'], d['vid'] or 0, d['pid'] or 0))
        ifac = [i for i in d['interfaces'] if i['class'] == PRINTER_CLASS]
        if d['class'] == PRINTER_CLASS:
            print('  the device itself declares class 7 (Printer)')
        for i in ifac:
            bidir = 'bidirectional' if i['protocol'] == 2 else 'unidirectional'
            print('  interface %s/%s/%s (Printer, %s), %s endpoint(s)' % (
                i['class'], i['subclass'], i['protocol'], bidir, i['endpoints']))
            if i['endpoints'] is not None and i['endpoints'] < 2:
                print('    fewer than two bulk endpoints: the backchannel will not work')
        for q in queues:
            if not queue_names_device(q, d):
                continue
            if q['serial'] and d['serial']:
                same = q['serial'] == d['serial']
                print('  queue %s asks for serial %s: %s' % (
                    q['queue'], q['serial'], 'matches' if same else 'DIFFERENT'))
                if not same:
                    print('    the queue points at a device that is not the attached one;'
                          ' the usb backend will not find it')
            elif q['serial'] and not d['serial']:
                print('  queue %s asks for a serial, the device reports none' % q['queue'])
        if queues and not any(queue_names_device(q, d) for q in queues):
            print('  no queue on this machine names this device, so none of them can print to'
                  ' it as configured')
    print()
    print('  if the device re-enumerated, its entry id above differs from the previous')
    print('  report; diff two --json runs to see exactly what changed.')


def synthetic_registry(product='HP LaserJet Professional P1102', serial=None,
                       vendor_id=HP_VENDOR_ID, product_id=0x3D17, printer_interface=True):
    """A registry shaped the way `ioreg -a` returns one, for use without a printer.

    Only the fields the report reads are here, and the layout is the part that matters:
    interfaces are *siblings* of their device rather than children, and both carry the
    locationID that ties them together.
    """
    location = 0x1420000
    device = {
        'IORegistryEntryID': 4294967296,
        'IORegistryEntryName': product,
        'USB Product Name': product,
        'USB Vendor Name': 'Hewlett-Packard',
        'idVendor': vendor_id,
        'idProduct': product_id,
        'bDeviceClass': 0,
        'bDeviceSubClass': 0,
        'bDeviceProtocol': 0,
        'bcdUSB': 512,
        'USBSpeed': 3,
        'locationID': location,
        'IORegistryEntryChildren': [],
    }
    if serial:
        device['USB Serial Number'] = serial
    interface = {
        'IORegistryEntryID': 4294967297,
        'IOObjectClass': 'IOUSBHostInterface',
        'idVendor': vendor_id,
        'idProduct': product_id,
        'locationID': location,
        'bInterfaceClass': PRINTER_CLASS if printer_interface else 3,
        'bInterfaceSubClass': 1,
        'bInterfaceProtocol': 2,
        'bNumEndpoints': 2,
    }
    return plistlib.dumps([{'IORegistryEntryID': 1,
                            'IORegistryEntryChildren': [device, interface]}])


def capture(func, *args):
    """Run a report function and return what it printed."""
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        func(*args)
    return buf.getvalue()


def report_from_dump(raw):
    """Read a file passed to --from: a registry dump, or one of this script's own reports.

    `--json` writes the parsed result rather than the registry, so a saved report can be
    re-rendered later - or sent to someone whose printer is elsewhere - without the device
    being present. A registry dump is tried first, because that is what the tool reads from
    the system. Returning None means "this is a registry; parse it as one"."""
    try:
        plistlib.loads(raw)
        return None
    except Exception:
        pass
    try:
        data = json.loads(raw.decode('utf-8'))
    except Exception as exc:
        raise SystemExit('cannot read the dump: %s' % exc)
    if 'devices' not in data:
        raise SystemExit('that file is neither a registry dump nor a report from this script')
    return data


def selftest():
    """Check the report against a registry built by hand, because a printer may not be here.

    The tool exists for the first moment the printer is attached, so this runs its whole
    success path - device found, interface matched, serial compared, verdict written - on data
    shaped like a real dump. Without it, the first hardware session would also be the first
    time this code ever ran.
    """

    checks = 0
    queues = cups_queues()
    # Use the serial the queue actually asks for, when there is a queue on this machine.
    wanted = queues[0]['serial'] if queues else '000000000Q80E182PR1a'

    devices = usb_devices(synthetic_registry(serial=wanted))
    printers = [d for d in devices if is_printer(d)]
    assert len(printers) == 1, 'expected one printer, got %r' % printers
    assert printers[0]['serial'] == wanted, printers[0]
    checks += 2

    report = capture(human, devices, queues, False)
    wanted_lines = ['HP LaserJet Professional P1102', 'interface 7/1/2', 'endpoint(s)']
    if queues:
        wanted_lines.append('matches')
    for needle in wanted_lines:
        assert needle in report, 'the report is missing %r:\n%s' % (needle, report)
        checks += 1

    # Queues for other manufacturers must not be reported as serial mismatches: their serials
    # always differ from this device's, and saying so would be noise, not a finding.
    if any(queue_names_device(q, printers[0]) for q in queues):
        assert 'DIFFERENT' not in report, 'a queue that names another device was flagged:\n%s' % report
        checks += 1

    # A printer with the wrong serial must be called out, not quietly accepted.
    other = usb_devices(synthetic_registry(serial='NOT-THE-SERIAL-IN-THE-URI'))
    report = capture(human, other, queues, False)
    if queues:
        assert 'DIFFERENT' in report, report
        checks += 1

    # And a device that is not a printer must not be mistaken for one.
    keyboard = usb_devices(synthetic_registry(product='Keyboard', vendor_id=0x1234,
                                              printer_interface=False))
    assert not [d for d in keyboard if is_printer(d)], keyboard
    report = capture(human, keyboard, queues, False)
    assert 'no USB Printer-class device' in report, report
    checks += 2

    print('preflight self-test: %d checks passed' % checks)
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--json', action='store_true',
                    help='dump everything collected as JSON, for diffing two runs')
    ap.add_argument('--all', action='store_true',
                    help='list every USB device, not just printers and hubs')
    ap.add_argument('--from', dest='from_file', metavar='PLIST',
                    help='read a saved `ioreg -a` registry dump, or a report this script wrote '
                         'with --json, instead of the live registry')
    ap.add_argument('--selftest', action='store_true',
                    help='check the report against a hand-built registry, then stop')
    args = ap.parse_args(argv)

    if args.selftest:
        return selftest()

    source = None
    if args.from_file:
        try:
            source = open(args.from_file, 'rb').read()
        except OSError as exc:
            raise SystemExit('cannot read %s: %s' % (args.from_file, exc))
    reported = report_from_dump(source) if source is not None else None
    if reported is not None:
        devices, queues = reported['devices'], reported.get('queues', [])
    else:
        devices = usb_devices(source)
        queues = cups_queues()
    printers = [d for d in devices if is_printer(d)]

    if args.json:
        print(json.dumps({'printers_attached': len(printers), 'devices': devices,
                          'queues': queues}, indent=2, sort_keys=True))
    else:
        human(devices, queues, args.all)
    return 0 if printers else 1


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
