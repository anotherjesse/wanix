const DEFAULT_PI_TERMS: u64 = 1_000_000;

pub(crate) fn pi(n: Option<u64>) {
    println!("{:.10}", leibniz_pi(n.unwrap_or(DEFAULT_PI_TERMS)));
}

/// Approximates pi with `n` Leibniz terms: 4 * sum (-1)^k / (2k+1).
/// A pure floating-point hot loop — the CPU benchmark kernel, identical to the
/// native Rust and QuickJS versions so the three execution paths are comparable.
fn leibniz_pi(n: u64) -> f64 {
    let mut acc = 0.0f64;
    let mut sign = 1.0f64;
    for k in 0..n {
        acc += sign / (2 * k + 1) as f64;
        sign = -sign;
    }
    4.0 * acc
}
