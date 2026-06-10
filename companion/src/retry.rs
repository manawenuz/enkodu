//! Retry strategy for HTTP operations.

use std::thread;
use std::time::Duration;

/// Error classification for retry decisions.
#[derive(Debug, Clone, PartialEq)]
pub enum ErrorKind {
    /// Transient error that should be retried.
    Transient,
    /// Permanent error that should not be retried.
    Permanent,
    /// Network-level error (connection reset, timeout, DNS failure).
    Network,
    /// Server error (5xx, rate limit).
    Server,
}

/// Result of a retryable operation.
pub type RetryResult<T> = Result<T, (ErrorKind, String)>;

/// Retry configuration.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts.
    pub max_retries: u32,
    /// Base delay in milliseconds.
    pub base_delay_ms: u64,
    /// Maximum delay in milliseconds.
    pub max_delay_ms: u64,
    /// Multiplier for exponential backoff.
    pub backoff_multiplier: f64,
    /// Add jitter to avoid thundering herd.
    pub jitter: bool,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 5,
            base_delay_ms: 1000,
            max_delay_ms: 60000,
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

impl RetryConfig {
    /// Conservative config for mobile (shorter delays, fewer retries).
    pub fn mobile() -> Self {
        Self {
            max_retries: 8,
            base_delay_ms: 500,
            max_delay_ms: 30000,
            backoff_multiplier: 1.5,
            jitter: true,
        }
    }

    /// Aggressive config for upload/download (more retries, longer delays).
    pub fn transfer() -> Self {
        Self {
            max_retries: 10,
            base_delay_ms: 1000,
            max_delay_ms: 120000,
            backoff_multiplier: 2.0,
            jitter: true,
        }
    }
}

/// Calculate delay for attempt N (0-indexed).
fn delay_for_attempt(config: &RetryConfig, attempt: u32) -> Duration {
    let base = config.base_delay_ms as f64;
    let delay = base * config.backoff_multiplier.powi(attempt as i32);
    let delay = delay.min(config.max_delay_ms as f64) as u64;

    let jittered = if config.jitter {
        let jitter = rand::random::<f64>() * 0.3 * delay as f64;
        delay + jitter as u64
    } else {
        delay
    };

    Duration::from_millis(jittered)
}

/// Execute a function with retry logic.
/// Returns Ok(T) on success, or the last error on failure.
pub fn with_retry<T, F>(config: &RetryConfig, mut f: F) -> Result<T, String>
where
    F: FnMut() -> RetryResult<T>,
{
    let mut last_error = String::new();

    for attempt in 0..=config.max_retries {
        match f() {
            Ok(val) => return Ok(val),
            Err((kind, msg)) => {
                last_error = msg;
                if kind == ErrorKind::Permanent {
                    break;
                }
                if attempt < config.max_retries {
                    let delay = delay_for_attempt(config, attempt);
                    log::debug!(
                        "Retry attempt {}/{} after {:?}: {}",
                        attempt + 1,
                        config.max_retries,
                        delay,
                        last_error
                    );
                    thread::sleep(delay);
                }
            }
        }
    }

    Err(last_error)
}

/// Classify an HTTP status code into an error kind.
pub fn classify_status(status: u16) -> ErrorKind {
    match status {
        408 | 429 | 502 | 503 | 504 => ErrorKind::Server,
        500 | 501 => ErrorKind::Transient,
        400 | 401 | 403 | 404 | 405 | 422 => ErrorKind::Permanent,
        _ => ErrorKind::Transient,
    }
}

/// Classify a reqwest error into an error kind.
pub fn classify_reqwest_error(err: &reqwest::Error) -> ErrorKind {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        ErrorKind::Network
    } else if err.status().map(|s| s.as_u16() >= 500).unwrap_or(false) {
        ErrorKind::Server
    } else {
        ErrorKind::Transient
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_delay_calculation() {
        let config = RetryConfig::default();
        let d0 = delay_for_attempt(&config, 0);
        let d1 = delay_for_attempt(&config, 1);
        let d2 = delay_for_attempt(&config, 2);

        assert!(d0 >= Duration::from_millis(1000));
        assert!(d1 >= Duration::from_millis(2000));
        assert!(d2 >= Duration::from_millis(4000));
    }

    #[test]
    fn test_status_classification() {
        assert_eq!(classify_status(429), ErrorKind::Server);
        assert_eq!(classify_status(503), ErrorKind::Server);
        assert_eq!(classify_status(404), ErrorKind::Permanent);
        assert_eq!(classify_status(400), ErrorKind::Permanent);
        assert_eq!(classify_status(500), ErrorKind::Transient);
    }

    #[test]
    fn test_retry_succeeds_eventually() {
        let config = RetryConfig {
            max_retries: 3,
            base_delay_ms: 10,
            max_delay_ms: 100,
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let mut attempts = 0;
        let result = with_retry(&config, || {
            attempts += 1;
            if attempts < 3 {
                Err((ErrorKind::Transient, "not yet".to_string()))
            } else {
                Ok(42)
            }
        });

        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn test_retry_respects_permanent() {
        let config = RetryConfig {
            max_retries: 5,
            base_delay_ms: 10,
            max_delay_ms: 100,
            backoff_multiplier: 2.0,
            jitter: false,
        };

        let mut attempts = 0;
        let result: Result<i32, _> = with_retry(&config, || {
            attempts += 1;
            Err((ErrorKind::Permanent, "never".to_string()))
        });

        assert!(result.is_err());
        assert_eq!(attempts, 1);
    }
}
