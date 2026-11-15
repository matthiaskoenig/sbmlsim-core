"""Tests of `LoadedModel.simulate_many`, the batch of requests over rayon threads."""

import _thread
import sys
import threading
import time
from pathlib import Path
from typing import Any

import pytest

import sbmlsim_core

ROOT = Path(__file__).resolve().parents[2]


def request(**fields: Any) -> dict[str, Any]:
    """A request of `simulation/1` over `[0, end]` in `steps` equal intervals.

    `preinit` is the pre-initialization, every other field a setting of the integrator.
    """
    steps = fields.pop("steps")
    end = fields.pop("end")
    preinit = fields.pop("preinit", [])
    settings = {
        "relative_tolerance": 1e-6,
        "absolute_tolerance": {"scalar": 1e-12},
        "maximum_num_steps": 20000,
        "maximum_time_step": 0.0,
        "initial_time_step": 0.0,
        "stiff": True,
        "linear_solver": "auto",
        "jacobian": "auto",
    } | fields
    return {
        "contract": sbmlsim_core.SIMULATION_CONTRACT,
        "start": 0.0,
        "end": end,
        "output": {"uniform": {"steps": steps}},
        "preinit": preinit,
        "changes": [],
        "steady_state": None,
        "selections": [],
        "settings": settings,
    }


def slow_request() -> dict[str, Any]:
    """A request of about 80 ms with 11 rows: a long time in small steps of the integrator.

    A slice of such requests takes long enough to be told apart from no work at all,
    without the memory of a large table.
    """
    return request(
        end=1e6, steps=10, maximum_time_step=10.0, maximum_num_steps=100_000_000
    )


@pytest.fixture(scope="module")
def decay() -> sbmlsim_core.LoadedModel:
    """The decay model of the host tests: y' = -k y, y(0) = 1, k = 0.5."""
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    compiled = sbmlsim_core.compile_sbml(sbml)
    return sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)


def test_a_batch_gives_one_result_per_request_in_order(decay):
    requests = [
        request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(1, 9)
    ]
    results = decay.simulate_many(requests, threads=4)
    assert len(results) == len(requests)
    for n, out in enumerate(results, start=1):
        assert out.values[0, out.columns.index("y")] == float(n)


def test_a_batch_does_not_depend_on_the_number_of_threads(decay):
    requests = [request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(64)]
    one = decay.simulate_many(requests, threads=1)
    many = decay.simulate_many(requests, threads=8)
    serial = [decay.instance().simulate(r) for r in requests]
    for a, b, c in zip(one, many, serial, strict=True):
        assert a.values.tobytes() == b.values.tobytes() == c.values.tobytes()


def test_a_batch_does_not_depend_on_how_it_is_sliced(decay):
    # the batch runs in slices of 8 * threads requests, so 16 requests on two threads are one
    # slice and 48 are three. The last 16 are the comparison, not the first: as the third
    # slice of the long batch each of them runs on an instance which has simulated 16 points
    # before it, as a batch of their own on a fresh instance, and both give the same bytes.
    requests = [request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(48)]
    three_slices = decay.simulate_many(requests, threads=2)
    one_slice = decay.simulate_many(requests[32:], threads=2)
    assert len(three_slices) == 48 and len(one_slice) == 16
    for a, b in zip(one_slice, three_slices[32:], strict=True):
        assert a.values.tobytes() == b.values.tobytes()


def test_a_batch_without_threads_gives_the_results_of_one_thread(decay):
    requests = [request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(32)]
    every = decay.simulate_many(requests)
    one = decay.simulate_many(requests, threads=1)
    for a, b in zip(every, one, strict=True):
        assert a.values.tobytes() == b.values.tobytes()


def test_a_batch_without_requests_is_empty(decay):
    assert decay.simulate_many([], threads=2) == []


def test_a_failed_point_is_an_error_and_its_neighbours_still_run(decay):
    good = request(end=10.0, steps=10)
    bad = request(end=1e9, steps=10, maximum_num_steps=1)
    results = decay.simulate_many([good, bad, good], threads=2)
    assert isinstance(results[1], sbmlsim_core.PointError)
    assert results[1].kind == "integration"
    assert results[1].flag == -1
    assert results[1].time > 0.0
    assert "mxstep" in results[1].message
    assert results[0].values.shape == results[2].values.shape == (11, 4)


def test_a_failed_point_leaves_the_instance_of_its_thread_usable(decay):
    # one thread runs every point of the batch on one instance, and the points after the one
    # which failed are the bytes of a fresh instance
    good = request(end=10.0, steps=10)
    bad = request(end=1e9, steps=10, maximum_num_steps=1)
    fresh = decay.instance().simulate(good)
    results = decay.simulate_many([good, bad, good, good], threads=1)
    assert isinstance(results[1], sbmlsim_core.PointError)
    for out in (results[0], results[2], results[3]):
        assert out.values.tobytes() == fresh.values.tobytes()


def test_a_point_the_engine_refuses_is_a_request_error(decay):
    bad = request(end=10.0, steps=10) | {"selections": ["nope"]}
    (point,) = decay.simulate_many([bad], threads=1)
    assert isinstance(point, sbmlsim_core.PointError)
    assert point.kind == "request"
    assert point.flag is None and point.time is None
    assert "nope" in point.message
    assert "request" in repr(point)


def test_a_request_which_is_none_is_refused_before_anything_runs(decay):
    requests = [request(end=10.0, steps=10), request(end="ten", steps=10)]
    with pytest.raises(ValueError, match=r"requests\[1\]"):
        decay.simulate_many(requests, threads=2)


def test_threads_below_one_is_refused(decay):
    with pytest.raises(ValueError, match="threads"):
        decay.simulate_many([request(end=1.0, steps=1)], threads=0)
    # the threads are checked before the batch looks at its requests, so a batch which has
    # nothing to run is refused as well
    with pytest.raises(ValueError, match="threads"):
        decay.simulate_many([], threads=0)


def test_a_long_batch_is_interrupted(decay):
    # The interrupt has to reach the batch while it runs, which only the check between two
    # slices sees: an interrupt which is pending before the call is raised by the interpreter
    # at the call itself (CPython 3.14), and one which arrives after the batch returned says
    # nothing about the batch. The switch interval keeps the main thread from handing the GIL
    # to the timer in the eval loop, so the timer cannot run before the batch releases it for
    # its pool, and the batch is 160 requests, ten slices of 16 and about 6 s of work, so it
    # cannot have finished before the timer either.
    #
    # The interrupt is caught here rather than with `pytest.raises`, because the elapsed time
    # is the whole claim of the test and `pytest.raises` would not reach it: a batch which
    # never checks the signals returns, and the interrupt which is pending then is raised
    # inside `__exit__`, where it ends the session instead of failing the test.
    requests = [slow_request() for _ in range(160)]
    interrupt = threading.Timer(0.02, _thread.interrupt_main)
    switch = sys.getswitchinterval()
    sys.setswitchinterval(1000.0)
    started = time.perf_counter()
    try:
        interrupt.start()
        decay.simulate_many(requests, threads=2)
        # the batch ran to its end, so the interrupt is pending now: a backward jump of the
        # interpreter raises it, which happens here and not in the assertions below
        for _ in range(1000):
            pass
        interrupted = False
    except KeyboardInterrupt:
        interrupted = True
    finally:
        interrupt.cancel()
        sys.setswitchinterval(switch)
    elapsed = time.perf_counter() - started
    assert interrupted, "the batch was never interrupted"
    # a slice of 16 of these requests on two threads takes about 0.6 s and the whole batch
    # about 6 s, so the elapsed time says the batch ran a slice and not all ten: the
    # interrupt is the one of the check between two slices
    assert 0.15 < elapsed < 2.0
