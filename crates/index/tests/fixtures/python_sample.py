# SPDX-License-Identifier: Apache-2.0
"""Fixture for the extraction tests: an order-processing module."""

import os
import sys as system
from . import sibling
from ..pkg.models import (Order, Customer as Client)
from typing import Optional, List
import a.b.c

MAX_RETRIES = 3
DEFAULT_CURRENCY: str = "EUR"
registry = {}


class Repository:
    """Base class of every repository."""

    def find(self, key):
        raise NotImplementedError


class OrderRepository(Repository, metaclass=Meta):
    """Stores orders.

    Orders are kept in memory.
    """

    table = "orders"
    LIMIT = 100

    def __init__(self, path: str, retries: int = MAX_RETRIES) -> None:
        self.path = os.path.join(path, "orders")
        super().__init__()

    @property
    def size(self) -> int:
        return len(self._rows)

    @staticmethod
    def build(path: str) -> "OrderRepository":
        return OrderRepository(path)

    def find(self, key: str) -> Optional[Order]:
        """Finds an order."""
        return self._lookup(key)

    def _lookup(self, key):
        return helpers.lookup(self, key)

    def __repr__(self):
        return "OrderRepository()"

    class Cursor:
        def advance(self): pass


@decorator(3)
@other.tag
async def fetch_all(repo: OrderRepository, limit: int = 10) -> List[Order]:
    """Fetches all orders.

    Uses the repository.
    """
    rows = await repo.load(limit)

    def inner(row):
        return normalize(row)

    return [inner(r) for r in rows]


def _private_helper():
    return sorted([1, 2, 3])


def main():
    repo = OrderRepository("/tmp")
    print(fetch_all(repo))


if __name__ == "__main__":
    main()
