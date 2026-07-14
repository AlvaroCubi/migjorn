"""Type stubs for the migjorn MCNP parser (compiled Rust extension).

These mirror the runtime API implemented in `crates/migjorn-py/src/lib.rs`.
Keep them in sync when the bindings change.

Handle attributes are declared as properties so writability is explicit: a
read-only attribute has only a getter, while an assignable one also has a
``@x.setter``. Note that a setter's type can be narrower than its getter's —
e.g. ``Cell.material`` reads ``int | None`` but is assigned an ``int`` (use
``material = 0`` to make a cell void, not ``None``).
"""

from collections.abc import Callable

__version__: str

# A surface/cell number mapping accepted by the renumber methods: either an
# explicit dict (unmapped ids are unchanged) or a callable applied per id.
Mapping = dict[int, int] | Callable[[int], int]

def parse(text: str) -> Model:
    """Parse MCNP ``text`` into a :class:`Model`."""
    ...

class Surface:
    """A surface card (a live handle onto its model).

    ``coeffs`` and ``transform`` are assignable in place. Assigning ``coeffs``
    rewrites every coefficient and requires a list of the same length (changing
    the count raises ``ValueError``); ``transform`` may be set to a number or to
    ``None`` — adding or removing the field is a lossless splice.
    """

    @property
    def id(self) -> int:
        """Surface number."""
        ...

    @property
    def kind(self) -> str:
        """MCNP mnemonic, e.g. ``"PX"``, ``"GQ"``, ``"C/X"``, ``"RPP"``."""
        ...

    @property
    def coeffs(self) -> list[float]:
        """Surface coefficients."""
        ...
    @coeffs.setter
    def coeffs(self, value: list[float]) -> None:
        """Rewrite all coefficients; the list length must match the current
        count (a count change raises ``ValueError``)."""
        ...

    @property
    def transform(self) -> int | None:
        """Transformation number (negative => periodic), or ``None``."""
        ...
    @transform.setter
    def transform(self, value: int | None) -> None:
        """Set, add, or remove the transform number (a lossless splice)."""
        ...

    @property
    def reflective(self) -> bool:
        """Reflective boundary (leading ``*``)."""
        ...

    @property
    def white(self) -> bool:
        """White boundary (leading ``+``)."""
        ...

    @property
    def well_formed(self) -> bool:
        """``False`` if a coefficient could not be parsed."""
        ...

    @property
    def text(self) -> str:
        """The card's exact source text, reflecting any edits."""
        ...

    def set_coeff(self, index: int, value: float) -> None:
        """Set a single coefficient (by index) in place. Raises ``ValueError``
        for an out-of-range index."""
        ...

    def __repr__(self) -> str: ...

class Cell:
    """A cell card (a live handle onto its model).

    ``material`` and ``density`` are assignable in place through the lossless
    engine (``cell.material = 124``). Assigning a real material to a void cell
    gives it a placeholder density of ``0.0`` (set the real value via
    ``density``); assigning ``material = 0`` makes the cell void and drops its
    density.
    """

    @property
    def id(self) -> int:
        """Cell number."""
        ...

    @property
    def material(self) -> int | None:
        """Material number (0 = void); ``None`` for ``LIKE n BUT`` cells."""
        ...
    @material.setter
    def material(self, value: int) -> None:
        """Set the material (``0`` = void). Crossing the void/non-void boundary
        adds or removes the density field losslessly."""
        ...

    @property
    def density(self) -> float | None:
        """Density (positive = atom, negative = mass), or ``None`` for void."""
        ...
    @density.setter
    def density(self, value: float) -> None:
        """Set the density of a non-void cell (raises ``ValueError`` on a void
        cell — assign a ``material`` first)."""
        ...

    @property
    def is_void(self) -> bool:
        """``True`` when the material number is 0."""
        ...

    @property
    def like(self) -> int | None:
        """Base cell number for a ``LIKE n BUT`` card, or ``None``."""
        ...

    @property
    def surface_ids(self) -> list[int]:
        """Referenced surface numbers (magnitudes)."""
        ...

    @property
    def signed_surfaces(self) -> list[int]:
        """Referenced surfaces with sense (sign)."""
        ...

    @property
    def cell_refs(self) -> list[int]:
        """Referenced cells (``#n`` complements, ``LIKE n`` base)."""
        ...

    @property
    def well_formed(self) -> bool:
        """``False`` if the geometry could not be fully parsed."""
        ...

    @property
    def text(self) -> str:
        """The card's exact source text, including inline ``$`` comments and
        continuations, reflecting any edits."""
        ...

    def add_surface(self, surface: int) -> None:
        """Intersect the geometry with a signed surface (negative int = negative
        sense). Spliced in losslessly; the rest of the card stays byte-for-byte."""
        ...

    def remove_surface(self, id: int) -> bool:
        """Remove every reference to surface ``id`` (either sense). Returns
        whether anything was removed; raises ``ValueError`` if it would empty
        the cell's geometry."""
        ...

    def add_complement(self, id: int) -> None:
        """Intersect the geometry with a ``#n`` complement of cell ``id``."""
        ...

    def remove_complement(self, id: int) -> bool:
        """Remove every ``#n`` complement of cell ``id`` from the geometry."""
        ...

    def add_param(self, text: str) -> None:
        """Append a parameter (e.g. ``"imp:n=1"``) to the cell's parameter
        section, spliced in before any trailing inline ``$`` comment."""
        ...

    def remove_param(self, key: str) -> bool:
        """Remove the first parameter whose keyword equals ``key`` (case-
        insensitive, ignoring any ``:particle``). Returns whether one was
        removed."""
        ...

    def __repr__(self) -> str: ...

class Material:
    """A material (``Mn``) card (a live handle onto its model)."""

    @property
    def id(self) -> int:
        """Material number."""
        ...

    @property
    def entries(self) -> list[tuple[str, float]]:
        """``(zaid, fraction)`` pairs; positive = atomic, negative = by weight."""
        ...

    @property
    def well_formed(self) -> bool:
        """``False`` if entries were not clean ZAID/fraction pairs."""
        ...

    @property
    def text(self) -> str:
        """The card's exact source text, reflecting any edits."""
        ...

    def set_fraction(self, entry: int, value: float) -> None:
        """Set the fraction of the ``entry``-th ``(zaid, fraction)`` pair in
        place (positive = atomic, negative = by weight). Raises ``ValueError``
        for an out-of-range index."""
        ...

    def set_zaid(self, entry: int, zaid: str) -> None:
        """Set the ZAID of the ``entry``-th pair in place (e.g.
        ``"1001.31c"``)."""
        ...

    def __repr__(self) -> str: ...

class Transform:
    """A coordinate transformation (``TRn`` / ``*TRn``) card (a live handle).

    ``displacement`` is assignable in place: components already written are
    rewritten, and a component that defaulted to 0 (no token) is spliced in
    losslessly.
    """

    @property
    def id(self) -> int:
        """Transformation number."""
        ...

    @property
    def degrees(self) -> bool:
        """Rotation entries are angles in degrees (``*TRn``)."""
        ...

    @property
    def displacement(self) -> tuple[float, float, float]:
        """Origin displacement."""
        ...
    @displacement.setter
    def displacement(self, value: tuple[float, float, float]) -> None:
        """Set the displacement vector; missing components are filled in
        losslessly."""
        ...

    @property
    def rotation(self) -> list[float]:
        """Rotation entries as written."""
        ...

    @property
    def text(self) -> str:
        """The card's exact source text, reflecting any edits."""
        ...

    def set_rotation(self, rotation: list[float]) -> None:
        """Rewrite the rotation entries in place. Extra entries are spliced in
        and surplus entries deleted losslessly; adding rotation to a transform
        whose displacement is incomplete raises ``ValueError``."""
        ...

    def __repr__(self) -> str: ...

class DataCard:
    """A generically-parsed data card."""

    @property
    def name(self) -> str:
        """Uppercased mnemonic including any number (``"SDEF"``, ``"F4"``)."""
        ...

    @property
    def particle(self) -> str | None:
        """Particle designator after ``:`` (``"n"``, ``"n,p"``), or ``None``."""
        ...

    @property
    def starred(self) -> bool:
        """Leading ``*`` modifier (``*F``, ``*TR``)."""
        ...

    def __repr__(self) -> str: ...

class Diagnostic:
    """A parser diagnostic."""

    @property
    def severity(self) -> str:
        """``"error"`` or ``"warning"``."""
        ...

    @property
    def message(self) -> str:
        """Human-readable description."""
        ...

    @property
    def start(self) -> int:
        """Byte offset where the span begins."""
        ...

    @property
    def end(self) -> int:
        """Byte offset where the span ends."""
        ...

    def __repr__(self) -> str: ...

class Model:
    """A parsed MCNP model: lossless tree, diagnostics, typed access, editing."""

    def __init__(self, text: str) -> None: ...
    @staticmethod
    def from_file(path: str) -> Model:
        """Parse the MCNP model at ``path``."""
        ...

    def to_source(self) -> str:
        """Re-emit the model as MCNP text (lossless when unedited)."""
        ...

    def save(self, path: str) -> None:
        """Write the model to ``path``."""
        ...

    def __str__(self) -> str: ...
    def __repr__(self) -> str: ...
    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Parser diagnostics (empty on a clean parse)."""
        ...

    @property
    def cells(self) -> list[Cell]:
        """All cells, in source order (materialises a list)."""
        ...

    @property
    def surfaces(self) -> list[Surface]:
        """All surfaces, in source order (materialises a list)."""
        ...

    @property
    def transforms(self) -> list[Transform]:
        """All ``TRn`` transforms, in source order."""
        ...

    @property
    def materials(self) -> list[Material]:
        """All ``Mn`` materials, in source order."""
        ...

    @property
    def data_cards(self) -> list[DataCard]:
        """All data cards (generic view), in source order."""
        ...

    @property
    def num_cells(self) -> int:
        """Number of cells (cheap; does not build the cell list)."""
        ...

    @property
    def num_surfaces(self) -> int:
        """Number of surfaces (cheap; does not build the surface list)."""
        ...

    @property
    def num_materials(self) -> int:
        """Number of materials (cheap; does not build the material list)."""
        ...

    @property
    def num_transforms(self) -> int:
        """Number of transforms (cheap; does not build the transform list)."""
        ...

    def surface(self, id: int) -> Surface | None:
        """Look up a surface by number, or ``None``."""
        ...

    def cell(self, id: int) -> Cell | None:
        """Look up a cell by number, or ``None``."""
        ...

    def material(self, id: int) -> Material | None:
        """Look up a material by number, or ``None``."""
        ...

    def transform(self, id: int) -> Transform | None:
        """Look up a transform by number, or ``None``."""
        ...

    def renumber_surfaces(self, mapping: Mapping) -> None:
        """Renumber every surface (definitions and references) via ``mapping``."""
        ...

    def renumber_cells(self, mapping: Mapping) -> None:
        """Renumber every cell (definitions, ``#n``, ``LIKE n``) via ``mapping``."""
        ...

    def renumber_materials(self, mapping: Mapping) -> None:
        """Renumber every material (``Mn`` defs, cell material fields,
        ``MT``/``MX``) via ``mapping``. Void cells are left unchanged."""
        ...

    def renumber_transforms(self, mapping: Mapping) -> None:
        """Renumber every transform (``TRn``/``*TRn`` defs and surface transform
        fields, periodic sign preserved) via ``mapping``."""
        ...

    def renumber_universes(self, mapping: Mapping) -> None:
        """Renumber every universe (``u=`` defs and ``fill=`` references,
        including lattice fill arrays) via ``mapping``. Universe 0 is left
        unchanged."""
        ...

    def renumber_tallies(self, mapping: Mapping) -> None:
        """Renumber every tally *id* (``Fn`` and companion cards) via
        ``mapping``. Cell/surface ids inside tally bins are updated by
        ``renumber_cells`` / ``renumber_surfaces`` instead."""
        ...

    def offset_surfaces(self, delta: int) -> None:
        """Shift every surface number by ``delta``."""
        ...

    def offset_cells(self, delta: int) -> None:
        """Shift every cell number by ``delta``."""
        ...

    def add_cell(self, text: str) -> Cell:
        """Add a new cell from an MCNP card body (e.g. ``"10 6 -7.85 -5 6
        imp:n=1"``), appended to the cell block. Returns a live handle. Raises
        ``ValueError`` if the text is not exactly one well-formed cell."""
        ...

    def add_surface(self, text: str) -> Surface:
        """Add a new surface from an MCNP card body (e.g. ``"5 SO 12.0"``),
        appended to the surface block. Returns a live handle."""
        ...

    def add_material(self, text: str) -> Material:
        """Add a new material from an MCNP card body (e.g. ``"m7 26000 -1.0"``),
        appended to the data block. Returns a live handle."""
        ...

    def remove_cell(self, id: int) -> bool:
        """Remove the cell numbered ``id``. Returns whether one was removed."""
        ...

    def remove_surface(self, id: int) -> bool:
        """Remove the surface numbered ``id``. Returns whether one was removed."""
        ...

    def validate(self) -> list[str]:
        """Referential-integrity check: a list of human-readable problems
        (dangling surface/cell/material references), empty when consistent."""
        ...
