"""Best-effort PSE (ParticleFX editor project) header dump."""

from __future__ import annotations

import struct
from dataclasses import dataclass
from pathlib import Path


@dataclass
class PseHeader:
    word0: int
    word1: int
    word2: int
    float_a: float
    float_b: float
    ascii_paths: list[str]


def read_pse(path: str | Path) -> PseHeader:
    data = Path(path).read_bytes()
    w0, w1, w2 = struct.unpack_from("<III", data, 0)
    fa, fb = struct.unpack_from("<ff", data, 12)
    paths: list[str] = []
    # crude ASCII path harvest
    i = 0
    while i < len(data):
        if data[i : i + 3] in (b"C:\\", b"c:\\"):
            j = i
            while j < len(data) and data[j] != 0 and data[j] >= 32:
                j += 1
            paths.append(data[i:j].decode("latin-1", "replace"))
            i = j
        else:
            i += 1
    return PseHeader(w0, w1, w2, fa, fb, paths)
