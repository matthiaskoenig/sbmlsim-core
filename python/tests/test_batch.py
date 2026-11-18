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

    `output` is the times of the output (`{"uniform": {"steps": n}}` from `steps`, or
    `"integrator_steps"`, a bare string since it is a unit variant), `preinit` is the
    pre-initialization, every other field a setting of the integrator.
    """
    steps = fields.pop("steps", None)
    end = fields.pop("end")
    output = fields.pop("output", {"uniform": {"steps": steps}})
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
        "output": output,
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
    requests = [
        request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(64)
    ]
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
    requests = [
        request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(48)
    ]
    three_slices = decay.simulate_many(requests, threads=2)
    one_slice = decay.simulate_many(requests[32:], threads=2)
    assert len(three_slices) == 48 and len(one_slice) == 16
    for a, b in zip(one_slice, three_slices[32:], strict=True):
        assert a.values.tobytes() == b.values.tobytes()


def test_a_batch_without_threads_gives_the_results_of_one_thread(decay):
    requests = [
        request(end=10.0, steps=10, preinit=[("y", 1.0 * n)]) for n in range(32)
    ]
    every = decay.simulate_many(requests)
    one = decay.simulate_many(requests, threads=1)
    for a, b in zip(every, one, strict=True):
        assert a.values.tobytes() == b.values.tobytes()


def test_a_batch_over_the_steps_of_the_integrator_is_ragged(decay):
    # the steps of the integrator depend on the point, so the tables differ in their rows
    requests = [
        request(end=end, output="integrator_steps") for end in (1.0, 10.0, 100.0)
    ]
    results = decay.simulate_many(requests, threads=2)
    rows = [out.values.shape[0] for out in results]
    assert len(set(rows)) == len(rows), rows
    for out, end in zip(results, (1.0, 10.0, 100.0), strict=True):
        assert out.values[0, 0] == 0.0
        assert out.values[-1, 0] == end
    serial = decay.simulate_many(requests, threads=1)
    for a, b in zip(results, serial, strict=True):
        assert a.values.shape == b.values.shape
        assert (a.values == b.values).all()


def test_a_per_state_absolute_tolerance_is_one_value_per_state(decay):
    n_states = 1  # y of the decay model
    scalar = decay.simulate_many([request(end=10.0, steps=10)])[0]
    per_state = decay.simulate_many(
        [
            request(
                end=10.0, steps=10, absolute_tolerance={"per_state": [1e-12] * n_states}
            )
        ]
    )[0]
    assert (scalar.values == per_state.values).all()
    wrong = decay.simulate_many(
        [request(end=10.0, steps=10, absolute_tolerance={"per_state": [1e-12] * 3})]
    )[0]
    assert isinstance(wrong, sbmlsim_core.PointError)


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


@pytest.mark.parametrize("persistent", [False, True])
def test_a_long_batch_is_interrupted(decay, persistent):
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
    # Calibrate one slice on this runner. An absolute threshold is unreliable when
    # other test processes compete for CPU; a batch without signal checks runs ten slices.
    calibrated = time.perf_counter()
    decay.simulate_many(requests[:16], threads=2)
    slice_seconds = time.perf_counter() - calibrated
    interrupt = threading.Timer(0.02, _thread.interrupt_main)
    switch = sys.getswitchinterval()
    sys.setswitchinterval(1000.0)
    started = time.perf_counter()
    try:
        interrupt.start()
        if persistent:
            with decay.batch_executor(threads=2) as executor:
                executor.simulate_many(requests)
        else:
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
    # Leave a five-slice margin for scheduling variation while still rejecting the
    # ten-slice mutation that only delivers the interrupt after returning to Python.
    assert elapsed < 5 * slice_seconds


def test_executor_repeated_calls_and_failures_recover(decay):
    good = request(end=10.0, steps=10)
    fresh = decay.instance().simulate(good).values.tobytes()
    bad = request(end=1e9, steps=10, maximum_num_steps=1)
    with decay.batch_executor(threads=1) as executor:
        assert executor.simulate_many([]) == []
        for _ in range(3):
            outputs = executor.simulate_many([good, bad, good])
            assert isinstance(outputs[1], sbmlsim_core.PointError)
            assert outputs[0].values.tobytes() == fresh
            assert outputs[2].values.tobytes() == fresh
        with pytest.raises(ValueError, match=r"requests\[1\]"):
            executor.simulate_many([good, {"invalid": True}])
        assert executor.simulate_many([good])[0].values.tobytes() == fresh
    executor.close()
    with pytest.raises(RuntimeError, match="closed"):
        executor.simulate_many([])
    with pytest.raises(RuntimeError, match="closed"):
        executor.__enter__()


def test_executor_growth_and_parallel_parity(decay):
    requests = [
        request(end=10.0, steps=10, preinit=[("y", float(n))]) for n in range(1, 10)
    ]
    expected = decay.simulate_many(requests, threads=1)
    with decay.batch_executor(threads=4) as executor:
        executor.simulate_many(requests[:1])
        for _ in range(3):
            actual = executor.simulate_many(requests)
            assert [out.values.tobytes() for out in actual] == [
                out.values.tobytes() for out in expected
            ]


@pytest.mark.parametrize("prepared_mode", [False, True])
def test_executor_rejects_concurrent_and_reentrant_calls(decay, prepared_mode):
    entered, release = threading.Event(), threading.Event()
    errors = []
    prepared = decay.prepare(request(end=1.0, steps=1))
    with decay.batch_executor(threads=1) as executor:

        def requests():
            entered.set()
            assert release.wait(5)
            yield [] if prepared_mode else request(end=1.0, steps=1)

        def run():
            try:
                if prepared_mode:
                    executor.simulate_prepared(prepared, requests())
                else:
                    executor.simulate_many(requests())
            except BaseException as error:
                errors.append(error)

        worker = threading.Thread(target=run)
        worker.start()
        try:
            assert entered.wait(5)
            with pytest.raises(RuntimeError, match="busy"):
                executor.simulate_many([])
            with pytest.raises(RuntimeError, match="busy"):
                executor.simulate_prepared(prepared, [])
            with pytest.raises(RuntimeError, match="busy"):
                executor.close()
        finally:
            release.set()
            worker.join(5)
        assert not worker.is_alive() and not errors

        def reentrant():
            with pytest.raises(RuntimeError, match="busy"):
                executor.simulate_many([])
            yield request(end=1.0, steps=1)

        assert len(executor.simulate_many(reentrant())) == 1


def test_stream_consumes_bounded_chunks(decay):
    consumed = []

    def requests():
        for n in range(11):
            consumed.append(n)
            yield request(end=1.0, steps=1)

    with decay.batch_executor(threads=2) as executor:
        results = sbmlsim_core.simulate_stream(executor, requests(), batch_size=3)
        next(results)
        assert len(consumed) == 3
        assert len(list(results)) == 10
        with pytest.raises(ValueError, match="batch_size"):
            list(sbmlsim_core.simulate_stream(executor, [], batch_size=0))


def test_executor_context_closes_on_error(decay):
    with pytest.raises(LookupError), decay.batch_executor() as executor:
        raise LookupError("stop")
    with pytest.raises(RuntimeError, match="closed"):
        executor.simulate_many([])
    for threads in (0, -1, 1025):
        with pytest.raises(ValueError, match="threads"):
            decay.batch_executor(threads=threads)


def test_prepared_numeric_rows_match_requests_and_reuse_workers(decay):
    import numpy as np

    base = request(end=10.0, steps=10, preinit=[("y", 2.0)])
    prepared = decay.prepare(base, varying=["k", "y"])
    assert prepared.n_values == 2
    rows = np.array([[0.2, 3.0], [0.5, 1.0], [0.8, 4.0]])
    expected = [
        decay.instance().simulate(
            base | {"preinit": [("k", float(k)), ("y", float(y))]}
        )
        for k, y in rows
    ]
    with decay.batch_executor(threads=3) as executor:
        executor.simulate_prepared(prepared, rows[:1])
        for _ in range(2):
            results = executor.simulate_prepared(prepared, rows)
            for result, reference in zip(results, expected, strict=True):
                assert result.columns == reference.columns
                assert result.values.tobytes() == reference.values.tobytes()
        assert executor.simulate_many([base])[0].values.tobytes() == (
            decay.instance().simulate(base).values.tobytes()
        )
    with pytest.raises(RuntimeError, match="closed"):
        executor.simulate_prepared(prepared, [])


def test_prepared_points_isolate_invalid_values(decay):
    prepared = decay.prepare(request(end=1.0, steps=1), varying=["k"])
    with decay.batch_executor(threads=1) as executor:
        results = executor.simulate_prepared(
            prepared, [[0.2], [], [float("nan")], [float("inf")], [0.2]]
        )
        assert results[0].values.tobytes() == results[-1].values.tobytes()
        assert all(
            isinstance(result, sbmlsim_core.PointError) for result in results[1:4]
        )
        with pytest.raises(ValueError, match=r"points\[1\]"):
            executor.simulate_prepared(prepared, [[0.2], ["invalid"]])
        assert isinstance(
            executor.simulate_prepared(prepared, [[0.2]])[0],
            sbmlsim_core.SimulationOutput,
        )
    with pytest.raises(sbmlsim_core.RequestError):
        decay.instance().simulate_prepared(prepared, [])


def test_prepared_default_values_and_model_identity(decay):
    base = request(end=1.0, steps=1)
    prepared = decay.prepare(base)
    assert prepared.n_values == 0
    assert decay.instance().simulate_prepared(prepared).values.tobytes() == (
        decay.instance().simulate(base).values.tobytes()
    )
    compiled = sbmlsim_core.compile_sbml(
        (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    )
    other = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)
    with pytest.raises(sbmlsim_core.RequestError):
        other.instance().simulate_prepared(prepared)
    with (
        other.batch_executor(threads=1) as executor,
        pytest.raises(ValueError, match="another model"),
    ):
        executor.simulate_prepared(prepared, [[]])
    with pytest.raises(ValueError):
        decay.prepare(base, varying=["missing"])
    with pytest.raises(ValueError):
        decay.prepare(base, varying=["k", "k"])


def test_prepared_executor_reentrant_submissions_are_busy(decay):
    prepared = decay.prepare(request(end=1.0, steps=1), varying=["k"])
    with decay.batch_executor(threads=1) as executor:

        def rows():
            with pytest.raises(RuntimeError, match="busy"):
                executor.simulate_many([])
            with pytest.raises(RuntimeError, match="busy"):
                executor.simulate_prepared(prepared, [])
            with pytest.raises(RuntimeError, match="busy"):
                executor.close()
            yield [0.5]

        assert len(executor.simulate_prepared(prepared, rows())) == 1


@pytest.mark.parametrize("steady", [False, True])
def test_prepared_fixed_changes_and_steady_state_match_requests(decay, steady):
    base = request(end=10.0, steps=10)
    base["changes"] = [{"time": 5.0, "values": [("y", 2.0)]}]
    if steady:
        base["steady_state"] = {
            "preinit": [],
            "absolute_tolerance": 1e-8,
            "relative_tolerance": 1e-6,
            "max_time": 1e4,
        }
    prepared = decay.prepare(base, varying=["k"])
    with decay.batch_executor(threads=2) as executor:
        outputs = executor.simulate_prepared(prepared, [[0.2], [0.8]])
    for k, actual in zip([0.2, 0.8], outputs, strict=True):
        expected = decay.instance().simulate(base | {"preinit": [("k", k)]})
        assert actual.values.tobytes() == expected.values.tobytes()


def test_prepared_concentration_values_follow_preinit_semantics():
    compiled = sbmlsim_core.compile_sbml("""
    <sbml xmlns="http://www.sbml.org/sbml/level3/version2/core" level="3" version="2">
      <model id="concentration">
        <listOfCompartments><compartment id="C" size="2" constant="true"/></listOfCompartments>
        <listOfSpecies><species id="S" compartment="C" initialConcentration="1"
          hasOnlySubstanceUnits="false" boundaryCondition="false" constant="false"/></listOfSpecies>
      </model>
    </sbml>""")
    model = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)
    base = request(end=1.0, steps=1) | {"selections": ["S", "[S]", "C"]}
    prepared = model.prepare(base, varying=["C", "[S]"])
    rows = [[3.0, 4.0], [5.0, 2.0]]
    with model.batch_executor(threads=2) as executor:
        outputs = executor.simulate_prepared(prepared, rows)
    for (volume, concentration), actual in zip(rows, outputs, strict=True):
        expected = model.instance().simulate(
            base | {"preinit": [("C", volume), ("[S]", concentration)]}
        )
        assert actual.values.tobytes() == expected.values.tobytes()


def test_plans_run_their_own_requests_on_the_same_workers(decay):
    short = decay.prepare(request(end=2.0, steps=2), varying=["k"])
    long = decay.prepare(request(end=8.0, steps=8), varying=["k"])
    expected = [
        decay.instance().simulate(request(end=2.0, steps=2, preinit=[("k", 1.0)])),
        decay.instance().simulate(request(end=8.0, steps=8, preinit=[("k", 0.5)])),
        decay.instance().simulate(request(end=2.0, steps=2, preinit=[("k", 0.2)])),
    ]
    with decay.batch_executor(threads=2) as executor:
        results = executor.simulate_plans(
            [(short, [1.0]), (long, [0.5]), (short, [0.2])]
        )
    for result, reference in zip(results, expected, strict=True):
        assert result.columns == reference.columns
        assert result.values.tobytes() == reference.values.tobytes()


def test_plans_refuse_a_plan_which_is_none_before_simulating(decay):
    prepared = decay.prepare(request(end=1.0, steps=1), varying=["k"])
    with decay.batch_executor(threads=1) as executor:
        assert executor.simulate_plans([]) == []
        with pytest.raises(ValueError, match=r"plans\[1\]"):
            executor.simulate_plans([(prepared, [0.2]), (prepared, ["invalid"])])
        with pytest.raises(ValueError, match=r"plans\[0\]"):
            executor.simulate_plans([prepared])
        results = executor.simulate_plans([(prepared, [0.2]), (prepared, [])])
        assert isinstance(results[0], sbmlsim_core.SimulationOutput)
        assert isinstance(results[1], sbmlsim_core.PointError)


def test_plans_refuse_the_prepared_request_of_another_model(decay):
    sbml = (ROOT / "crates/host/tests/decay.xml").read_text(encoding="utf-8")
    compiled = sbmlsim_core.compile_sbml(sbml)
    other = sbmlsim_core.Runtime().load(compiled.wasm, compiled.description)
    foreign = other.prepare(request(end=1.0, steps=1), varying=["k"])
    with (
        decay.batch_executor(threads=1) as executor,
        pytest.raises(ValueError, match=r"plans\[0\].*another model"),
    ):
        executor.simulate_plans([(foreign, [0.2])])
