from pathlib import Path

from migjorn.cell_card import CellCard
from migjorn.data_card import MaterialCard, TallyCard, TransformCard, UnparsedCard
from migjorn.surface import SurfaceCard

class Model:
    """An MCNP input file loaded into memory.

    Load a model with ``Model.from_file(path)`` or ``Model(text)``, inspect or mutate individual
    cards, then call ``write_to_file(path)`` to write it back.

    Example::

        from migjorn import Model
        m = Model.from_file("input.i")
        for cell in m.cells:
            print(cell.cell_id, cell.material_id)
        m.write_to_file("output.i")
    """
    def __init__(self, text: str) -> None: ...
    @staticmethod
    def from_file(path: str | Path) -> Model:
        """Load an MCNP input file and return a ``Model`` instance.

        Raises ``IOError`` if the file cannot be opened or read.
        """
        ...
    @staticmethod
    def from_text(text: str) -> Model:
        """Load an MCNP model from a string and return a ``Model`` instance."""
        ...
    @property
    def title(self) -> str:
        """The title of the model, taken from the first line of the input file."""
        ...
    @property
    def cells(self) -> list[CellCard]:
        """All successfully parsed cell cards, in input order."""
        ...
    @property
    def surfaces(self) -> list[SurfaceCard]:
        """All successfully parsed surface cards, in input order."""
        ...
    @property
    def materials(self) -> list[MaterialCard]:
        """All successfully parsed material cards, in input order."""
        ...
    @property
    def transforms(self) -> list[TransformCard]:
        """All successfully parsed transformation cards, in input order."""
        ...
    @property
    def tallies(self) -> list[TallyCard]:
        """All successfully parsed tally cards, in input order."""
        ...
    @property
    def unparsed_data_cards(self) -> list[UnparsedCard]:
        """Data cards that were not parsed in detail, in input order."""
        ...
    def write_to_file(self, path: str | Path) -> None:
        """Write the (possibly modified) model back to *path*.

        Cards are re-serialized from their in-memory representation.
        Raises ``IOError`` on failure.
        """
        ...
    def renumber_cells(self, offset: int, range: tuple[int, int] | None = None) -> None:
        """Renumber cell IDs by adding *offset* to them. If *range* is given, only cell IDs
        in the range ``[range[0], range[1]]`` are renumbered."""
        ...
    def renumber_surfaces(
        self, offset: int, range: tuple[int, int] | None = None
    ) -> None:
        """Renumber surface IDs by adding *offset* to them. If *range* is given, only surface IDs
        in the range ``[range[0], range[1]]`` are renumbered."""
        ...
    def renumber_materials(
        self, offset: int, range: tuple[int, int] | None = None
    ) -> None:
        """Renumber material IDs by adding *offset* to them. If *range* is given, only material IDs
        in the range ``[range[0], range[1]]`` are renumbered."""
        ...
    def renumber_transformations(
        self, offset: int, range: tuple[int, int] | None = None
    ) -> None:
        """Renumber transformation IDs by adding *offset* to them. If *range* is given, only transformation IDs
        in the range ``[range[0], range[1]]`` are renumbered."""
        ...
    def renumber_universes(
        self, offset: int, range: tuple[int, int] | None = None
    ) -> None:
        """Renumber universe IDs by adding *offset* to them. If *range* is given, only universe IDs
        in the range ``[range[0], range[1]]`` are renumbered."""
        ...
    def validation_checks(self) -> None:
        """Run validation checks on the model.

        Returns ``None`` if the model is consistent.
        Raises ``ValueError`` with a report string if any checks fail.
        """
        ...
