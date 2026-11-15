//! The order of evaluations which read each other: a topological order, ties by position.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// The positions `0..deps.len()` in an order in which every position comes after the positions
/// it depends on (`deps[i]`), and of two positions which may come next the smaller one first, so
/// independent evaluations keep their given (document) order.
///
/// A position which depends on itself, directly or through others, is a cycle: the error is the
/// positions of one cycle, each depending on the next and the last on the first, the one reached
/// by following from the smallest position left its smallest dependency left.
pub(crate) fn order(deps: &[Vec<usize>]) -> Result<Vec<usize>, Vec<usize>> {
    let n = deps.len();
    let mut waiting = vec![0usize; n];
    let mut dependents = vec![Vec::new(); n];
    for (i, ds) in deps.iter().enumerate() {
        let mut seen = Vec::new();
        for &d in ds {
            assert!(d < n, "dependency {d} out of range");
            if !seen.contains(&d) {
                seen.push(d);
                waiting[i] += 1;
                dependents[d].push(i);
            }
        }
    }
    let mut ready: BinaryHeap<Reverse<usize>> =
        (0..n).filter(|&i| waiting[i] == 0).map(Reverse).collect();
    let mut out = Vec::with_capacity(n);
    while let Some(Reverse(i)) = ready.pop() {
        out.push(i);
        for &j in &dependents[i] {
            waiting[j] -= 1;
            if waiting[j] == 0 {
                ready.push(Reverse(j));
            }
        }
    }
    if out.len() == n {
        return Ok(out);
    }
    // every position left waits on another position left: follow the smallest of them until a
    // position repeats
    let left: Vec<bool> = (0..n).map(|i| waiting[i] > 0).collect();
    let mut path = Vec::new();
    let mut at = (0..n).find(|&i| left[i]).expect("a position is left");
    loop {
        if let Some(start) = path.iter().position(|&p| p == at) {
            return Err(path[start..].to_vec());
        }
        path.push(at);
        at = deps[at]
            .iter()
            .copied()
            .filter(|&d| left[d])
            .min()
            .expect("a position left waits on another one left");
    }
}

#[cfg(test)]
mod tests {
    use super::order;

    #[test]
    fn dependencies_first_ties_by_position() {
        // 0 needs 2, 1 needs nothing, 2 needs 1
        assert_eq!(order(&[vec![2], vec![], vec![1]]), Ok(vec![1, 2, 0]));
        assert_eq!(order(&[vec![], vec![], vec![]]), Ok(vec![0, 1, 2]));
        assert_eq!(order(&[vec![1, 1], vec![]]), Ok(vec![1, 0]));
    }

    #[test]
    fn cycles_are_named() {
        assert_eq!(order(&[vec![0]]), Err(vec![0]));
        // 0 waits on the cycle 1 -> 2 -> 1
        assert_eq!(order(&[vec![1], vec![2], vec![1]]), Err(vec![1, 2]));
        assert_eq!(
            order(&[vec![], vec![3], vec![1], vec![2]]),
            Err(vec![1, 3, 2])
        );
    }
}
