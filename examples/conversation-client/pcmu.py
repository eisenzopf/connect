"""G.711 mu-law conversion for the native microphone example (little-endian PCM)."""
import struct

_BIAS = 0x84
_SEGMENTS = (0xFF, 0x1FF, 0x3FF, 0x7FF, 0xFFF, 0x1FFF, 0x3FFF, 0x7FFF)


def encode(pcm):
    """Convert signed 16-bit PCM into 8-bit PCMU without external codec tools."""
    result = bytearray()
    for (sample,) in struct.iter_unpack('<h', pcm):
        mask = 0x7F if sample < 0 else 0xFF
        magnitude = min(abs(sample), 32635) + _BIAS
        segment = next(i for i, end in enumerate(_SEGMENTS) if magnitude <= end)
        value = (segment << 4) | ((magnitude >> (segment + 3)) & 0x0F)
        result.append(value ^ mask)
    return bytes(result)


def decode(pcmu):
    """Convert PCMU into signed 16-bit PCM."""
    result = bytearray()
    for value in pcmu:
        value = (~value) & 0xFF
        magnitude = (((value & 0x0F) << 3) + _BIAS) << ((value & 0x70) >> 4)
        sample = _BIAS - magnitude if value & 0x80 else magnitude - _BIAS
        result.extend(struct.pack('<h', sample))
    return bytes(result)
