#!/usr/bin/env python3
"""Read-only ELF identifier, used for the HP *.acl device firmware images.

The HP ZJS bundle ships `HP1005.acl`, `HP1006.acl`, `HP1505.acl`, `hp1020.acl`.
They are not Mach-O and not plain blobs: each is an ELF image with a short
ASCII date prefix. This script reports the ELF identity plus the surrounding
container framing so the upload protocol can be understood without guessing.

Usage: python3 scripts/elfinfo.py <file> [file...]
"""

import struct
import sys

EM = {
    0: "EM_NONE", 3: "EM_386", 8: "EM_MIPS", 20: "EM_PPC", 21: "EM_PPC64",
    40: "EM_ARM", 42: "EM_SH", 62: "EM_X86_64", 83: "EM_AVR", 94: "EM_TRICORE",
    183: "EM_AARCH64", 243: "EM_RISCV",
}

ET = {1: "ET_REL", 2: "ET_EXEC", 3: "ET_DYN", 4: "ET_CORE"}


def hexdump(data, offset, length, width=16):
    out = []
    for i in range(0, length, width):
        chunk = data[offset + i:offset + i + width]
        if not chunk:
            break
        hx = " ".join("%02x" % b for b in chunk)
        pr = "".join(chr(b) if 32 <= b < 127 else "." for b in chunk)
        out.append("%08x  %-*s  %s" % (offset + i, width * 3 - 1, hx, pr))
    return "\n".join(out)


def analyse(path):
    with open(path, "rb") as f:
        d = f.read()

    print("== %s  (%d bytes)" % (path, len(d)))
    print("   head: %s" % hexdump(d, 0, 32).replace("\n", "\n         "))

    off = d.find(b"\x7fELF")
    if off < 0:
        print("   not an ELF container")
        return
    print("   ELF magic at offset %d (prefix=%r)" % (off, d[:off]))

    ei_class = d[off + 4]
    ei_data = d[off + 5]
    endian = "<" if ei_data == 1 else ">"
    if ei_class != 1:
        print("   only 32-bit ELF headers handled here (class=%d)" % ei_class)
        return

    (e_type, e_machine, e_version, e_entry, e_phoff, e_shoff, e_flags) = \
        struct.unpack_from(endian + "HHIIIII", d, off + 16)
    (e_ehsize, e_phentsize, e_phnum, e_shentsize, e_shnum, e_shstrndx) = \
        struct.unpack_from(endian + "HHHHHH", d, off + 40)

    print("   32-bit, %s-endian, %s, %s (%d)" % (
        "little" if ei_data == 1 else "big", ET.get(e_type, e_type),
        EM.get(e_machine, "unknown"), e_machine))
    print("   entry=0x%08x  phoff=%d phnum=%d  shoff=%d shnum=%d  flags=0x%x" % (
        e_entry, e_phoff, e_phnum, e_shoff, e_shnum, e_flags))

    print("   program headers:")
    for i in range(e_phnum):
        base = off + e_phoff + i * e_phentsize
        p_type, p_offset, p_vaddr, p_paddr, p_filesz, p_memsz, p_flags, p_align = \
            struct.unpack_from(endian + "IIIIIIII", d, base)
        kinds = {0: "NULL", 1: "LOAD", 2: "DYNAMIC", 3: "INTERP", 4: "NOTE"}
        print("     [%d] %-8s file=0x%06x vaddr=0x%08x filesz=%-8d memsz=%-8d flags=%d align=%d" % (
            i, kinds.get(p_type, str(p_type)), p_offset, p_vaddr, p_filesz, p_memsz, p_flags, p_align))

    if len(d) > off + 64:
        print("   bytes after header: %s" % hexdump(d, off + 52, 32).replace("\n", "\n      "))


def main(argv):
    if len(argv) < 2:
        print(__doc__)
        return 2
    for p in argv[1:]:
        analyse(p)
        print()
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
