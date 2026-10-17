#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub backoff_ms: u64,
}

impl RetryPolicy {
    pub const fn fixed(max_attempts: u32, backoff_ms: u64) -> Self {
        Self {
            max_attempts,
            backoff_ms,
        }
    }
}

pub const HLS_SEGMENT_RETRY_POLICY: RetryPolicy = RetryPolicy::fixed(3, 500);
pub const HTTP_CHUNK_RETRY_POLICY: RetryPolicy = RetryPolicy::fixed(3, 1000);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_policies_are_explicit() {
        assert_eq!(
            HLS_SEGMENT_RETRY_POLICY,
            RetryPolicy {
                max_attempts: 3,
                backoff_ms: 500,
            }
        );
        assert_eq!(
            HTTP_CHUNK_RETRY_POLICY,
            RetryPolicy {
                max_attempts: 3,
                backoff_ms: 1000,
            }
        );
    }
}
