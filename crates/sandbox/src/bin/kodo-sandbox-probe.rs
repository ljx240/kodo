//! The probe helper: a tiny program that performs one observable action and
//! reports the result as an exit code.
//!
//! It exists so the sandbox capability probe can *observe* the operating
//! system's behaviour instead of inferring it from a client that failed for
//! an unrelated reason. It uses nothing but the standard library.

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(kodo_sandbox::probe::serve_direct(&args));
}
