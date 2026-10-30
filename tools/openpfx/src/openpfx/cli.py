from __future__ import annotations

import argparse
import sys
from pathlib import Path

from .pse import read_pse


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(prog="openpfx")
    sub = p.add_subparsers(dest="cmd", required=True)
    d = sub.add_parser("dump-pse", help="Dump PSE header / paths")
    d.add_argument("path", type=Path)
    args = p.parse_args(argv)
    if args.cmd == "dump-pse":
        h = read_pse(args.path)
        print(h)
        return 0
    return 1


if __name__ == "__main__":
    sys.exit(main())
