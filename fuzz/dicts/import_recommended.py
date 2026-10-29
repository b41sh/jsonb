#!/usr/bin/env python3
"""Appends libFuzzer's "Recommended dictionary" output to a `-dict` file.

libFuzzer prints non-ASCII bytes in that output as octal escapes such as
`\\377`, but `-dict` files only accept `\\xNN`, `\\\\` and `\\"`. This script
re-encodes each entry, drops the `# Uses: N` comments (a dictionary line must
end with the closing quote), and skips entries the file already contains.

Usage, from the `fuzz` directory:

    python3 dicts/import_recommended.py dicts/raw_jsonb.dict < fuzz.log
"""

import re
import sys

PRINTED_ENTRY = re.compile(r'^\s*"((?:[^"\\]|\\.)*)"\s*(?:#.*)?$')
# A `-dict` line is `"value"` or `name="value"`.
DICT_ENTRY = re.compile(r'^\s*(?:\w+=)?"((?:[^"\\]|\\.)*)"\s*$')
START = "###### Recommended dictionary. ######"
END = "###### End of recommended dictionary. ######"


def decode_printed(text):
    """Decodes an entry as printed by libFuzzer (`\\\\`, `\\"`, `\\ooo`)."""
    out = bytearray()
    i = 0
    while i < len(text):
        c = text[i]
        if c != "\\":
            out += c.encode("latin-1")
            i += 1
        elif text[i + 1] in '\\"':
            out += text[i + 1].encode("latin-1")
            i += 2
        else:
            out.append(int(text[i + 1 : i + 4], 8))
            i += 4
    return bytes(out)


def decode_dict(text):
    """Decodes an entry in `-dict` file syntax (`\\\\`, `\\"`, `\\xNN`)."""
    out = bytearray()
    i = 0
    while i < len(text):
        c = text[i]
        if c != "\\":
            out += c.encode("latin-1")
            i += 1
        elif text[i + 1] == "x":
            out.append(int(text[i + 2 : i + 4], 16))
            i += 4
        else:
            out += text[i + 1].encode("latin-1")
            i += 2
    return bytes(out)


def encode_dict(data):
    """Encodes bytes in `-dict` file syntax."""
    parts = []
    for b in data:
        if b in (0x22, 0x5C):
            parts.append("\\" + chr(b))
        elif 0x20 <= b < 0x7F:
            parts.append(chr(b))
        else:
            parts.append(f"\\x{b:02X}")
    return '"' + "".join(parts) + '"'


def read_existing(path):
    try:
        with open(path, encoding="latin-1") as f:
            lines = f.read().splitlines()
    except FileNotFoundError:
        return set()
    entries = set()
    for line in lines:
        match = DICT_ENTRY.match(line)
        if match:
            entries.add(decode_dict(match.group(1)))
    return entries


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    path = sys.argv[1]
    lines = sys.stdin.read().splitlines()
    # Use only the recommended-dictionary block when the whole log is piped in.
    if START in lines:
        lines = lines[lines.index(START) + 1 :]
        lines = lines[: lines.index(END)] if END in lines else lines

    existing = read_existing(path)
    new = []
    for line in lines:
        match = PRINTED_ENTRY.match(line)
        if not match:
            continue
        data = decode_printed(match.group(1))
        if data and data not in existing:
            existing.add(data)
            new.append(encode_dict(data))

    if new:
        with open(path, "a", encoding="latin-1") as f:
            f.write("\n".join(new) + "\n")
    print(f"added {len(new)} entries to {path}")


if __name__ == "__main__":
    main()
