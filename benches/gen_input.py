"""Generate a large, syntactically valid MCNP file for benchmarking.

The performance budget (docs/04) is defined on a ~380 MB / ~1M-card model. The
real one (`big.mcnp`) is private and untracked, so this script produces a
reproducible stand-in with the same shape: many cells (each with geometry refs
and parameters), many surfaces, and a spread of data cards. It is deliberately
repetitive — that is representative of machine-generated reactor/tokamak models,
which is where the size comes from.

Usage:
    python gen_input.py --cells 1000000 > big.mcnp
    python gen_input.py --cells 50000   > medium.mcnp     # quick local runs

Rough sizing: ~1,000,000 cells produces a file in the low-hundreds of MB.
"""

from __future__ import annotations

import argparse
import sys


def generate(n_cells: int, out) -> None:
    n_surfaces = max(3, n_cells // 4)

    out.write("Generated benchmark model\n")

    # --- cell block ---------------------------------------------------------
    # Each cell references a few surfaces and carries a couple of parameters,
    # so renumber passes have real work (defs + references + bins).
    for i in range(1, n_cells + 1):
        s = (i % n_surfaces) + 1
        mat = (i % 5)  # 0 => void, exercises the void/density branch
        if mat == 0:
            out.write(f"{i} 0 -{s} {s + 1} imp:n=1 u={i % 100}\n")
        else:
            rho = -1.0 - (i % 7) * 0.1
            out.write(
                f"{i} {mat} {rho:.3f} -{s} {s + 1} #{(i % 10) + 1} "
                f"imp:n=1 imp:p=0 vol={i % 1000}.0\n"
            )
    out.write("\n")

    # --- surface block ------------------------------------------------------
    kinds = ["SO", "PX", "PY", "PZ", "CX", "CY", "CZ"]
    for s in range(1, n_surfaces + 2):
        kind = kinds[s % len(kinds)]
        out.write(f"{s} {kind} {(s % 50) + 1}.0\n")
    out.write("\n")

    # --- data block ---------------------------------------------------------
    for m in range(1, 5):
        out.write(f"m{m} 1001.31c {m} 8016.31c 1\n")
    out.write("tr1 0 0 0\n")
    out.write("*tr2 0 0 0 30 60 90 60 30 90 90 90 0\n")
    out.write("f4:n 1 2 3\n")
    out.write("fc4 flux tally\n")
    out.write("mode n p\n")
    out.write("nps 1e6\n")
    out.write("sdef pos=0 0 0\n")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--cells", type=int, default=1_000_000,
                    help="number of cell cards (default 1,000,000)")
    args = ap.parse_args()
    generate(args.cells, sys.stdout)


if __name__ == "__main__":
    main()
