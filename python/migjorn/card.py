"""Protocol defining the common interface shared by all card types."""

from typing import Protocol


class Card(Protocol):
    """Common interface implemented by all MCNP card types."""

    def original_text(self) -> str: ...
    def updated_text(self) -> str: ...

    @property
    def card_id(self) -> int | str: ...

    @card_id.setter
    def card_id(self, new_id: int | str) -> None: ...
