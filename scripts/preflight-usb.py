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
import importlib.util
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


def usb_devices():
    """Every USB device, each with the interfaces that belong to it.

    Interfaces are siblings of their device in the registry, not children, so they are
    matched back by locationID, which both carry. Endpoints are not registry entries at
    all: only their count (bNumEndpoints) is exposed, which is why the report shows that
    rather than inventing addresses.
    """
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
            if q['serial'] and d['serial']:
                same = q['serial'] == d['serial']
                print('  queue %s asks for serial %s: %s' % (
                    q['queue'], q['serial'], 'matches' if same else 'DIFFERENT'))
                if not same:
                    print('    the queue points at a device that is not the attached one;'
                          ' the usb backend will not find it')
            elif q['serial'] and not d['serial']:
                print('  queue %s asks for a serial, the device reports none' % q['queue'])
    print()
    print('  if the device re-enumerated, its entry id above differs from the previous')
    print('  report; diff two --json runs to see exactly what changed.')


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument('--json', action='store_true',
                    help='dump everything collected as JSON, for diffing two runs')
    ap.add_argument('--all', action='store_true',
                    help='list every USB device, not just printers and hubs')
    args = ap.parse_args(argv)

    devices = usb_devices()
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
