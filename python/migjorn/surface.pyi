from migjorn.card import Card

class SurfaceCard(Card):
    """A parsed MCNP surface card.

    Provides read/write access to the surface ID and read-only access to the
    surface type, transform ID, and coefficients.
    """

    @property
    def card_id(self) -> int:
        """The surface ID number."""
        ...

    @card_id.setter
    def card_id(self, new_id: int) -> None:  # type: ignore[override]
        """Set the surface ID number."""
        ...

    transform_id: int | None
    """ID of the coordinate transformation applied to this surface, or ``None``."""

    surface_type: str
    """String representation of the surface type mnemonic (e.g. ``'PZ'``). Read-only."""

    coeffs: list[float]
    """Surface equation coefficients. Read-only."""

    @property
    def original_text(self) -> str:
        """Return the raw card text exactly as it appeared in the source file."""
        ...

    @property
    def updated_text(self) -> str:
        """Return the card text with all in-memory modifications applied."""
        ...
