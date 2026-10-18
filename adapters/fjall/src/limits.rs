use crate::FjallError;

/// Maximum key length supported by the storage engine.
pub const MAX_KEY_LEN: usize = 65_535;

/// Maximum stream ID length, reserving 18 bytes for the `$all` key layout.
///
/// This limit applies in both index modes so changing configuration does not
/// change which stream IDs the adapter accepts. Empty IDs are invalid.
pub const MAX_STREAM_ID_LEN: usize = MAX_KEY_LEN - 18;

pub const fn validate_key(key: &[u8], max: usize) -> Result<(), FjallError> {
    if key.is_empty() || key.len() > max {
        return Err(FjallError::InvalidKey {
            len: key.len(),
            max,
        });
    }
    Ok(())
}

#[cfg(feature = "snapshot")]
pub fn state_value_len(payload: usize) -> Result<usize, FjallError> {
    value_len(payload, 12)
}

#[cfg(feature = "projection")]
pub fn checkpoint_value_len(payload: usize) -> Result<usize, FjallError> {
    value_len(payload, crate::checkpoint::HEADER_SIZE)
}

#[cfg(any(feature = "snapshot", feature = "projection"))]
fn value_len(payload: usize, header: usize) -> Result<usize, FjallError> {
    payload
        .checked_add(header)
        .filter(|&len| u32::try_from(len).is_ok())
        .ok_or(FjallError::StateTooLarge { payload, header })
}

#[cfg(all(test, any(feature = "snapshot", feature = "projection")))]
#[allow(clippy::expect_used, reason = "boundary test assertions")]
mod tests {
    use super::*;

    #[cfg(feature = "snapshot")]
    #[test]
    fn state_size_includes_header_without_allocating_large_values() {
        let max = usize::try_from(u32::MAX).expect("supported platforms are at least 32-bit");
        assert_eq!(state_value_len(max - 12).expect("maximum fits"), max);
        assert!(state_value_len(max - 11).is_err());
        assert!(state_value_len(usize::MAX).is_err());
    }

    #[cfg(feature = "projection")]
    #[test]
    fn checkpoint_size_includes_its_larger_header() {
        let max = usize::try_from(u32::MAX).expect("supported platforms are at least 32-bit");
        assert_eq!(checkpoint_value_len(max - 20).expect("maximum fits"), max);
        assert!(matches!(
            checkpoint_value_len(max - 19),
            Err(FjallError::StateTooLarge { header: 20, .. })
        ));
        assert!(checkpoint_value_len(usize::MAX).is_err());
    }
}
