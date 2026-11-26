//! Dependency boundaries checked by the workspace lint.

/// Runtime-neutral family crates.
pub const CORE: [&str; 7] = [
    "hyve-core",
    "hyve-state",
    "hyve-serde",
    "hyve-validate",
    "hyve-config",
    "hyve-api",
    "hyve-trace",
];

/// Packages that belong in adapters, with their diagnostic category.
pub const FORBIDDEN: [(&str, &str); 21] = [
    ("tokio", "async runtime"),
    ("async-std", "async runtime"),
    ("smol", "async runtime"),
    ("async-executor", "async runtime"),
    ("async-io", "async runtime"),
    ("mio", "async runtime"),
    ("futures-executor", "async runtime"),
    ("http", "HTTP"),
    ("hyper", "HTTP"),
    ("h2", "HTTP"),
    ("reqwest", "HTTP"),
    ("ureq", "HTTP"),
    ("axum", "HTTP"),
    ("web-sys", "DOM"),
    ("wgpu", "GPU"),
    ("wgpu-core", "GPU"),
    ("wgpu-hal", "GPU"),
    ("ash", "GPU"),
    ("metal", "GPU"),
    ("glow", "GPU"),
    ("winit", "windowing"),
];
