"""Original, deterministic small V-stitch colour/normal textures, embedded as PNG."""
import struct
import zlib

import numpy as np


def png_bytes(rgb):
    pixels = np.uint8(np.clip(rgb, 0, 1) * 255)
    height, width, _ = pixels.shape
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack('>I', zlib.crc32(kind + data))
    rows = b''.join(b'\0' + row.tobytes() for row in pixels)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>2I5B', width, height, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(rows)) + chunk(b'IEND', b''))


def knit_textures():
    y, x = np.mgrid[0:256, 0:256] / 32
    u, v = x % 1, y % 1
    distance = np.minimum(abs(u - (.5 - .29 * (1 - v))), abs(u - (.5 + .29 * (1 - v))))
    height = np.exp(-((distance / .075) ** 2)) * (.8 + .2 * np.sin(v * np.pi))
    shade = .90 + .085 * height + .01 * np.cos(2 * np.pi * (x * 4 + y * 3))
    colour = np.repeat(shade[:, :, None], 3, axis=2)
    dx = (np.roll(height, -1, axis=1) - np.roll(height, 1, axis=1)) * .5
    dy = (np.roll(height, -1, axis=0) - np.roll(height, 1, axis=0)) * .5
    normal = np.stack([-dx, dy, np.ones_like(height)], axis=-1)
    normal /= np.linalg.norm(normal, axis=-1, keepdims=True)
    return {'base': png_bytes(colour), 'normal': png_bytes(normal * .5 + .5)}
