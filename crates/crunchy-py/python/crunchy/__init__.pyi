"""Type stubs for the crunchy MCNP parser (compiled Rust extension).

These mirror the runtime API implemented in `crates/crunchy-py/src/lib.rs`.
Keep them in sync when the bindings change.
"""

from collections.abc import Callable
from typing import Optional, Union

__version__: str

# A surface/cell number mapping accepted by the renumber methods: either an
# explicit dict (unmapped ids are unchanged) or a callable applied per id.
Mapping = Union[dict[int, int], Callable[[int], int]]

def parse(text: str) -> Deck:
    """Parse MCNP ``text`` into a :class:`Deck`."""
    ...

class Surface:
    """A surface card."""

    id: int
    """Surface number."""
    kind: str
    """MCNP mnemonic, e.g. ``"PX"``, ``"GQ"``, ``"C/X"``, ``"RPP"``."""
    coeffs: list[float]
    """Surface coefficients."""
    transform: Optional[int]
    """Transformation number (negative => periodic), or ``None``."""
    reflective: bool
    """Reflective boundary (leading ``*``)."""
    white: bool
    """White boundary (leading ``+``)."""
    well_formed: bool
    """``False`` if a coefficient could not be parsed."""
    def __repr__(self) -> str: ...

class Cell:
    """A cell card."""

    id: int
    """Cell number."""
    material: Optional[int]
    """Material number (0 = void); ``None`` for ``LIKE n BUT`` cells."""
    density: Optional[float]
    """Density (positive = atom, negative = mass), or ``None`` for void."""
    is_void: bool
    """``True`` when the material number is 0."""
    like: Optional[int]
    """Base cell number for a ``LIKE n BUT`` card, or ``None``."""
    surface_ids: list[int]
    """Referenced surface numbers (magnitudes)."""
    signed_surfaces: list[int]
    """Referenced surfaces with sense (sign)."""
    cell_refs: list[int]
    """Referenced cells (``#n`` complements, ``LIKE n`` base)."""
    well_formed: bool
    """``False`` if the geometry could not be fully parsed."""
    def __repr__(self) -> str: ...

class Material:
    """A material (``Mn``) card."""

    id: int
    """Material number."""
    entries: list[tuple[str, float]]
    """``(zaid, fraction)`` pairs; positive = atomic, negative = by weight."""
    well_formed: bool
    """``False`` if entries were not clean ZAID/fraction pairs."""
    def __repr__(self) -> str: ...

class Transform:
    """A coordinate transformation (``TRn`` / ``*TRn``) card."""

    id: int
    """Transformation number."""
    degrees: bool
    """Rotation entries are angles in degrees (``*TRn``)."""
    displacement: tuple[float, float, float]
    """Origin displacement."""
    rotation: list[float]
    """Rotation entries as written."""
    def __repr__(self) -> str: ...

class DataCard:
    """A generically-parsed data card."""

    name: str
    """Uppercased mnemonic including any number (``"SDEF"``, ``"F4"``)."""
    particle: Optional[str]
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

class Deck:
    """A parsed MCNP deck: lossless tree, diagnostics, typed access, editing."""

    def __init__(self, text: str) -> None: ...
    @staticmethod
    def from_file(path: str) -> Deck:
        """Parse the MCNP deck at ``path``."""
        ...

    def to_source(self) -> str:
        """Re-emit the deck as MCNP text (lossless when unedited)."""
        ...

    def save(self, path: str) -> None:
        """Write the deck to ``path``."""
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

    def surface(self, id: int) -> Optional[Surface]:
        """Look up a surface by number, or ``None``."""
        ...

    def cell(self, id: int) -> Optional[Cell]:
        """Look up a cell by number, or ``None``."""
        ...

    def material(self, id: int) -> Optional[Material]:
        """Look up a material by number, or ``None``."""
        ...

    def transform(self, id: int) -> Optional[Transform]:
        """Look up a transform by number, or ``None``."""
        ...

    def renumber_surfaces(self, mapping: Mapping) -> None:
        """Renumber every surface (definitions and references) via ``mapping``."""
        ...

    def renumber_cells(self, mapping: Mapping) -> None:
        """Renumber every cell (definitions, ``#n``, ``LIKE n``) via ``mapping``."""
        ...

    def offset_surfaces(self, delta: int) -> None:
        """Shift every surface number by ``delta``."""
        ...

    def offset_cells(self, delta: int) -> None:
        """Shift every cell number by ``delta``."""
        ...
