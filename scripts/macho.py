#!/usr/bin/env python3
"""Minimal read-only Mach-O inspector.

Why this exists: Apple's otool/nm/lipo/strings are gated behind the Xcode
license prompt on this machine, and /usr/bin/python3 is gated too. This script
runs under Homebrew python and needs no Apple developer tools, so the driver
binaries stay inspectable without modifying anything.

Usage:
    python3 scripts/macho.py info   <file> [file...]
    python3 scripts/macho.py libs   <file>
    python3 scripts/macho.py syms   <file> [pattern]
    python3 scripts/macho.py segs   <file>
    python3 scripts/macho.py strings <file> [minlen]
"""

import re
import struct
import sys

MH_MAGIC = 0xFEEDFACE
MH_CIGAM = 0xCEFAEDFE
MH_MAGIC_64 = 0xFEEDFACF
MH_CIGAM_64 = 0xCFFAEDFE
FAT_MAGIC = 0xCAFEBABE
FAT_CIGAM = 0xBEBAFECA
FAT_MAGIC_64 = 0xCAFEBABF
FAT_CIGAM_64 = 0xBFBAFECA

CPU_TYPES = {
    7: "x86",
    0x01000007: "x86_64",
    12: "arm",
    0x0100000C: "arm64",
    0x0200000C: "arm64_32",
    18: "ppc",
}

# cputype arm64 with subtype 2 is arm64e (pointer-auth ABI).
CPU_SUBTYPE_ARM64E = 2

FILE_TYPES = {
    1: "object",
    2: "executable",
    3: "fixedvm-lib",
    4: "core",
    5: "preload",
    6: "dylib",
    7: "dylinker",
    8: "bundle",
    9: "dylib-stub",
    10: "dylib-bundle",
    11: "executable-bundle",
    12: "dSYM",
}

PLATFORMS = {1: "macOS", 2: "iOS", 3: "tvOS", 4: "watchOS", 5: "bridgeOS",
             6: "macCatalyst", 7: "iOSSimulator", 9: "driverKit", 10: "visionOS"}

LC_REQ_DYLD = 0x80000000

LC_NAMES = {
    0x1: "LC_SEGMENT",
    0x2: "LC_SYMTAB",
    0xB: "LC_DYSYMTAB",
    0xC: "LC_LOAD_DYLIB",
    0xD: "LC_ID_DYLIB",
    0xE: "LC_LOAD_DYLINKER",
    0x18 | LC_REQ_DYLD: "LC_LOAD_WEAK_DYLIB",
    0x19: "LC_SEGMENT_64",
    0x1C: "LC_UUID",
    0x1C | LC_REQ_DYLD: "LC_RPATH",
    0x1D: "LC_CODE_SIGNATURE",
    0x1F | LC_REQ_DYLD: "LC_REEXPORT_DYLIB",
    0x20: "LC_LAZY_LOAD_DYLIB",
    0x21: "LC_ENCRYPTION_INFO",
    0x22: "LC_DYLD_INFO",
    0x23 | LC_REQ_DYLD: "LC_LOAD_UPWARD_DYLIB",
    0x24: "LC_VERSION_MIN_MACOSX",
    0x25: "LC_VERSION_MIN_IPHONEOS",
    0x26: "LC_FUNCTION_STARTS",
    0x28 | LC_REQ_DYLD: "LC_MAIN",
    0x29: "LC_DATA_IN_CODE",
    0x2A: "LC_SOURCE_VERSION",
    0x2B: "LC_DYLIB_CODE_SIGN_DRS",
    0x2C: "LC_ENCRYPTION_INFO_64",
    0x2F: "LC_LINKER_OPTION",
    0x32: "LC_BUILD_VERSION",
    0x33: "LC_DYLD_EXPORTS_TRIE",
    0x34: "LC_DYLD_CHAINED_FIXUPS",
}

DYLIB_CMDS = {
    0xC: "load",
    0xD: "id",
    0x18 | LC_REQ_DYLD: "weak",
    0x1F | LC_REQ_DYLD: "reexport",
    0x20: "lazy",
    0x23 | LC_REQ_DYLD: "upward",
}


class Reader:
    def __init__(self, data):
        self.d = data

    def __len__(self):
        return len(self.d)

    def cstr(self, off, limit=4096):
        end = self.d.find(b"\x00", off, off + limit)
        if end < 0:
            end = min(len(self.d), off + limit)
        return self.d[off:end].decode("utf-8", "replace")


def slices(data):
    """Return [(arch, offset, size)] for fat or thin Mach-O data."""
    if len(data) < 8:
        return []
    magic = struct.unpack(">I", data[:4])[0]
    if magic in (FAT_MAGIC, FAT_MAGIC_64):
        wide = magic == FAT_MAGIC_64
        nfat = struct.unpack(">I", data[4:8])[0]
        out = []
        ent = 32 if wide else 20
        for i in range(nfat):
            base = 8 + i * ent
            cputype, cpusub = struct.unpack(">ii", data[base:base + 8])
            if wide:
                off, size = struct.unpack(">QQ", data[base + 8:base + 24])
            else:
                off, size = struct.unpack(">II", data[base + 8:base + 16])
            out.append((arch_name(cputype, cpusub), off, size))
        return out
    return [("thin", 0, len(data))]


def arch_name(cputype, cpusub):
    name = CPU_TYPES.get(cputype, "cpu0x%x" % cputype)
    if cputype == 0x0100000C and cpusub == CPU_SUBTYPE_ARM64E:
        name = "arm64e"
    return name


def parse_mach(data, base, is_fat):
    magic = struct.unpack("<I", data[base:base + 4])[0]
    if magic == MH_MAGIC_64:
        endian, is64 = "<", True
    elif magic == MH_CIGAM_64:
        endian, is64 = ">", True
    elif magic == MH_MAGIC:
        endian, is64 = "<", False
    elif magic == MH_CIGAM:
        endian, is64 = ">", False
    else:
        return None

    hdr = 32 if is64 else 28
    cputype, cpusub, ftype, ncmds, sizeofcmds, flags = struct.unpack(
        endian + "iiIIII", data[base + 4:base + 4 + 24])
    info = {
        "arch": arch_name(cputype, cpusub),
        "cputype": cputype,
        "cpusubtype": cpusub,
        "filetype": FILE_TYPES.get(ftype, str(ftype)),
        "flags": "0x%08x" % flags,
        "ncmds": ncmds,
        "dylibs": [],
        "rpaths": [],
        "segments": [],
        "platform": None,
        "minos": None,
        "sdk": None,
        "symtab": None,
        "uuid": None,
        "entry": None,
    }

    def vers(v):
        return "%d.%d.%d" % ((v >> 16) & 0xFFFF, (v >> 8) & 0xFF, v & 0xFF)

    off = base + hdr
    for _ in range(ncmds):
        cmd, cmdsize = struct.unpack(endian + "II", data[off:off + 8])
        body = data[off:off + cmdsize]
        if cmd in DYLIB_CMDS:
            nameoff = struct.unpack(endian + "I", body[8:12])[0]
            ts, cur, comp = struct.unpack(endian + "III", body[12:24])
            info["dylibs"].append({
                "kind": DYLIB_CMDS[cmd],
                "path": Reader(data).cstr(off + nameoff, 1024),
                "current": vers(cur),
                "compat": vers(comp),
            })
        elif cmd == (0x1C | LC_REQ_DYLD):  # LC_RPATH
            nameoff = struct.unpack(endian + "I", body[8:12])[0]
            info["rpaths"].append(Reader(data).cstr(off + nameoff, 1024))
        elif cmd == 0x19:  # LC_SEGMENT_64
            segname = body[8:24].rstrip(b"\x00").decode("utf-8", "replace")
            vmaddr, vmsize, fileoff, filesize = struct.unpack(endian + "QQQQ", body[24:56])
            initprot = struct.unpack(endian + "i", body[60:64])[0]
            info["segments"].append({
                "name": segname, "vmaddr": vmaddr, "vmsize": vmsize,
                "fileoff": fileoff, "filesize": filesize, "prot": initprot,
            })
        elif cmd in (0x24, 0x25):  # LC_VERSION_MIN_MACOSX / IPHONEOS
            v, sdk = struct.unpack(endian + "II", body[8:16])
            info["platform"] = "macOS" if cmd == 0x24 else "iOS"
            info["minos"], info["sdk"] = vers(v), vers(sdk)
        elif cmd == 0x32:  # LC_BUILD_VERSION
            plat, minos, sdk, ntools = struct.unpack(endian + "IIII", body[8:24])
            info["platform"] = PLATFORMS.get(plat, "platform%d" % plat)
            info["minos"], info["sdk"] = vers(minos), vers(sdk)
            info["ntools"] = ntools
        elif cmd == 0x2:  # LC_SYMTAB
            symoff, nsyms, stroff, strsize = struct.unpack(endian + "IIII", body[8:24])
            info["symtab"] = {"symoff": symoff, "nsyms": nsyms,
                              "stroff": stroff, "strsize": strsize}
        elif cmd == 0x1C:  # LC_UUID
            info["uuid"] = body[8:24].hex()
        elif cmd == (0x28 | LC_REQ_DYLD):  # LC_MAIN
            if len(body) >= 24:
                eo, ss = struct.unpack(endian + "QQ", body[8:24])
                info["entry"] = eo
        off += cmdsize

    info["_endian"] = endian
    info["_is64"] = is64
    return info


def nlist_symbols(data, mach, base):
    st = mach["symtab"]
    if not st or not st["nsyms"]:
        return []
    endian = mach["_endian"]
    entsize = 16 if mach["_is64"] else 12
    out = []
    # offsets are relative to the start of the Mach-O slice containing them
    for i in range(st["nsyms"]):
        o = base + st["symoff"] + i * entsize
        if mach["_is64"]:
            n_strx, n_type, n_sect, n_desc, n_value = struct.unpack(endian + "IBBHQ", data[o:o + 16])
        else:
            n_strx, n_type, n_sect, n_desc, n_value = struct.unpack(endian + "IBBHI", data[o:o + 12])
        name = Reader(data).cstr(base + st["stroff"] + n_strx, 2048)
        kind = "export" if (n_type & 0x01) else ("undef" if (n_type & 0x0E) == 0 else "other")
        out.append({"name": name, "value": n_value, "sect": n_sect, "kind": kind})
    return out


def cmd_info(path):
    with open(path, "rb") as f:
        data = f.read()
    print("== %s (%d bytes)" % (path, len(data)))
    for arch, off, size in slices(data):
        m = parse_mach(data, off, arch != "thin")
        if not m:
            print("   [%s] not a Mach-O slice" % arch)
            continue
        print("   arch=%-8s type=%-12s minos=%-8s sdk=%-8s plat=%s" % (
            m["arch"], m["filetype"], m["minos"], m["sdk"], m["platform"]))
        print("        slice=%d..%d  uuid=%s  entry=%s" % (off, off + size, m["uuid"], m["entry"]))


def cmd_libs(path):
    with open(path, "rb") as f:
        data = f.read()
    for arch, off, size in slices(data):
        m = parse_mach(data, off, arch != "thin")
        if not m:
            continue
        print("== %s [%s]" % (path, m["arch"]))
        for d in m["dylibs"]:
            print("   %-9s cur=%-10s compat=%-10s %s" % (d["kind"], d["current"], d["compat"], d["path"]))
        for r in m["rpaths"]:
            print("   rpath     %s" % r)


def cmd_syms(path, pattern=None):
    rx = re.compile(pattern) if pattern else None
    with open(path, "rb") as f:
        data = f.read()
    for arch, off, size in slices(data):
        m = parse_mach(data, off, arch != "thin")
        if not m:
            continue
        print("== %s [%s]" % (path, m["arch"]))
        for s in nlist_symbols(data, m, off):
            if not s["name"]:
                continue
            if rx and not rx.search(s["name"]):
                continue
            print("   %-8s %-16x %s" % (s["kind"], s["value"], s["name"]))


def cmd_segs(path):
    with open(path, "rb") as f:
        data = f.read()
    for arch, off, size in slices(data):
        m = parse_mach(data, off, arch != "thin")
        if not m:
            continue
        print("== %s [%s]" % (path, m["arch"]))
        for s in m["segments"]:
            print("   %-12s vm=%016x size=%08x file=%08x prot=%d" % (
                s["name"], s["vmaddr"], s["vmsize"], s["fileoff"], s["prot"]))


PRINTABLE = re.compile(rb"[ -~\t]{4,}")


def cmd_strings(path, minlen=6):
    with open(path, "rb") as f:
        data = f.read()
    print("== %s" % path)
    for m in PRINTABLE.finditer(data):
        s = m.group()
        if len(s) >= minlen:
            print("%08x  %s" % (m.start(), s.decode("ascii", "replace")))


def main(argv):
    if len(argv) < 3:
        print(__doc__)
        return 2
    mode, args = argv[1], argv[2:]
    if mode == "info":
        for p in args:
            cmd_info(p)
    elif mode == "libs":
        cmd_libs(args[0])
    elif mode == "syms":
        cmd_syms(args[0], args[1] if len(args) > 1 else None)
    elif mode == "segs":
        cmd_segs(args[0])
    elif mode == "strings":
        cmd_strings(args[0], int(args[1]) if len(args) > 1 else 6)
    else:
        print(__doc__)
        return 2
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
