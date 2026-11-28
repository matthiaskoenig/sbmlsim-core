//! The sizes of the arrays the crate makes, refused as numpy refuses them: a size is counted
//! without a wrap, on every target (a `usize` of 32 bits in wasm), and an array whose bytes
//! exceed the largest `isize` is the `ValueError` of numpy (`array is too big`), before any
//! memory is asked for.
//!
//! A size below that limit is reserved with `try_reserve_exact`, whose refusal is the same
//! error. That is no check of the memory: where the system overcommits (Linux by default) a
//! reservation succeeds and the memory runs out when it is filled, as with numpy, so the
//! memory of a size which can be counted is the caller's.

use crate::{Result, StatsError};

/// The message of the ValueError of numpy of an array whose bytes exceed the largest `isize`.
pub(crate) const ARRAY_TOO_BIG: &str =
    "array is too big; `arr.size * arr.dtype.itemsize` is larger than the maximum possible size.";

/// The error of an array too big to count or to hold, numpy's.
pub(crate) fn too_big() -> StatsError {
    StatsError::value(ARRAY_TOO_BIG)
}

/// The number of elements of an array of the shape `shape` of elements of the type `T`, the
/// product of the dimensions, as numpy counts them before it allocates: an error where the
/// bytes of the dimensions which are not zero exceed the largest `isize`, which numpy refuses
/// even where a dimension of zero leaves no element.
pub(crate) fn array_len<T>(shape: &[usize]) -> Result<usize> {
    let item = size_of::<T>().max(1);
    let mut bytes = item;
    let mut empty = false;
    for &dimension in shape {
        if dimension == 0 {
            empty = true;
            continue;
        }
        bytes = bytes
            .checked_mul(dimension)
            .filter(|&bytes| isize::try_from(bytes).is_ok())
            .ok_or_else(too_big)?;
    }
    Ok(if empty { 0 } else { bytes / item })
}

/// An empty vector with room for `len` elements, the error of numpy where the allocator
/// refuses them.
pub(crate) fn buffer<T>(len: usize) -> Result<Vec<T>> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len).map_err(|_| too_big())?;
    Ok(buffer)
}

/// A vector of `len` copies of `value`, the error of numpy where the allocator refuses them.
pub(crate) fn filled<T: Clone>(len: usize, value: T) -> Result<Vec<T>> {
    let mut filled = buffer(len)?;
    filled.resize(len, value);
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_size_is_counted_as_numpy_counts_it() {
        assert_eq!(array_len::<f64>(&[3, 4]), Ok(12));
        assert_eq!(array_len::<f64>(&[]), Ok(1));
        assert_eq!(array_len::<f64>(&[0, 5]), Ok(0));
        // the bytes of the largest array: isize::MAX rounded down to whole doubles
        let largest = isize::MAX as usize / 8;
        assert_eq!(array_len::<f64>(&[largest]), Ok(largest));
        assert_eq!(array_len::<f64>(&[largest + 1]), Err(too_big()));
        assert_eq!(
            array_len::<u8>(&[isize::MAX as usize]),
            Ok(isize::MAX as usize)
        );
        // a product which wraps a usize, of 32 bits and of 64 bits
        let half = 1usize << (usize::BITS / 2);
        assert_eq!(array_len::<f64>(&[half, half]), Err(too_big()));
        // a product which a usize holds but whose bytes exceed the largest isize
        assert_eq!(array_len::<f64>(&[half / 2, half / 2]), Err(too_big()));
        // numpy refuses the other dimensions of an empty array as well
        assert_eq!(array_len::<f64>(&[0, half, half]), Err(too_big()));
        assert_eq!(array_len::<f64>(&[half / 16, 0]), Ok(0));
    }

    #[test]
    #[cfg(target_pointer_width = "32")]
    fn a_size_past_32_bits_is_refused_in_wasm() {
        // 70000^2 wraps a usize of 32 bits, not one of 64 bits
        assert_eq!(array_len::<f64>(&[70_000, 70_000]), Err(too_big()));
        // 2^29 doubles: a usize of 32 bits counts them, their bytes exceed its largest isize
        assert_eq!(array_len::<f64>(&[1 << 14, 1 << 15]), Err(too_big()));
        assert_eq!(array_len::<u8>(&[1 << 14, 1 << 15]), Ok(1 << 29));
    }

    #[test]
    fn a_buffer_past_the_largest_isize_is_refused() {
        assert_eq!(buffer::<f64>(usize::MAX), Err(too_big()));
        assert_eq!(filled(isize::MAX as usize / 4, 0.0), Err(too_big()));
        assert_eq!(filled(3, 1.5), Ok(vec![1.5; 3]));
    }
}
