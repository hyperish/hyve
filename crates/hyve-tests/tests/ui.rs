//! Compile-fail tests: every public macro reports what is wrong and how to fix
//! it. Snapshots are tied to the stable compiler; regenerate with
//! `TRYBUILD=overwrite cargo test -p hyve-tests --test ui` and review the diff.

#[test]
fn ui() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/ui/*.rs");
}
