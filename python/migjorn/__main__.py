import sys

from migjorn import run


def main():
    raise SystemExit(run(["migjorn", *sys.argv[1:]]))


if __name__ == "__main__":
    main()
