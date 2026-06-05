const KB: usize = 1024;
const MB: usize = 1024 * KB;

const DEFAULT_PROGRESS_DRAW_HZ: u8 = 20;
const DEFAULT_PROGRESS_TICK_MS: u64 = 100;
const DEFAULT_READ_BUFFER_BYTES: usize = 64 * KB;

const DEFAULT_HTTP_CONNECT_TIMEOUT_SECS: u64 = 15;
const DEFAULT_HTTP_READ_TIMEOUT_SECS: u64 = 300;
const DEFAULT_HTTP_POOL_IDLE_TIMEOUT_SECS: u64 = 90;
const DEFAULT_HTTP_REDIRECT_LIMIT: usize = 10;
const DEFAULT_HTTP_RETRY_ATTEMPTS: usize = 4;
const DEFAULT_HTTP_RETRY_BACKOFF_MS: u64 = 500;

pub fn progress_draw_hz() -> u8 {
    parse_env_u8("FSC_PROGRESS_HZ", DEFAULT_PROGRESS_DRAW_HZ, 1, 60)
}

pub fn progress_tick_ms() -> u64 {
    parse_env_u64("FSC_PROGRESS_TICK_MS", DEFAULT_PROGRESS_TICK_MS, 16, 2_000)
}

pub fn read_buffer_bytes() -> usize {
    parse_env_usize(
        "FSC_READ_BUFFER_BYTES",
        DEFAULT_READ_BUFFER_BYTES,
        8 * KB,
        8 * MB,
    )
}

pub fn http_connect_timeout_secs() -> u64 {
    parse_env_u64(
        "FSC_HTTP_CONNECT_TIMEOUT_SECS",
        DEFAULT_HTTP_CONNECT_TIMEOUT_SECS,
        1,
        300,
    )
}

pub fn http_read_timeout_secs() -> u64 {
    parse_env_u64_with_fallback(
        "FSC_HTTP_READ_TIMEOUT_SECS",
        "FSC_HTTP_REQUEST_TIMEOUT_SECS",
        DEFAULT_HTTP_READ_TIMEOUT_SECS,
        5,
        3_600,
    )
}

pub fn http_pool_idle_timeout_secs() -> u64 {
    parse_env_u64(
        "FSC_HTTP_POOL_IDLE_TIMEOUT_SECS",
        DEFAULT_HTTP_POOL_IDLE_TIMEOUT_SECS,
        5,
        600,
    )
}

pub fn http_redirect_limit() -> usize {
    parse_env_usize(
        "FSC_HTTP_REDIRECT_LIMIT",
        DEFAULT_HTTP_REDIRECT_LIMIT,
        0,
        50,
    )
}

pub fn http_retry_attempts() -> usize {
    parse_env_usize(
        "FSC_HTTP_RETRY_ATTEMPTS",
        DEFAULT_HTTP_RETRY_ATTEMPTS,
        0,
        10,
    )
}

pub fn http_retry_backoff_ms() -> u64 {
    parse_env_u64(
        "FSC_HTTP_RETRY_BACKOFF_MS",
        DEFAULT_HTTP_RETRY_BACKOFF_MS,
        50,
        30_000,
    )
}

pub fn upload_parallelism(file_size: u64) -> usize {
    const GB: u64 = 1024 * 1024 * 1024;

    let from_size = match file_size {
        s if s > 50 * GB => 3,
        s if s > 10 * GB => 4,
        s if s > GB => 5,
        _ => 6,
    };

    let cpu_bound = std::thread::available_parallelism()
        .map(|n| n.get().clamp(2, 8))
        .unwrap_or(4);

    let default_max = from_size.min(cpu_bound);
    let mut min = parse_env_usize("FSC_UPLOAD_PARALLELISM_MIN", 2, 1, 32);
    let mut max = parse_env_usize("FSC_UPLOAD_PARALLELISM_MAX", default_max, 1, 32);

    if min > max {
        std::mem::swap(&mut min, &mut max);
    }

    from_size.clamp(min, max)
}

fn parse_env_u8(key: &str, default: u8, min: u8, max: u8) -> u8 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u8>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn parse_env_u64(key: &str, default: u64, min: u64, max: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn parse_env_u64_with_fallback(
    key: &str,
    fallback_key: &str,
    default: u64,
    min: u64,
    max: u64,
) -> u64 {
    std::env::var(key)
        .or_else(|_| std::env::var(fallback_key))
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn parse_env_usize(key: &str, default: usize, min: usize, max: usize) -> usize {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::upload_parallelism;

    #[test]
    fn parallelism_bounds_are_sensible() {
        let small = upload_parallelism(128 * 1024 * 1024);
        let huge = upload_parallelism(200 * 1024 * 1024 * 1024);
        assert!((1..=32).contains(&small));
        assert!((1..=32).contains(&huge));
        assert!(huge <= small);
    }
}
