//! `#[hyve::test]`.

use hyve::prelude::*;

#[hyve::test]
async fn async_tests_run_inside_a_root_scope() -> HyveResult<()> {
    let scope = Scope::current().expect("tests run inside a root scope");
    scope.provide(7u32);
    let seven = std::future::ready(Scope::current().and_then(|s| s.get::<u32>())).await;
    assert_eq!(seven.map(|c| *c), Some(7));
    Ok(())
}

#[hyve::test]
async fn errors_fail_the_test_through_the_result() -> HyveResult<()> {
    let fps = signal(60u32);
    fps.set(120);
    assert_eq!(fps.get(), 120);
    Ok(())
}

#[hyve::test]
#[should_panic(expected = "boom")]
async fn other_test_attributes_pass_through() {
    panic!("boom");
}

#[hyve::test]
fn sync_tests_are_plain_tests() {
    assert_eq!(1 + 1, 2);
}
