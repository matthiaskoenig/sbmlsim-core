"""Tests of the basis of a polynomial chaos expansion: `chaos_expansion`,
`chaos_design_matrix` and `chaos_predict`, against the numpy implementation of
`sbmlsim.sensitivity.chaos`, which they replace."""

import itertools

import numpy
import pytest
import sbmlsim_core

# --- the numpy implementation of sbmlsim (src/sbmlsim/sensitivity/chaos.py) ---

MAX_TERMS = 20000


def reference_expansion(
    d: int, degree: int, *, q: float = 1.0, max_interaction: int | None = None
) -> numpy.ndarray:
    """The multi-indices of `sbmlsim.sensitivity.chaos.expansion`."""
    rows: list[numpy.ndarray] = []
    for total in range(degree + 1):
        for combination in itertools.combinations_with_replacement(range(d), total):
            alpha = numpy.bincount(combination, minlength=d)
            inputs = int(numpy.count_nonzero(alpha))
            if max_interaction is not None and inputs > max_interaction:
                continue
            if q < 1.0 and total > 1:
                norm = float(numpy.sum(alpha.astype(float) ** q) ** (1.0 / q))
                if norm > degree * (1.0 + 1e-12):
                    continue
            rows.append(alpha)
            if len(rows) > MAX_TERMS:
                raise ValueError("too many terms")
    return numpy.asarray(rows, dtype=int)


def reference_legendre(u: numpy.ndarray, max_degree: int) -> numpy.ndarray:
    """The values of `sbmlsim.sensitivity.chaos.legendre_values`."""
    points = numpy.asarray(u, dtype=float)
    t = 2.0 * points - 1.0
    out = numpy.empty((*points.shape, max_degree + 1))
    out[..., 0] = 1.0
    if max_degree >= 1:
        out[..., 1] = t
    for k in range(1, max_degree):
        out[..., k + 1] = ((2 * k + 1) * t * out[..., k] - k * out[..., k - 1]) / (
            k + 1
        )
    return out * numpy.sqrt(2.0 * numpy.arange(max_degree + 1) + 1.0)


def reference_design_matrix(
    u: numpy.ndarray, multi_indices: numpy.ndarray
) -> numpy.ndarray:
    """The design matrix of `sbmlsim.sensitivity.chaos.design_matrix`."""
    points = numpy.asarray(u, dtype=float)
    indices = numpy.asarray(multi_indices, dtype=int)
    values = reference_legendre(points, int(indices.max(initial=0)))
    out = numpy.ones((points.shape[0], indices.shape[0]))
    for i in range(points.shape[1]):
        out *= values[:, i, :][:, indices[:, i]]
    return out


# --- the tests ---

TRUNCATIONS = [
    (1, 0, 1.0, None),
    (1, 6, 1.0, None),
    (3, 4, 1.0, None),
    (5, 3, 0.75, None),
    (6, 4, 0.5, None),
    (8, 4, 0.5, 2),
    (22, 4, 0.75, 2),
    (22, 3, 1.0, 1),
]


@pytest.mark.parametrize(("d", "degree", "q", "max_interaction"), TRUNCATIONS)
def test_the_expansion_is_the_one_of_sbmlsim(d, degree, q, max_interaction):
    basis = sbmlsim_core.chaos_expansion(
        d, degree, q=q, max_interaction=max_interaction
    )
    expected = reference_expansion(d, degree, q=q, max_interaction=max_interaction)
    assert basis.dtype == numpy.int64
    numpy.testing.assert_array_equal(basis, expected)


@pytest.mark.parametrize(("d", "degree", "q", "max_interaction"), TRUNCATIONS)
def test_the_design_matrix_is_bitwise_the_one_of_sbmlsim(d, degree, q, max_interaction):
    basis = reference_expansion(d, degree, q=q, max_interaction=max_interaction)
    u = numpy.random.default_rng(d * 10 + degree).random((300, d))
    x = sbmlsim_core.chaos_design_matrix(u, basis)
    expected = reference_design_matrix(u, basis)
    assert x.shape == expected.shape
    assert x.tobytes() == expected.tobytes()


def test_the_design_matrix_takes_any_array_of_the_points():
    basis = sbmlsim_core.chaos_expansion(3, 3)
    u = numpy.random.default_rng(1).random((40, 3))
    expected = sbmlsim_core.chaos_design_matrix(u, basis)
    # a transposed view, a list and float32 multi-indices are read as numpy reads them
    assert (
        sbmlsim_core.chaos_design_matrix(numpy.asfortranarray(u), basis).tobytes()
        == expected.tobytes()
    )
    assert (
        sbmlsim_core.chaos_design_matrix(
            u.tolist(), basis.astype(numpy.int32)
        ).tobytes()
        == expected.tobytes()
    )


def test_the_prediction_is_the_design_matrix_times_the_coefficients():
    basis = sbmlsim_core.chaos_expansion(6, 4, q=0.75, max_interaction=2)
    rng = numpy.random.default_rng(7)
    u = rng.random((500, 6))
    coefficients = rng.normal(size=(len(basis), 5))
    expected = reference_design_matrix(u, basis) @ coefficients
    predicted = sbmlsim_core.chaos_predict(u, basis, coefficients)
    assert predicted.shape == (500, 5)
    numpy.testing.assert_allclose(predicted, expected, rtol=1e-12, atol=1e-12)
    # coefficients of one element give a value per point, as `x @ c` does
    single = sbmlsim_core.chaos_predict(u, basis, coefficients[:, 2])
    assert single.shape == (500,)
    numpy.testing.assert_allclose(single, expected[:, 2], rtol=1e-12, atol=1e-12)


def test_the_threads_do_not_change_a_bit():
    basis = sbmlsim_core.chaos_expansion(10, 3, q=0.75)
    rng = numpy.random.default_rng(3)
    u = rng.random((2000, 10))
    coefficients = rng.normal(size=(len(basis), 4))
    x = sbmlsim_core.chaos_design_matrix(u, basis, threads=1)
    y = sbmlsim_core.chaos_predict(u, basis, coefficients, threads=1)
    for threads in (2, 5, 16):
        assert (
            sbmlsim_core.chaos_design_matrix(u, basis, threads=threads).tobytes()
            == x.tobytes()
        )
        assert (
            sbmlsim_core.chaos_predict(
                u, basis, coefficients, threads=threads
            ).tobytes()
            == y.tobytes()
        )


def test_no_point_is_no_row():
    basis = sbmlsim_core.chaos_expansion(2, 2)
    assert sbmlsim_core.chaos_design_matrix(numpy.empty((0, 2)), basis).shape == (0, 6)
    assert sbmlsim_core.chaos_predict(
        numpy.empty((0, 2)), basis, numpy.ones((6, 3))
    ).shape == (0, 3)


@pytest.mark.parametrize(
    ("arguments", "message"),
    [
        ({"d": 0, "degree": 2}, "at least one input, not 0"),
        ({"d": 2, "degree": -1}, "must not be negative, not -1"),
        ({"d": 2, "degree": 2, "q": 0.0}, r"q must be in \(0, 1\], not 0"),
        ({"d": 2, "degree": 2, "q": 1.5}, r"q must be in \(0, 1\], not 1.5"),
        ({"d": 2, "degree": 2, "max_interaction": 0}, "at least 1, not 0"),
        ({"d": 22, "degree": 5}, "more than 20000 terms"),
    ],
)
def test_a_basis_out_of_range_is_refused(arguments, message):
    with pytest.raises(ValueError, match=message):
        sbmlsim_core.chaos_expansion(**arguments)


def test_shapes_which_do_not_fit_are_refused():
    basis = sbmlsim_core.chaos_expansion(3, 2)
    u = numpy.full((4, 3), 0.5)
    with pytest.raises(ValueError, match="two-dimensional"):
        sbmlsim_core.chaos_design_matrix(u[0], basis)
    with pytest.raises(ValueError, match="two-dimensional"):
        sbmlsim_core.chaos_design_matrix(u, basis[0])
    with pytest.raises(
        ValueError, match="The points have 2 inputs, the multi-indices 3"
    ):
        sbmlsim_core.chaos_design_matrix(u[:, :2], basis)
    with pytest.raises(ValueError, match="must not be negative, not -1"):
        sbmlsim_core.chaos_design_matrix(u, -basis)
    with pytest.raises(ValueError, match="The basis has 10 terms, the coefficients 9"):
        sbmlsim_core.chaos_predict(u, basis, numpy.ones(9))
    with pytest.raises(ValueError, match="one value per term or a row per term"):
        sbmlsim_core.chaos_predict(u, basis, numpy.ones((10, 2, 2)))
    with pytest.raises(ValueError, match="threads"):
        sbmlsim_core.chaos_design_matrix(u, basis, threads=0)
