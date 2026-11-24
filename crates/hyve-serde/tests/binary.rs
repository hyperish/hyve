//! Binary codec: round trips, validation of untrusted bytes, and cross-target
//! format (the same archive bytes on 64-bit native and 32-bit wasm).

use hyve_core::{Id, Timestamp, Ulid, Value};
use hyve_serde::__private::rkyv;
use hyve_serde::{AlignedBytes, Binary, access};

#[derive(Debug, PartialEq, rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)]
#[rkyv(crate = hyve_serde::__private::rkyv)]
struct Project {
    id: Id<Project>,
    title: String,
    count: u32,
    tags: Vec<String>,
    created_at: Timestamp,
    meta: Value,
    score: Option<f64>,
}

fn sample() -> Project {
    let mut meta = Value::Null;
    meta.insert("k", vec![1i64, 2]);
    Project {
        id: Id::from_ulid(Ulid::from_parts(1_700_000_000_000, 42)),
        title: "hello".into(),
        count: 7,
        tags: vec!["a".into(), "b".into()],
        created_at: Timestamp::from_unix_micros(1_700_000_000_123_456).unwrap(),
        meta,
        score: Some(0.5),
    }
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn roundtrip_and_zero_copy_access() {
    let p = sample();
    let bytes = p.to_binary().unwrap();
    assert_eq!(Project::from_binary(&bytes).unwrap(), p);
    let archived = access::<Project>(&bytes).unwrap();
    assert_eq!(archived.title, "hello");
    assert_eq!(Id::from(&archived.id), p.id);
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn misaligned_input_is_copied_not_rejected() {
    let bytes = sample().to_binary().unwrap();
    let mut shifted = vec![0u8; bytes.len() + 1];
    shifted[1..].copy_from_slice(&bytes);
    assert_eq!(Project::from_binary(&shifted[1..]).unwrap(), sample());
}

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn untrusted_bytes_are_validated() {
    let bytes = sample().to_binary().unwrap();
    assert!(Project::from_binary(&bytes[..bytes.len() / 2]).is_err());
    assert!(Project::from_binary(&[]).is_err());
    // Flip every byte one at a time: never a panic, never UB; either an error or
    // a value that re-encodes.
    for i in 0..bytes.len() {
        let mut b = bytes.to_vec();
        b[i] ^= 0xA5;
        let aligned = AlignedBytes::copy_from(&b);
        if let Ok(p) = Project::from_binary(&aligned) {
            let _ = p.to_binary();
        }
    }
}

/// Bytes produced on the reference platform (64-bit native) for `sample()`;
/// the wasm32 run checks it decodes the same bytes. Regenerate with
/// `cargo test -p hyve-serde --test binary -- --ignored --nocapture print_reference`.
const REFERENCE_LEN: usize = include_bytes!("reference.bin").len();

#[cfg_attr(
    all(target_arch = "wasm32", target_os = "unknown"),
    wasm_bindgen_test::wasm_bindgen_test
)]
#[cfg_attr(not(all(target_arch = "wasm32", target_os = "unknown")), test)]
fn cross_target_reference_archive() {
    let reference = AlignedBytes::copy_from(include_bytes!("reference.bin"));
    assert_eq!(reference.len(), REFERENCE_LEN);
    assert_eq!(Project::from_binary(&reference).unwrap(), sample());
    assert_eq!(&*sample().to_binary().unwrap(), &*reference);
}

#[test]
#[ignore]
fn print_reference() {
    let bytes = sample().to_binary().unwrap();
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/tests/reference.bin"),
        &*bytes,
    )
    .unwrap();
}

#[test]
fn deep_nesting_is_rejected_without_exhausting_the_stack() {
    fn nested(depth: usize) -> Value {
        let mut v = Value::Null;
        for _ in 0..depth {
            v = Value::List(vec![v]);
        }
        v
    }
    fn flatten(mut v: Value) {
        // Iterative drop (a recursive drop of a deep value needs a deep stack).
        while let Value::List(mut items) = v {
            v = items.pop().unwrap_or(Value::Null);
        }
    }
    fn on_stack<R: Send + 'static>(stack: usize, f: impl FnOnce() -> R + Send + 'static) -> R {
        std::thread::Builder::new()
            .stack_size(stack)
            .spawn(f)
            .unwrap()
            .join()
            .expect("stack overflow")
    }

    // Producing a hostile archive needs a big stack (the encoder recurses).
    let bytes = on_stack(256 << 20, || {
        let deep = nested(20_000);
        let bytes = deep.to_binary().unwrap();
        flatten(deep);
        bytes
    });
    // Decoding it on an ordinary 2 MiB stack fails cleanly.
    let result = on_stack(2 << 20, move || Value::from_binary(&bytes).map(flatten));
    assert!(result.is_err());

    let ok = nested(hyve_serde::MAX_BINARY_DEPTH / 2);
    assert_eq!(Value::from_binary(&ok.to_binary().unwrap()).unwrap(), ok);
}
