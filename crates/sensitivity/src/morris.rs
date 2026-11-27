//! Morris: the trajectories of its design and their elementary effects.

use crate::bootstrap::{Bootstrap, counts, weighted_sums};
use crate::random::Stream;
use crate::{
    Columns, Matrix, MatrixRef, Result, SensitivityError, by_columns, check_parameters, interval,
};

/// The step of the grid of `levels` levels on `[0, 1]`, `levels / (2 (levels - 1))`, the jump
/// of a parameter in a trajectory of SALib (`SALib.util._compute_delta`).
#[must_use]
pub fn morris_delta(levels: usize) -> f64 {
    levels as f64 / (2.0 * (levels as f64 - 1.0))
}

/// Check the levels of a Morris design.
fn check_levels(levels: usize) -> Result<()> {
    if levels < 2 {
        return Err(SensitivityError::new(format!(
            "The levels of a Morris design are at least 2, not {levels}."
        )));
    }
    Ok(())
}

/// The design of Morris of `d` parameters: `trajectories` trajectories of `d + 1` points on
/// the grid of `levels` levels of `[0, 1]`, a row per point.
///
/// A trajectory of `SALib.sample.morris.sample` (Morris 1991): its start `x*` takes every
/// parameter from the lower levels `linspace(0, 1 - delta, floor(levels / 2))`, and every step
/// moves one parameter, in a random order, by `delta = levels / (2 (levels - 1))`, up from
/// `x*` or down to it in a random direction, so every point is `x*` or `x* + delta` in every
/// parameter. The draws of a trajectory, the order, the directions and the start, come from the
/// stream of `seed`, a trajectory after the other.
///
/// # Errors
///
/// If `d` or `trajectories` is zero or `levels` is below 2.
pub fn morris_sample(d: usize, trajectories: usize, levels: usize, seed: u64) -> Result<Matrix> {
    check_parameters(d)?;
    check_levels(levels)?;
    if trajectories == 0 {
        return Err(SensitivityError::new(
            "A Morris design needs at least one trajectory.",
        ));
    }
    let delta = morris_delta(levels);
    // the lower levels a trajectory starts from, `numpy.linspace(0, 1 - delta, levels // 2)`
    let count = levels / 2;
    let bound = 1.0 - delta;
    let grid: Vec<f64> = (0..count)
        .map(|k| {
            if k + 1 == count {
                if count == 1 { 0.0 } else { bound }
            } else {
                k as f64 * (bound / (count - 1) as f64)
            }
        })
        .collect();
    let mut stream = Stream::new(seed);
    let size = d + 1;
    let mut points = Matrix::zeros(trajectories * size, d);
    let mut order: Vec<usize> = Vec::with_capacity(d);
    for t in 0..trajectories {
        // the step which moves every parameter, its direction and its start
        order.clear();
        order.extend(0..d);
        stream.shuffle(&mut order);
        let up: Vec<bool> = (0..d).map(|_| stream.bits(1) == 1).collect();
        let start: Vec<f64> = (0..d).map(|_| grid[stream.below(count)]).collect();
        for p in 0..size {
            let row = &mut points.data[(t * size + p) * d..(t * size + p + 1) * d];
            for j in 0..d {
                let moved = order[j] < p;
                row[j] = if moved == up[j] {
                    start[j] + delta
                } else {
                    start[j]
                };
            }
        }
    }
    Ok(points)
}

/// The elementary effects of the parameters, every column of the values estimated at once.
#[derive(Debug, Clone, PartialEq)]
pub struct MorrisEffects {
    /// The mean of the effects `mu`, a row per parameter and a column per element.
    pub mu: Matrix,
    /// The mean of their magnitudes `mu_star`.
    pub mu_star: Matrix,
    /// Their standard deviation `sigma`, with one degree of freedom less; not a number for a
    /// single trajectory.
    pub sigma: Matrix,
    /// The interval of `mu_star`.
    pub mu_star_conf: Matrix,
}

impl Columns for MorrisEffects {
    fn join(parts: Vec<Self>) -> Self {
        let join = |field: fn(&Self) -> &Matrix| {
            Matrix::join(&parts.iter().map(field).collect::<Vec<_>>())
        };
        Self {
            mu: join(|p| &p.mu),
            mu_star: join(|p| &p.mu_star),
            sigma: join(|p| &p.sigma),
            mu_star_conf: join(|p| &p.mu_star_conf),
        }
    }
}

/// The elementary effects of the values of the trajectories of a Morris design
/// ([`morris_sample`]).
///
/// One step of a trajectory moves one parameter, so the effect of that parameter is the change
/// of the output over the step divided by `delta = levels / (2 (levels - 1))`, the step of the
/// grid of SALib on `[0, 1]`. Which parameter a step moves and in which direction follows from
/// the points of the cube alone (a centred grid gives the same effects, only the signs of the
/// steps are read) and is the same for every element. `mu` and `mu_star` are the mean of the
/// effects and of their magnitudes over the trajectories (Campolongo et al. 2007), `sigma` is
/// their standard deviation, and `mu_star_conf` is `z(conf_level)` times the standard deviation
/// of `mu_star` over resamples of the trajectories, drawn per parameter; given resamples are
/// row-major `(d, resamples, trajectories)`, the trajectories each resample draws.
///
/// The elements are spread over at most `threads` threads, and an effect does not depend on
/// them.
///
/// # Errors
///
/// If the points are no trajectories of `d + 1` points of the `d` parameters of the cube, the
/// values have another number of rows, a step of a trajectory moves other than one parameter
/// or a parameter moves in other than one step, `levels` is below 2 or the bootstrap is not
/// valid.
pub fn morris_effects(
    cube: MatrixRef<'_>,
    y: MatrixRef<'_>,
    levels: usize,
    bootstrap: &Bootstrap<'_>,
    threads: usize,
) -> Result<MorrisEffects> {
    by_columns(y, threads, |part| {
        morris_part(cube, part, levels, bootstrap)
    })
}

/// The elementary effects of the elements of `y` on one thread, see [`morris_effects`].
fn morris_part(
    cube: MatrixRef<'_>,
    y: MatrixRef<'_>,
    levels: usize,
    bootstrap: &Bootstrap<'_>,
) -> Result<MorrisEffects> {
    let (rows, d) = (cube.rows(), cube.columns());
    check_parameters(d)?;
    check_levels(levels)?;
    let size = d + 1;
    if rows == 0 || !rows.is_multiple_of(size) || y.rows() != rows {
        return Err(SensitivityError::new(format!(
            "The design of {d} parameters has trajectories of {size} points, which the {rows} \
             points of the cube and the {} rows of the values must be a multiple of.",
            y.rows()
        )));
    }
    let z = bootstrap.z()?;
    let trajectories = rows / size;
    let elements = y.columns();
    let delta = morris_delta(levels);
    // the effects, a row per parameter and trajectory
    let mut effects = vec![0.0; d * trajectories * elements];
    for t in 0..trajectories {
        let mut moved = vec![false; d];
        for p in 0..d {
            let (before, after) = (cube.row(t * size + p), cube.row(t * size + p + 1));
            let steps: Vec<usize> = (0..d).filter(|&j| after[j] != before[j]).collect();
            let [j] = steps[..] else {
                return Err(SensitivityError::new(format!(
                    "The step {p} of the trajectory {t} moves {} parameters, a step of a \
                     trajectory of Morris moves one.",
                    steps.len()
                )));
            };
            if moved[j] {
                return Err(SensitivityError::new(format!(
                    "The trajectory {t} moves the parameter {j} twice, a trajectory of Morris \
                     moves every parameter once."
                )));
            }
            moved[j] = true;
            let (low, high) = if after[j] > before[j] {
                (y.row(t * size + p), y.row(t * size + p + 1))
            } else {
                (y.row(t * size + p + 1), y.row(t * size + p))
            };
            let effect = &mut effects[(j * trajectories + t) * elements..][..elements];
            for e in 0..elements {
                effect[e] = (high[e] - low[e]) / delta;
            }
        }
    }
    let magnitudes: Vec<f64> = effects.iter().map(|e| e.abs()).collect();
    let (indices, resamples) = bootstrap.indices(
        d,
        trajectories,
        trajectories,
        "(d, resamples, trajectories) of the drawn trajectories",
    )?;
    let mut found = MorrisEffects {
        mu: Matrix::zeros(d, elements),
        mu_star: Matrix::zeros(d, elements),
        sigma: Matrix::zeros(d, elements),
        mu_star_conf: Matrix::zeros(d, elements),
    };
    let count = trajectories as f64;
    for j in 0..d {
        // the effects of the parameter in the trajectory t
        let row =
            |t: usize| (j * trajectories + t) * elements..(j * trajectories + t + 1) * elements;
        let out = j * elements..(j + 1) * elements;
        let (mu, mu_star) = (
            &mut found.mu.data[out.clone()],
            &mut found.mu_star.data[out.clone()],
        );
        for t in 0..trajectories {
            crate::add_row(mu, &effects[row(t)]);
            crate::add_row(mu_star, &magnitudes[row(t)]);
        }
        for value in mu.iter_mut().chain(mu_star.iter_mut()) {
            *value /= count;
        }
        let sigma = &mut found.sigma.data[out.clone()];
        for t in 0..trajectories {
            for ((s, v), m) in sigma
                .iter_mut()
                .zip(&effects[row(t)])
                .zip(&found.mu.data[out.clone()])
            {
                *s += (v - m) * (v - m);
            }
        }
        for s in sigma.iter_mut() {
            *s = (*s / (count - 1.0)).sqrt();
        }
        let drawn = &indices[j * resamples * trajectories..(j + 1) * resamples * trajectories];
        let weights = counts(drawn, trajectories, trajectories);
        let mut resampled =
            weighted_sums(&weights, trajectories, elements, |t| &magnitudes[row(t)]);
        for value in &mut resampled {
            *value /= count;
        }
        found.mu_star_conf.data[out].copy_from_slice(&interval(&resampled, resamples, elements, z));
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Resamples;

    #[test]
    fn a_trajectory_moves_every_parameter_once_on_the_grid() {
        for (levels, starts) in [
            (4, vec![0.0, 1.0 - morris_delta(4)]),
            (6, vec![0.0, 0.2, 0.4]),
            (2, vec![0.0]),
            (5, vec![0.0, 0.375]),
        ] {
            let d = 4;
            let delta = morris_delta(levels);
            let points = morris_sample(d, 50, levels, 3).unwrap();
            assert_eq!((points.rows, points.columns), (250, d));
            for t in 0..50 {
                let first = points.row(t * (d + 1));
                let mut moved = vec![false; d];
                for p in 0..d {
                    let (before, after) =
                        (points.row(t * (d + 1) + p), points.row(t * (d + 1) + p + 1));
                    let steps: Vec<usize> = (0..d).filter(|&j| before[j] != after[j]).collect();
                    assert_eq!(steps.len(), 1);
                    let j = steps[0];
                    assert!(!moved[j]);
                    moved[j] = true;
                    assert!(((after[j] - before[j]).abs() - delta).abs() < 1e-15);
                }
                for (j, &value) in first.iter().enumerate() {
                    let low = (0..=d)
                        .map(|p| points.get(t * (d + 1) + p, j))
                        .fold(f64::INFINITY, f64::min);
                    assert!(starts.contains(&low), "{levels} {low}");
                    assert!(value == low || value == low + delta);
                }
            }
            // every start level and both directions are drawn
            let lows: Vec<f64> = (0..50).map(|t| points.get(t * (d + 1), 0)).collect();
            for start in &starts {
                assert!(lows.iter().any(|x| x == start || *x == start + delta));
            }
        }
    }

    #[test]
    fn the_effects_of_a_linear_function_are_its_slopes() {
        let d = 3;
        let points = morris_sample(d, 20, 4, 1).unwrap();
        let slopes = [2.0, -1.0, 0.5];
        let y: Vec<f64> = (0..points.rows)
            .flat_map(|i| {
                let x = points.row(i);
                let value: f64 = x.iter().zip(&slopes).map(|(a, b)| a * b).sum();
                [value, 1.0]
            })
            .collect();
        let found = morris_effects(
            points.view(),
            MatrixRef::new(&y, points.rows, 2).unwrap(),
            4,
            &Bootstrap {
                conf_level: 0.95,
                resamples: Resamples::Seeded { count: 50, seed: 2 },
            },
            1,
        )
        .unwrap();
        for (j, slope) in slopes.iter().enumerate() {
            assert!((found.mu.get(j, 0) - slope).abs() < 1e-12);
            assert!((found.mu_star.get(j, 0) - slope.abs()).abs() < 1e-12);
            assert!(found.sigma.get(j, 0) < 1e-12);
            assert!(found.mu_star_conf.get(j, 0) < 1e-12);
            assert_eq!(found.mu.get(j, 1), 0.0);
        }
    }

    #[test]
    fn the_points_of_a_design_are_checked() {
        let cube = vec![0.0; 10 * 3];
        let y = vec![0.0; 10];
        let bootstrap = Bootstrap {
            conf_level: 0.95,
            resamples: Resamples::Seeded { count: 2, seed: 1 },
        };
        let error = morris_effects(
            MatrixRef::new(&cube, 10, 3).unwrap(),
            MatrixRef::new(&y, 10, 1).unwrap(),
            4,
            &bootstrap,
            1,
        )
        .unwrap_err();
        assert!(error.0.contains("trajectories of 4 points"));
        let cube = vec![0.0; 8 * 3];
        let y = vec![0.0; 8];
        let error = morris_effects(
            MatrixRef::new(&cube, 8, 3).unwrap(),
            MatrixRef::new(&y, 8, 1).unwrap(),
            4,
            &bootstrap,
            1,
        )
        .unwrap_err();
        assert!(error.0.contains("moves 0 parameters"));
        assert!(morris_sample(2, 0, 4, 1).is_err());
        assert!(morris_sample(2, 3, 1, 1).is_err());
    }
}
