use crate::models::RateEntry;

pub struct Pricing {
    pub input: f64,
    pub output: f64,
    pub cache_write_5m: f64,
    pub cache_write_1h: f64,
    pub cache_read: f64,
}

pub fn get_pricing(model: &str) -> Pricing {
    let n = normalize_model(model);
    if n.contains("opus") {
        // base input $15 → 5m write 1.25x, 1h write 2x, read 0.1x
        Pricing { input: 15.00, output: 75.00, cache_write_5m: 18.75, cache_write_1h: 30.00, cache_read: 1.50 }
    } else if n.contains("haiku") {
        Pricing { input: 0.80, output: 4.00, cache_write_5m: 1.00, cache_write_1h: 1.60, cache_read: 0.08 }
    } else {
        // sonnet + unknown fallback
        Pricing { input: 3.00, output: 15.00, cache_write_5m: 3.75, cache_write_1h: 6.00, cache_read: 0.30 }
    }
}

/// Strip 8-digit date suffixes: claude-opus-4-6-20250514 → claude-opus-4-6
pub fn normalize_model(model: &str) -> String {
    let parts: Vec<&str> = model.split('-').collect();
    let end = if parts.last().map_or(false, |p| p.len() == 8 && p.chars().all(|c| c.is_ascii_digit())) {
        parts.len() - 1
    } else {
        parts.len()
    };
    parts[..end].join("-")
}

/// Compute per-bucket cost. `cache_write_1h_tokens` is the portion of
/// `cache_write_tokens` that used the 1-hour TTL (billed at 2x base input);
/// the remainder is billed at the 5-minute rate (1.25x).
pub fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    cache_write_tokens: u64,
    cache_write_1h_tokens: u64,
    cache_read_tokens: u64,
    model: &str,
) -> (f64, f64, f64, f64) {
    let p = get_pricing(model);
    let write_1h = cache_write_1h_tokens.min(cache_write_tokens);
    let write_5m = cache_write_tokens - write_1h;
    (
        (input_tokens as f64 / 1_000_000.0) * p.input,
        (output_tokens as f64 / 1_000_000.0) * p.output,
        (write_5m as f64 / 1_000_000.0) * p.cache_write_5m
            + (write_1h as f64 / 1_000_000.0) * p.cache_write_1h,
        (cache_read_tokens as f64 / 1_000_000.0) * p.cache_read,
    )
}

pub fn rate_card() -> Vec<RateEntry> {
    vec![
        RateEntry {
            model: "claude-opus-4".into(),
            input_per_mtok: 15.00,
            output_per_mtok: 75.00,
            cache_write_5m_per_mtok: 18.75,
            cache_write_1h_per_mtok: 30.00,
            cache_read_per_mtok: 1.50,
        },
        RateEntry {
            model: "claude-sonnet-4".into(),
            input_per_mtok: 3.00,
            output_per_mtok: 15.00,
            cache_write_5m_per_mtok: 3.75,
            cache_write_1h_per_mtok: 6.00,
            cache_read_per_mtok: 0.30,
        },
        RateEntry {
            model: "claude-haiku-4".into(),
            input_per_mtok: 0.80,
            output_per_mtok: 4.00,
            cache_write_5m_per_mtok: 1.00,
            cache_write_1h_per_mtok: 1.60,
            cache_read_per_mtok: 0.08,
        },
    ]
}
