"""Stub-drift guard: the shipped ``migjorn.pyi`` must match the runtime module.

Parses the type stub and compares, in both directions, the public members of
every class against the actual PyO3 extension. If someone adds/removes/renames a
method or property in ``crates/migjorn-py/src/lib.rs`` without updating the stub
(or vice versa), this fails.

Run: ``pytest tests/test_stub.py -q``.
"""

from __future__ import annotations

import ast
from pathlib import Path

import migjorn

# Classes whose members are compared. MergeError is excluded — it only subclasses
# ValueError and declares no members of its own.
CLASSES = [
    "Model",
    "Cell",
    "Surface",
    "Material",
    "Transform",
    "Fill",
    "CellParam",
    "Diagnostic",
]


def _stub_path() -> Path:
    # Prefer the maintained source stub; fall back to the installed one.
    source = (
        Path(__file__).resolve().parents[1] / "crates" / "migjorn-py" / "migjorn.pyi"
    )
    if source.exists():
        return source
    return Path(migjorn.__file__).with_name("__init__.pyi")


def _public(names) -> set[str]:
    return {n for n in names if not n.startswith("_")}


def _stub_members() -> dict[str, set[str]]:
    tree = ast.parse(_stub_path().read_text(encoding="utf-8"))
    out: dict[str, set[str]] = {}
    for node in tree.body:
        if isinstance(node, ast.ClassDef):
            members = {
                item.name
                for item in node.body
                if isinstance(item, (ast.FunctionDef, ast.AsyncFunctionDef))
            }
            out[node.name] = _public(members)
    return out


def test_stub_declares_every_documented_class() -> None:
    declared = _stub_members()
    for name in CLASSES:
        assert name in declared, f"{name} missing from stub"
        assert hasattr(migjorn, name), f"{name} missing from runtime module"
    assert hasattr(migjorn, "parse")
    assert hasattr(migjorn, "MergeError")
    assert issubclass(migjorn.MergeError, ValueError)


def test_class_members_match_runtime() -> None:
    stub = _stub_members()
    for name in CLASSES:
        cls = getattr(migjorn, name)
        runtime = _public(dir(cls))
        assert stub[name] == runtime, (
            f"{name}: stub/runtime drift\n"
            f"  only in stub:    {sorted(stub[name] - runtime)}\n"
            f"  only in runtime: {sorted(runtime - stub[name])}"
        )


def _stub_top_level_names() -> set[str]:
    tree = ast.parse(_stub_path().read_text(encoding="utf-8"))
    names = {
        node.name
        for node in tree.body
        if isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef))
    }
    return _public(names)


def test_module_api_matches_stub() -> None:
    """Bidirectional check of the *module-level* public API: a new pyclass or
    #[pyfunction] added to lib.rs (or removed) must show up here even if nobody
    remembers to add it to the hardcoded CLASSES list above."""
    stub_top_level = _stub_top_level_names()
    # `migjorn/__init__.py` does `from .migjorn import *`, which binds the
    # compiled submodule itself under the name `migjorn` as a side effect of
    # the star-import — not part of the public API the stub documents.
    runtime_top_level = _public(dir(migjorn)) - {"migjorn"}
    assert stub_top_level == runtime_top_level, (
        "module-level API drift\n"
        f"  only in stub:    {sorted(stub_top_level - runtime_top_level)}\n"
        f"  only in runtime: {sorted(runtime_top_level - stub_top_level)}"
    )
