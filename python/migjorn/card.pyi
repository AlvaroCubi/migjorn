"""Protocol defining the common interface shared by all card types."""

from typing import Protocol

class Card(Protocol):
    """Common interface implemented by all MCNP card types.

    ``SurfaceCard``, ``CellCard``, ``MaterialCard``, ``TransformCard``, and
    ``OtherCard`` all satisfy this protocol, so any function that only needs
    the shared behaviour can accept ``Card`` instead of a concrete type.
    """

    @property
    def card_id(self) -> int | str:
        """The card identifier.

        Returns an ``int`` for surface, cell, material, and transform cards;
        a ``str`` (the keyword) for ``OtherCard``.
        """
        ...

    @card_id.setter
    def card_id(self, new_id: int | str) -> None:
        """Set the card identifier."""
        ...

    @property
    def original_text(self) -> str:
        """Return the raw card text exactly as it appeared in the source file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the card text with all in-memory modifications applied."""
        ...
