"""Guard against type-stub drift.

Parses the shipped stub (`crunchy/__init__.pyi`) and compares its public API to
the runtime API of the compiled extension. Fails if the Rust bindings expose a
public member the stub does not declare (undocumented) or the stub declares one
that no longer exists (stale). Run via `pytest`, or directly for a quick check.
"""

import ast
import os

import crunchy

PUBLIC_CLASSES = ["Deck", "Surface", "Cell", "Material", "Transform", "DataCard", "Diagnostic"]


def _load_stub() -> ast.Module:
    pyi = os.path.join(os.path.dirname(crunchy.__file__), "__init__.pyi")
    assert os.path.exists(pyi), f"type stub not found next to the package: {pyi}"
    with open(pyi, encoding="utf-8") as f:
        return ast.parse(f.read())


def _public(names) -> set:
    return {n for n in names if not n.startswith("_")}


def _stub_module_api(tree: ast.Module) -> set:
    """Public functions and classes declared at module level in the stub."""
    api = set()
    for node in tree.body:
        if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef)):
            if not node.name.startswith("_"):
                api.add(node.name)
    return api


def _stub_class_members(tree: ast.Module, cls: str) -> set:
    """Public methods, properties, and annotated attributes declared for `cls`."""
    for node in tree.body:
        if isinstance(node, ast.ClassDef) and node.name == cls:
            members = set()
            for b in node.body:
                if isinstance(b, (ast.FunctionDef, ast.AsyncFunctionDef)):
                    members.add(b.name)
                elif isinstance(b, ast.AnnAssign) and isinstance(b.target, ast.Name):
                    members.add(b.target.id)
                elif isinstance(b, ast.Assign):
                    members.update(t.id for t in b.targets if isinstance(t, ast.Name))
            return _public(members)
    raise AssertionError(f"class {cls} not declared in the stub")


def _runtime_module_api() -> set:
    api = set()
    for n in dir(crunchy):
        if n.startswith("_"):
            continue
        obj = getattr(crunchy, n)
        if isinstance(obj, type) or callable(obj):
            api.add(n)
    return api


def test_module_api_matches_stub():
    tree = _load_stub()
    assert _runtime_module_api() == _stub_module_api(tree)


def test_class_members_match_stub():
    tree = _load_stub()
    for cls in PUBLIC_CLASSES:
        runtime = _public(dir(getattr(crunchy, cls)))
        stub = _stub_class_members(tree, cls)
        assert runtime == stub, (
            f"{cls}: stub is out of sync with the runtime API\n"
            f"  only in runtime (add to stub):  {sorted(runtime - stub)}\n"
            f"  only in stub (remove/rename):   {sorted(stub - runtime)}"
        )


if __name__ == "__main__":
    test_module_api_matches_stub()
    test_class_members_match_stub()
    print("stub API matches runtime")
