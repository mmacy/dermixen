#!/usr/bin/env python3
"""Writes a valid stereo FLAC file of digital silence of any length.

Usage: python3 tools/make_long_flac.py <minutes> <output.flac> [--no-total] [--rate HZ]

Each FLAC frame is a constant subframe of 17 to 19 bytes that decodes to
65,535 stereo frames, so 91 minutes of audio is about 65 KB. The decoder tests
use such a file to show that a track over the length limit is refused without
being decoded. With --no-total the header states no total length, so a decoder
learns the length only by decoding.

--rate writes the file at another sample rate, which is what shows how much
memory a decode of a high-rate file takes. FLAC's frame header has a code for
44,100 Hz and none for most other rates, so at any other rate the frame header
carries code 0, which tells a decoder to read the rate from the stream header
instead. Standard library only.
"""
import struct
import sys

minutes = float(sys.argv[1])
options = sys.argv[3:]
NO_TOTAL = "--no-total" in options
RATE = int(options[options.index("--rate") + 1]) if "--rate" in options else 44100
BLOCK = 65535
# The four bits of the frame header that name the sample rate. Code 9 is
# 44,100 Hz, and code 0 sends the decoder to the stream header for the rate.
RATE_CODE = 9 if RATE == 44100 else 0
frames = int(minutes * 60 * RATE) // BLOCK
total = frames * BLOCK


def crc(data, poly, bits):
    top = 1 << (bits - 1)
    mask = (1 << bits) - 1
    value = 0
    for byte in data:
        value ^= byte << (bits - 8)
        for _ in range(8):
            value = ((value << 1) ^ poly) & mask if value & top else (value << 1) & mask
    return value


def utf8(n):
    if n < 0x80:
        return bytes([n])
    if n < 0x800:
        return bytes([0xC0 | n >> 6, 0x80 | n & 0x3F])
    if n < 0x10000:
        return bytes([0xE0 | n >> 12, 0x80 | (n >> 6) & 0x3F, 0x80 | n & 0x3F])
    return bytes([0xF0 | n >> 18, 0x80 | (n >> 12) & 0x3F, 0x80 | (n >> 6) & 0x3F, 0x80 | n & 0x3F])


info = struct.pack(">HH", BLOCK, BLOCK) + b"\0\0\0" + b"\0\0\0"
packed = (RATE << 44) | (1 << 41) | (15 << 36) | (0 if NO_TOTAL else total)
info += packed.to_bytes(8, "big") + b"\0" * 16
out = bytearray(b"fLaC" + b"\x80" + len(info).to_bytes(3, "big") + info)
for n in range(frames):
    header = b"\xff\xf8" + bytes([0x70 | RATE_CODE]) + b"\x18" + utf8(n) + struct.pack(">H", BLOCK - 1)
    header += bytes([crc(header, 0x07, 8)])
    body = header + b"\x00\x00\x00" + b"\x00\x00\x00"
    out += body + struct.pack(">H", crc(body, 0x8005, 16))
open(sys.argv[2], "wb").write(out)
print(f"{sys.argv[2]}: {len(out)} bytes for {total} stereo frames ({total / RATE / 60:.1f} minutes)")
