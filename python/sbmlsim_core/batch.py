"""Bounded consumption of request iterables with persistent batch workers."""

from collections.abc import Iterable, Iterator
from itertools import islice
from typing import Any

from sbmlsim_core._core import BatchExecutor, PointError, SimulationOutput


def simulate_stream(
    executor: BatchExecutor,
    requests: Iterable[dict[str, Any]],
    *,
    batch_size: int = 32,
) -> Iterator[SimulationOutput | PointError]:
    """Yield ordered results, consuming at most batch_size requests ahead.

    The caller owns the executor and closes it after iteration. Malformed requests
    raise before their chunk runs; results from earlier chunks have been yielded.
    Each chunk checks Python signals, and the executor accepts one active call.
    """
    if not isinstance(batch_size, int) or batch_size < 1:
        raise ValueError("batch_size must be a positive integer")
    source = iter(requests)
    while chunk := list(islice(source, batch_size)):
        yield from executor.simulate_many(chunk)
