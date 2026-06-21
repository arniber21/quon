//! Qubit-index key codec for the §8.3 noise wire format.
//!
//! JSON object keys are strings, so the domain [`NoiseModel`](crate::target::NoiseModel)'s
//! `usize` and `(usize, usize)` qubit keys are encoded as `"q"` and `"u,v"` on
//! the wire. Both directions of `NoiseDescriptor` ↔ `NoiseModel` go through this
//! one module, so the encoding and decoding cannot drift apart.

use crate::error::BackendError;
use crate::target::qubit_in_range;

/// Encodes a single-qubit key.
pub fn encode_qubit(q: usize) -> String {
    q.to_string()
}

/// Decodes a single-qubit key, validating it lies in `0..num_qubits`.
pub fn decode_qubit(key: &str, num_qubits: usize) -> Result<usize, BackendError> {
    let q: usize = key
        .parse()
        .map_err(|_| BackendError::BadQubitKey(key.to_string()))?;
    if qubit_in_range(q, num_qubits) {
        Ok(q)
    } else {
        Err(BackendError::QubitOutOfRange { got: q, num_qubits })
    }
}

/// Encodes a two-qubit key as `"u,v"`.
pub fn encode_pair(u: usize, v: usize) -> String {
    format!("{u},{v}")
}

/// Decodes a `"u,v"` two-qubit key into validated indices.
pub fn decode_pair(key: &str, num_qubits: usize) -> Result<(usize, usize), BackendError> {
    let (u, v) = key
        .split_once(',')
        .ok_or_else(|| BackendError::BadTwoQubitKey(key.to_string()))?;
    let u = u
        .trim()
        .parse::<usize>()
        .map_err(|_| BackendError::BadTwoQubitKey(key.to_string()))?;
    let v = v
        .trim()
        .parse::<usize>()
        .map_err(|_| BackendError::BadTwoQubitKey(key.to_string()))?;
    if !qubit_in_range(u, num_qubits) {
        return Err(BackendError::QubitOutOfRange { got: u, num_qubits });
    }
    if !qubit_in_range(v, num_qubits) {
        return Err(BackendError::QubitOutOfRange { got: v, num_qubits });
    }
    Ok((u, v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        /// `decode_qubit ∘ encode_qubit` is the identity for in-range indices.
        #[test]
        fn single_qubit_key_round_trips(q in 0usize..256, extra in 0usize..256) {
            let num_qubits = q + extra + 1; // guarantees q < num_qubits
            prop_assert_eq!(decode_qubit(&encode_qubit(q), num_qubits).unwrap(), q);
        }

        /// `decode_pair ∘ encode_pair` is the identity for in-range indices.
        #[test]
        fn pair_key_round_trips(u in 0usize..256, v in 0usize..256, extra in 0usize..256) {
            let num_qubits = u.max(v) + extra + 1;
            prop_assert_eq!(decode_pair(&encode_pair(u, v), num_qubits).unwrap(), (u, v));
        }
    }

    #[test]
    fn rejects_out_of_range() {
        assert!(matches!(
            decode_qubit("5", 3),
            Err(BackendError::QubitOutOfRange { .. })
        ));
        assert!(matches!(
            decode_pair("1,9", 3),
            Err(BackendError::QubitOutOfRange { .. })
        ));
    }

    #[test]
    fn rejects_malformed() {
        assert!(matches!(
            decode_qubit("x", 3),
            Err(BackendError::BadQubitKey(_))
        ));
        assert!(matches!(
            decode_pair("1", 3),
            Err(BackendError::BadTwoQubitKey(_))
        ));
        assert!(matches!(
            decode_pair("a,b", 3),
            Err(BackendError::BadTwoQubitKey(_))
        ));
    }
}
