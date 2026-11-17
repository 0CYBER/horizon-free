#!/usr/bin/env python3
"""Decode Java's modified UTF-8, the string encoding used by NBT and writeUTF.

Java does not write ordinary UTF-8. ``DataOutputStream.writeUTF`` encodes each
UTF-16 code unit of a Java String on its own:

  * U+0001..U+007F  -> one byte ``0xxxxxxx`` (a raw 0x00 byte also means U+0000)
  * U+0000, U+0080..U+07FF -> ``110xxxxx 10xxxxxx`` (so NUL is written C0 80)
  * U+0800..U+FFFF  -> ``1110xxxx 10xxxxxx 10xxxxxx`` (surrogates included)

A supplementary character is therefore two 3-byte forms: the high and low
surrogate code units. Decoding must recombine them into one Python code point,
while a surrogate unit that stands alone stays a lone surrogate in the result,
just as it does in a Java String.

Overlong 2-/3-byte forms and a raw 0x00 byte are accepted by
``DataInputStream.readUTF`` even though ``writeUTF`` uses canonical forms. Four-byte UTF-8,
bare continuation bytes and truncated or illegal sequences are not valid
modified UTF-8 and raise ValueError instead of being replaced.

Usage:
  modified_utf8.py HEX       decode the hex bytes as modified UTF-8
  modified_utf8.py --stream HEX
                             first two hex bytes are the unsigned length
                             (the form DataInputStream.readUTF reads)
  modified_utf8.py           read the hex from standard input
"""
import json
import sys

USAGE = __doc__


def decode_mutf8(data):
    """Decode a complete modified-UTF-8 payload (no length prefix) to a str."""
    if isinstance(data, str):
        raise TypeError('decode_mutf8 expects bytes, not str')
    units = []
    i = 0
    n = len(data)
    while i < n:
        lead = data[i]
        top = lead >> 4
        if top < 8:  # 0xxxxxxx (a raw 0x00 byte is Java's raw U+0000)
            units.append(lead)
            i += 1
        elif top in (12, 13):  # 110xxxxx 10xxxxxx
            if i + 1 >= n:
                raise ValueError('truncated modified UTF-8: 2-byte form at byte %d' % i)
            b2 = data[i + 1]
            if b2 & 0xC0 != 0x80:
                raise ValueError('illegal modified UTF-8 continuation at byte %d' % (i + 1))
            units.append(((lead & 0x1F) << 6) | (b2 & 0x3F))
            i += 2
        elif top == 14:  # 1110xxxx 10xxxxxx 10xxxxxx
            if i + 2 >= n:
                raise ValueError('truncated modified UTF-8: 3-byte form at byte %d' % i)
            b2, b3 = data[i + 1], data[i + 2]
            if b2 & 0xC0 != 0x80 or b3 & 0xC0 != 0x80:
                raise ValueError('illegal modified UTF-8 continuation at byte %d' % (i + 1))
            units.append(((lead & 0x0F) << 12) | ((b2 & 0x3F) << 6) | (b3 & 0x3F))
            i += 3
        else:  # 10xxxxxx (bare continuation) or 1111xxxx (4-byte UTF-8 lead)
            raise ValueError('invalid modified UTF-8 lead byte 0x%02x at byte %d' % (lead, i))

    out = []
    j = 0
    m = len(units)
    while j < m:
        unit = units[j]
        if (0xD800 <= unit <= 0xDBFF and j + 1 < m
                and 0xDC00 <= units[j + 1] <= 0xDFFF):
            low = units[j + 1]
            out.append(chr(0x10000 + ((unit - 0xD800) << 10) + (low - 0xDC00)))
            j += 2
        else:
            out.append(chr(unit))
            j += 1
    return ''.join(out)


def read_utf(data, offset=0):
    """Read a DataInputStream.readUTF() string from `data` at `offset`.

    Consumes the two-byte unsigned length and that many modified-UTF-8 bytes.
    Returns (text, offset_after_string).
    """
    if offset + 2 > len(data):
        raise ValueError('truncated modified UTF-8 length')
    length = (data[offset] << 8) | data[offset + 1]
    start = offset + 2
    end = start + length
    if end > len(data):
        raise ValueError('truncated modified UTF-8 string: want %d bytes' % length)
    return decode_mutf8(data[start:end]), end


def main(argv):
    if '--help' in argv or '-h' in argv:
        print(USAGE)
        return 0
    length_prefixed = '--stream' in argv
    argv = [a for a in argv if a != '--stream']
    text = ''.join(argv) if argv else sys.stdin.read().strip()
    try:
        raw = bytes.fromhex(text)
    except ValueError as exc:
        print('bad hex: %s' % exc, file=sys.stderr)
        return 2
    try:
        value = read_utf(raw)[0] if length_prefixed else decode_mutf8(raw)
    except ValueError as exc:
        print('invalid modified UTF-8: %s' % exc, file=sys.stderr)
        return 1
    print(json.dumps(value))
    return 0


if __name__ == '__main__':
    sys.exit(main(sys.argv[1:]))
