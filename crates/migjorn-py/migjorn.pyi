"""Type stub for the `migjorn` extension module.

Kept in lockstep with the PyO3 surface in `src/lib.rs`; `tests/test_stub.py`
fails if they drift. Handles (`Cell`, `Surface`, ...) are live: they resolve
against current model state on every access, and using a handle whose card was
removed raises `ValueError`.
"""

from typing import TypeAlias

__version__: str

Mapping: TypeAlias = dict[int, int]
"""Old id -> new id; ids absent from it are left unchanged."""

def parse(text: str) -> Model:
    """Parse MCNP input text into a :class:`Model`. Never raises on malformed
    input — problems are collected in :attr:`Model.diagnostics` and the bytes are
    preserved. Equivalent to ``Model(text)``."""

class Model:
    """A parsed MCNP model: lossless, typed access, cheap iterative editing.

    ``parse(t).to_source() == t`` byte-for-byte, and stays lossless except at
    cards you edit. No operation re-parses the whole file, and reads always
    reflect the latest edit with no explicit flush.
    """

    def __init__(self, text: str) -> None:
        """Parse MCNP input ``text`` (see :func:`parse`)."""

    @staticmethod
    def from_file(path: str) -> Model:
        """Read and parse the file at ``path`` (UTF-8)."""

    def to_source(self) -> str:
        """Re-emit the model as MCNP text — byte-identical to the input when
        unedited, and identical except at edited cards otherwise. O(bytes)."""

    def save(self, path: str) -> None:
        """Write :meth:`to_source` to ``path``."""

    @property
    def diagnostics(self) -> list[Diagnostic]:
        """Problems found while parsing (unrecognised tokens, malformed cards).
        Empty on clean input; a model with diagnostics still round-trips."""

    @property
    def title(self) -> str | None:
        """The model's title line, or ``None`` if the model has no title card
        (e.g. an empty input). Assigning replaces it, creating one — positioned
        correctly relative to a leading ``MESSAGE`` block — if none exists."""

    @title.setter
    def title(self, value: str) -> None: ...
    @property
    def num_cells(self) -> int:
        """Number of cell cards."""

    @property
    def num_surfaces(self) -> int:
        """Number of surface cards."""

    @property
    def num_materials(self) -> int:
        """Number of ``Mn`` material cards."""

    @property
    def num_transforms(self) -> int:
        """Number of ``TRn`` / ``*TRn`` transform cards."""

    def cell(self, id: int) -> Cell | None:
        """The cell with this id, or ``None`` if none is defined. O(1)."""

    def surface(self, id: int) -> Surface | None:
        """The surface with this id, or ``None``. O(1)."""

    def material(self, id: int) -> Material | None:
        """The ``Mn`` material with this id, or ``None``. O(1)."""

    def transform(self, id: int) -> Transform | None:
        """The ``TRn`` transform with this id, or ``None``. O(1)."""

    def cells(self) -> list[Cell]:
        """All cells, in file order."""

    def surfaces(self) -> list[Surface]:
        """All surfaces, in file order."""

    def materials(self) -> list[Material]:
        """All ``Mn`` material cards, in file order."""

    def transforms(self) -> list[Transform]:
        """All ``TRn`` / ``*TRn`` transform cards, in file order."""

    def data_cards(self) -> list[DataCard]:
        """All data cards, in file order — a superset of :meth:`materials` and
        :meth:`transforms` (``Mn``/``TRn`` are ``Data`` cards too), generically."""

    def add_cell(self, text: str) -> Cell:
        """Append a cell card (given as one line of MCNP, no terminator needed) at
        the end of the cell block and return a live handle to it. Local: no
        reparse. Raises ``ValueError`` if there is no cell block to add to."""

    def add_surface(self, text: str) -> Surface:
        """Append a surface card at the end of the surface block; returns a handle."""

    def add_material(self, text: str) -> Material:
        """Append an ``Mn`` card at the end of the data block; returns a handle."""

    def add_transform(self, text: str) -> Transform:
        """Append a ``TRn`` card at the end of the data block; returns a handle."""

    def add_data_card(self, text: str) -> DataCard:
        """Append a generic data card at the end of the data block; returns a
        handle. Cards with a dedicated constructor (``Mn``, ``TRn``) should
        normally go through :meth:`add_material` / :meth:`add_transform`
        instead, so their id is indexed immediately — this is for everything
        else (``sdef``, ``mode``, ``kcode``, ``print``, ...)."""

    def remove_cell(self, id: int) -> bool:
        """Remove the cell with this id. Returns ``True`` if one was removed,
        ``False`` if the id was absent. Handles to *other* cards stay valid."""

    def remove_surface(self, id: int) -> bool:
        """Remove the surface with this id; ``True`` if one was removed."""

    def remove_material(self, id: int) -> bool:
        """Remove the ``Mn`` material with this id; ``True`` if one was removed."""

    def remove_transform(self, id: int) -> bool:
        """Remove the ``TRn`` transform with this id; ``True`` if one was removed."""

    def renumber_cells(self, mapping: Mapping) -> None:
        """Renumber cells through ``mapping``, moving each cell definition
        **and** every reference to it (``#n`` complements, ``LIKE n`` bases)
        together."""

    def renumber_surfaces(self, mapping: Mapping) -> None:
        """Renumber surfaces, moving each definition (keeping a ``+`` white prefix)
        and every geometry reference (keeping its ``-``/``+`` sense) together."""

    def renumber_materials(self, mapping: Mapping) -> None:
        """Renumber materials: ``Mn`` definitions, the cell material fields that
        use them, and the ``MTn`` / ``MXn`` cards that reference them."""

    def renumber_transforms(self, mapping: Mapping) -> None:
        """Renumber transforms: ``TRn`` / ``*TRn`` definitions and the surface
        transform fields that reference them (a ``-`` periodic sign is kept)."""

    def renumber_universes(self, mapping: Mapping) -> None:
        """Renumber universes in cell ``u=`` and single-universe ``fill=``
        parameters. Lattice ``fill=`` arrays are left untouched."""

    def renumber_tallies(self, mapping: Mapping) -> None:
        """Renumber the trailing id of tally-family cards (``Fn``, ``FCn``, ``En``,
        ...). Cell/surface *bins* move with :meth:`renumber_cells` /
        :meth:`renumber_surfaces`."""

    def offset_cells(self, delta: int) -> None:
        """Shift every cell id by ``delta``."""

    def offset_surfaces(self, delta: int) -> None:
        """Shift every surface id by ``delta``, definitions and references together."""

    def validate(self) -> list[str]:
        """Return human-readable messages for dangling references — a cell that
        points at an undefined surface / material / cell / universe, or a surface
        at an undefined transform. Empty means every reference resolves."""

    def universe_ids(self) -> list[int]:
        """The distinct universe ids declared by cells (``u=``), ascending."""

    def extract_universe(self, u: int) -> Model:
        """A new standalone model of universe ``u``'s cells and the surfaces,
        materials and transforms they use. Does not recurse into filled
        sub-universes — use :meth:`extract_cells` for that."""

    def extract_level0(self) -> Model:
        """A new standalone model of the level-0 (root) cells — those with no
        ``u=`` — and their surfaces, materials and transforms. Does not recurse
        into filled sub-universes — use :meth:`extract_cells` for that."""

    def extract_cells(self, ids: list[int]) -> Model:
        """A new standalone model of the given cells and everything they need
        to be self-contained: their surfaces, materials and transforms, plus —
        recursively — any cell reached through a ``fill=`` (pulls in that
        universe's own cells), a ``LIKE n BUT`` base, or a ``#n`` complement.
        Ids that don't name an existing cell are silently ignored."""

    def merge(self, others: list[Model]) -> None:
        """Fold the cells, surfaces and data cards of ``others`` into this model.
        Raises :class:`MergeError` (without changing anything) if any cell /
        surface / material / transform id is shared across the models."""

class Cell:
    """A live handle onto a cell card. Reads reflect current state; a handle to a
    removed cell raises ``ValueError`` on use."""

    @property
    def id(self) -> int | None:
        """The cell id, or ``None`` if it cannot be read."""

    @property
    def material(self) -> int | None:
        """The material number; ``0`` means void (no density). Setting it crosses
        the void boundary as needed — void→real adds a placeholder density field,
        real→void drops it."""

    @material.setter
    def material(self, value: int) -> None: ...
    @property
    def density(self) -> float | None:
        """The density (negative = mass g/cm³, positive = atom density), or
        ``None`` for a void cell. Setting it on a void cell raises ``ValueError``."""

    @density.setter
    def density(self, value: float) -> None: ...
    @property
    def is_void(self) -> bool:
        """``True`` when the material number is ``0``."""

    @property
    def like(self) -> int | None:
        """The base cell of a ``LIKE n BUT`` card, or ``None``."""

    @property
    def universe(self) -> int | None:
        """The cell's ``u=`` universe, or ``None`` (level 0)."""

    @property
    def signed_surfaces(self) -> list[int]:
        """Surfaces referenced by the geometry, in file order, with their sense:
        ``-1`` keeps its negative sense, ``5`` its positive."""

    @property
    def surface_ids(self) -> list[int]:
        """Magnitudes of every referenced surface (senses dropped). Surfaces inside
        a ``#( ... )`` region count; a bare ``#n`` (a cell) does not."""

    @property
    def cell_refs(self) -> list[int]:
        """Cells referenced by a ``#n`` complement, plus a ``LIKE n`` base."""

    @property
    def fill(self) -> Fill | None:
        """The single-universe ``fill=`` entry, or ``None`` (absent, or a lattice
        array form)."""

    @property
    def params(self) -> list[CellParam]:
        """All trailing keyword parameters (``imp:n=1``, ``vol=3``, ``u=5`` ...)."""

    @property
    def geometry(self) -> list[GeometryTerm]:
        """The geometry expression as a flat list of terms, in file order:
        signed surfaces, ``#n``/``#(...)`` complements, ``(``, ``)``, ``:``
        (union). Addressed for editing by position in this list — see
        :meth:`set_geometry_term` / :meth:`insert_geometry_term`."""

    @property
    def well_formed(self) -> bool:
        """Whether the card parsed as a structurally complete cell."""

    @property
    def text(self) -> str:
        """The card's exact current source text, terminator included."""

    def param(self, key: str) -> CellParam | None:
        """The parameter with this qualified key (``imp:n``, ``vol``, ...), matched
        case-insensitively, or ``None``."""

    def set_param(self, key: str, value: str) -> bool:
        """Set the value of an existing parameter. Returns ``False`` (no change) if
        the cell has no such parameter — use :meth:`add_param` to add one. Other
        parameters and a trailing ``$`` comment are preserved."""

    def add_param(self, text: str) -> None:
        """Append a keyword parameter (e.g. ``"imp:n=1"``), placing it after the
        last value and before any trailing ``$`` comment."""

    def remove_param(self, key: str) -> bool:
        """Remove the parameter with this qualified key; ``True`` if one was
        removed."""

    def append_comment(self, text: str) -> None:
        """Append an inline ``$`` comment before the line terminator. A ``$`` is
        prepended if ``text`` does not already start with one."""

    def set_geometry_term(self, position: int, text: str) -> None:
        """Replace the geometry term at ``position`` (0-based, into the list
        :attr:`geometry` returns) with new literal text — ``"124"`` for a
        surface, ``"#457"`` for a whole complement. The replacement can be a
        different length, but the term count is unchanged; use
        :meth:`insert_geometry_term` to grow the expression. Raises
        ``ValueError`` if ``position`` is out of range."""

    def insert_geometry_term(self, position: int, text: str) -> None:
        """Insert a new term into the geometry expression at ``position`` — the
        same list :attr:`geometry` returns, so ``position == len(cell.geometry)``
        appends after the last term, matching ``list.insert``. This is the one
        structural geometry primitive; everything else (wrapping in
        parentheses, adding a union or a cell complement) composes from
        repeated calls. Hashing a cell's geometry with cell 123:

        .. code-block:: python

            n = len(cell.geometry)          # read once, before any insert
            cell.insert_geometry_term(0, "(")
            cell.insert_geometry_term(n + 1, ")")
            cell.insert_geometry_term(n + 2, "#123")

        Raises ``ValueError`` if ``position`` is past the end of the list."""

class Surface:
    """A live handle onto a surface card."""

    @property
    def id(self) -> int | None:
        """The surface id, or ``None``."""

    @property
    def kind(self) -> str | None:
        """The surface mnemonic as written (``SO``, ``PX``, ``RPP`` ...)."""

    @property
    def coeffs(self) -> list[float]:
        """The surface coefficients. Assigning a list replaces them all."""

    @coeffs.setter
    def coeffs(self, value: list[float]) -> None: ...
    @property
    def transform(self) -> int | None:
        """The transform number applied to the surface (negative = periodic), or
        ``None``. Assign ``None`` to remove it, an int to set/insert it."""

    @transform.setter
    def transform(self, value: int | None) -> None: ...
    @property
    def reflective(self) -> bool:
        """Whether the surface has a leading ``*`` (reflective boundary)."""

    @property
    def white(self) -> bool:
        """Whether the surface has a leading ``+`` (white boundary)."""

    @property
    def well_formed(self) -> bool:
        """Whether the card parsed as a structurally complete surface."""

    @property
    def text(self) -> str:
        """The card's exact current source text."""

    def set_coeff(self, index: int, value: float) -> None:
        """Replace the coefficient at ``index`` (0-based)."""

class Material:
    """A live handle onto an ``Mn`` material card."""

    @property
    def id(self) -> int | None:
        """The material number ``n`` of the ``Mn`` card, or ``None``."""

    @property
    def entries(self) -> list[tuple[str, float]]:
        """``(zaid, fraction)`` pairs. The ZAID keeps its library suffix; a
        negative fraction is by weight, a positive one by atom."""

    @property
    def well_formed(self) -> bool:
        """Whether every entry parsed as a ``zaid fraction`` pair."""

    @property
    def text(self) -> str:
        """The card's exact current source text."""

    def set_fraction(self, entry: int, value: float) -> None:
        """Set the fraction of the entry at ``entry`` (0-based); sign kept as given."""

    def set_zaid(self, entry: int, zaid: str) -> None:
        """Set the ZAID of the entry at ``entry`` (include any library suffix)."""

class Transform:
    """A live handle onto a ``TRn`` / ``*TRn`` transform card."""

    @property
    def id(self) -> int | None:
        """The transform number ``n``, or ``None``."""

    @property
    def degrees(self) -> bool:
        """``True`` for a ``*TRn`` card (rotation entries are angles in degrees)."""

    @property
    def displacement(self) -> list[float]:
        """The first three coefficients — the origin displacement."""

    @property
    def coeffs(self) -> list[float]:
        """All transform coefficients. Assigning a list replaces them all."""

    @coeffs.setter
    def coeffs(self, value: list[float]) -> None: ...
    @property
    def text(self) -> str:
        """The card's exact current source text."""

class DataCard:
    """A live handle onto any data card — the generic superset view. This
    includes ``Mn`` and ``TRn`` cards too (they are ``Data`` cards like
    everything else); prefer :class:`Material` / :class:`Transform` for
    those, since they have an id maintained in the model's index. ``DataCard``
    has none, so it is addressed and removed by its own handle instead."""

    @property
    def text(self) -> str:
        """The card's exact current source text. Assigning replaces the whole
        card in place."""

    @text.setter
    def text(self, value: str) -> None: ...
    @property
    def name(self) -> str | None:
        """The card's name as written, id included: ``m1``, ``f4``, ``sdef``."""

    @property
    def particle(self) -> str | None:
        """The ``:particle`` designator after the name, if any."""

    @property
    def starred(self) -> bool:
        """Whether the card has a leading ``*`` (e.g. ``*tr1``)."""

    def remove(self) -> bool:
        """Remove this card. Returns ``True`` if it was removed, ``False`` if
        it had already been removed. If this handle happens to address a
        material or transform, the model's id index is cleaned up too."""

class Fill:
    """A cell's single-universe ``fill=`` entry."""

    @property
    def universe(self) -> int:
        """The filled universe number."""

    @property
    def starred(self) -> bool:
        """``True`` for the ``*fill=`` form (transform angles in degrees)."""

    @property
    def transform(self) -> str | None:
        """The parenthesised transform following the universe, verbatim, or
        ``None``."""

class CellParam:
    """One ``keyword[:particle][=]value`` entry on a cell card."""

    @property
    def key(self) -> str:
        """The keyword (``imp``, ``vol``, ``fill`` ...)."""

    @property
    def particle(self) -> str | None:
        """The particle designator after a ``:`` (``n`` in ``imp:n``), or ``None``."""

    @property
    def starred(self) -> bool:
        """Whether the keyword carries a leading ``*`` (e.g. ``*fill``, ``*trcl``)."""

    @property
    def value(self) -> str:
        """The parameter's value text, exactly as written."""

class GeometryTerm:
    """One term of a cell's geometry expression, in file order. ``str(term)``
    is its exact text; ``repr(term)`` shows ``kind`` and ``text`` together."""

    @property
    def kind(self) -> str:
        """One of ``"surface"``, ``"complement"``, ``"lparen"``, ``"rparen"``,
        ``"union"``."""

    @property
    def text(self) -> str:
        """The term's exact text: a signed surface number (``"-1"``), a whole
        complement (``"#123"``), or the operator itself (``"("``, ``")"``,
        ``":"``)."""

class Diagnostic:
    """A problem recorded while parsing; the bytes are still preserved."""

    @property
    def severity(self) -> str:
        """``"error"`` or ``"warning"``."""

    @property
    def message(self) -> str:
        """A human-readable description of the problem."""

    @property
    def start(self) -> int:
        """Byte offset of the problem span in the source as parsed."""

    @property
    def end(self) -> int:
        """End byte offset (exclusive) of the problem span."""

class MergeError(ValueError):
    """Raised by :meth:`Model.merge` on a shared cell/surface/material/transform
    id."""
