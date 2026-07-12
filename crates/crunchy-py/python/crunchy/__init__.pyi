"""Type stubs for the crunchy MCNP parser (compiled Rust extension).

These mirror the runtime API implemented in `crates/crunchy-py/src/lib.rs`.
Keep them in sync when the bindings change.
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
    """A surface card (a live handle onto its model)."""

    id: int
    """Surface number."""
    kind: str
    """MCNP mnemonic, e.g. ``"PX"``, ``"GQ"``, ``"C/X"``, ``"RPP"``."""
    coeffs: list[float]
    """Surface coefficients."""
    transform: int | None
    """Transformation number (negative => periodic), or ``None``."""
    reflective: bool
    """Reflective boundary (leading ``*``)."""
    white: bool
    """White boundary (leading ``+``)."""
    well_formed: bool
    """``False`` if a coefficient could not be parsed."""
    text: str
    """The card's exact source text, reflecting any edits."""
    def __repr__(self) -> str: ...

class Cell:
    """A cell card (a live handle onto its model).

    ``material`` and ``density`` are assignable in place; the edit writes
    straight through the lossless engine (``cell.material = 124``). Changing a
    cell between void (material 0) and a real material, or adding a density to a
    void cell, needs a structural edit and raises ``ValueError``.
    """

    id: int
    """Cell number."""
    material: int | None
    """Material number (0 = void); ``None`` for ``LIKE n BUT`` cells. Writable."""
    density: float | None
    """Density (positive = atom, negative = mass), or ``None`` for void. Writable."""
    is_void: bool
    """``True`` when the material number is 0."""
    like: int | None
    """Base cell number for a ``LIKE n BUT`` card, or ``None``."""
    surface_ids: list[int]
    """Referenced surface numbers (magnitudes)."""
    signed_surfaces: list[int]
    """Referenced surfaces with sense (sign)."""
    cell_refs: list[int]
    """Referenced cells (``#n`` complements, ``LIKE n`` base)."""
    well_formed: bool
    """``False`` if the geometry could not be fully parsed."""
    text: str
    """The card's exact source text, including inline ``$`` comments and
    continuations, reflecting any edits."""

    def add_surface(self, surface: int) -> None:
        """Intersect the geometry with a signed surface (negative int = negative
        sense). Restructures the card; other cards stay byte-for-byte."""
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

    def __repr__(self) -> str: ...

class Material:
    """A material (``Mn``) card (a live handle onto its model)."""

    id: int
    """Material number."""
    entries: list[tuple[str, float]]
    """``(zaid, fraction)`` pairs; positive = atomic, negative = by weight."""
    well_formed: bool
    """``False`` if entries were not clean ZAID/fraction pairs."""
    text: str
    """The card's exact source text, reflecting any edits."""
    def __repr__(self) -> str: ...

class Transform:
    """A coordinate transformation (``TRn`` / ``*TRn``) card (a live handle)."""

    id: int
    """Transformation number."""
    degrees: bool
    """Rotation entries are angles in degrees (``*TRn``)."""
    displacement: tuple[float, float, float]
    """Origin displacement."""
    rotation: list[float]
    """Rotation entries as written."""
    text: str
    """The card's exact source text, reflecting any edits."""
    def __repr__(self) -> str: ...

class DataCard:
    """A generically-parsed data card."""

    name: str
    """Uppercased mnemonic including any number (``"SDEF"``, ``"F4"``)."""
    particle: str | None
    """Particle designator after ``:`` (``"n"``, ``"n,p"``), or ``None``."""
    starred: bool
    """Leading ``*`` modifier (``*F``, ``*TR``)."""
    def __repr__(self) -> str: ...

class Diagnostic:
    """A parser diagnostic."""

    severity: str
    """``"error"`` or ``"warning"``."""
    message: str
    """Human-readable description."""
    start: int
    """Byte offset where the span begins."""
    end: int
    """Byte offset where the span ends."""
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
