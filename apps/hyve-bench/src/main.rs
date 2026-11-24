//! hyve's own cores against the originals in the fork record
//! (`docs/05-implementation-plan.md`, 포크 기록):
//!
//! - `hyve-state` against `reactive_graph` `0.3.0-rc1` (its signals with
//!   the default `SyncStorage` and with `LocalStorage`; effects are
//!   `ImmediateEffect`, the original's effect that re-runs inside `set`
//!   like hyve's, so no executor is involved).
//! - `hyve-serde`'s binary codec against calling `rkyv` `0.8.18` directly
//!   (encoding is the same call; decoding differs by the depth-bounded
//!   validator and the copy of misaligned input).
//! - `hyve-serde`'s JSON codec (`#[model]`) against `serde_json`.
//!
//! Every scenario does the same work in each library: effect-run counts and
//! final values are compared outside the timed region, and a mismatch
//! aborts. Samples interleave the libraries; each sample times only the hot
//! loop. Reported: median and range of the time per operation. No timing
//! is asserted.
//!
//! Run in this directory: `cargo run --release` (`HYVE_BENCH_ONLY=chain`
//! runs only the scenarios whose title has that text).

use std::cell::Cell;
use std::hint::black_box;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const SAMPLES: usize = 21;

/// What a run did, compared across libraries.
type Check = (u64, u64);

struct Run {
    name: &'static str,
    /// Runs the scenario `n` times; returns the hot loop's time and the
    /// work it did.
    f: Box<dyn Fn(u64) -> (Duration, Check)>,
}

fn run(name: &'static str, f: impl Fn(u64) -> (Duration, Check) + 'static) -> Run {
    Run {
        name,
        f: Box::new(f),
    }
}

fn scenario(title: &str, n: u64, unit: &str, runs: &[Run]) {
    // `HYVE_BENCH_ONLY=<text>` runs only the scenarios whose title has it.
    if std::env::var("HYVE_BENCH_ONLY").is_ok_and(|only| !title.contains(&only)) {
        return;
    }
    // Warm up, and check the libraries did the same work.
    let checks: Vec<Check> = runs.iter().map(|r| (r.f)(n).1).collect();
    for (r, c) in runs.iter().zip(&checks) {
        assert_eq!(
            *c, checks[0],
            "{title}: `{}` did different work than `{}`",
            r.name, runs[0].name
        );
    }
    let mut times: Vec<Vec<f64>> = vec![Vec::new(); runs.len()];
    for _ in 0..SAMPLES {
        for (i, r) in runs.iter().enumerate() {
            let (t, c) = (r.f)(n);
            assert_eq!(c, checks[0], "{title}: `{}` changed its work", r.name);
            times[i].push(t.as_nanos() as f64 / n as f64);
        }
    }
    println!("\n{title}  (n = {n}, check = {:?})", checks[0]);
    for (r, t) in runs.iter().zip(&mut times) {
        t.sort_by(f64::total_cmp);
        println!(
            "  {:<34} median {:>10}  range {} – {}  per {unit}",
            r.name,
            fmt(t[t.len() / 2]),
            fmt(t[0]),
            fmt(t[t.len() - 1])
        );
    }
}

fn fmt(ns: f64) -> String {
    if ns >= 1000.0 {
        format!("{:.2} µs", ns / 1000.0)
    } else {
        format!("{ns:.1} ns")
    }
}

// ---------------------------------------------------------------- signals

mod hy {
    use super::*;
    use hyve_state::{Owner, effect, local_signal, memo, signal};

    fn owned<R>(f: impl FnOnce() -> R) -> R {
        let owner = Owner::new();
        let r = owner.with(f);
        owner.dispose();
        r
    }

    pub fn create(n: u64) -> (Duration, Check) {
        owned(|| {
            let t = Instant::now();
            for i in 0..n {
                black_box(signal(i));
            }
            (t.elapsed(), (n, 0))
        })
    }

    pub fn set_get(n: u64) -> (Duration, Check) {
        owned(|| {
            let s = signal(0u64);
            let mut acc = 0u64;
            let t = Instant::now();
            for i in 0..n {
                s.set(black_box(i));
                acc = acc.wrapping_add(s.get_untracked());
            }
            (t.elapsed(), (n, acc))
        })
    }

    pub fn effect_1(n: u64) -> (Duration, Check) {
        owned(|| {
            let s = local_signal(0u64);
            let (runs, sum) = (Rc::new(Cell::new(0u64)), Rc::new(Cell::new(0u64)));
            let (r, m) = (runs.clone(), sum.clone());
            let _e = effect(move || {
                r.set(r.get() + 1);
                m.set(m.get().wrapping_add(s.get()));
            });
            let t = Instant::now();
            for i in 0..n {
                s.set(black_box(i));
            }
            (t.elapsed(), (runs.get(), sum.get()))
        })
    }

    pub fn fan_out(effects: u64, n: u64) -> (Duration, Check) {
        owned(|| {
            let s = local_signal(0u64);
            let runs = Rc::new(Cell::new(0u64));
            for _ in 0..effects {
                let r = runs.clone();
                effect(move || {
                    black_box(s.get());
                    r.set(r.get() + 1);
                });
            }
            let t = Instant::now();
            for i in 0..n {
                s.set(black_box(i));
            }
            (t.elapsed(), (runs.get(), s.get_untracked()))
        })
    }

    pub fn chain(depth: u64, n: u64) -> (Duration, Check) {
        owned(|| {
            let s = signal(0u64);
            let mut tail = memo(move || s.get() + 1);
            for _ in 1..depth {
                let prev = tail;
                tail = memo(move || prev.get() + 1);
            }
            let mut acc = tail.get_untracked();
            let t = Instant::now();
            for i in 0..n {
                s.set(black_box(i));
                acc = acc.wrapping_add(tail.get_untracked());
            }
            (t.elapsed(), (n, acc))
        })
    }

    /// Effect runs, and runs that saw `d` disagree with `a` (`d = 3a + 1`).
    pub fn diamond(n: u64) -> (u64, u64) {
        owned(|| {
            let a = signal(0u64);
            let b = memo(move || a.get() * 2);
            let c = memo(move || a.get() + 1);
            let d = memo(move || b.get() + c.get());
            let (runs, glitches) = (Rc::new(Cell::new(0u64)), Rc::new(Cell::new(0u64)));
            let (r, g) = (runs.clone(), glitches.clone());
            let _e = effect(move || {
                r.set(r.get() + 1);
                if d.get() != 3 * a.get_untracked() + 1 {
                    g.set(g.get() + 1);
                }
            });
            for i in 0..n {
                a.set(i + 1);
            }
            (runs.get(), glitches.get())
        })
    }
}

/// `reactive_graph` with the signal type chosen by `$new`
/// (`RwSignal::new`: `SyncStorage`; `RwSignal::new_local`: `LocalStorage`).
macro_rules! rg {
    ($m:ident, $new:path) => {
        mod $m {
            use super::*;
            use reactive_graph::computed::Memo;
            use reactive_graph::effect::ImmediateEffect;
            use reactive_graph::owner::Owner;
            use reactive_graph::prelude::*;

            fn owned<R>(f: impl FnOnce() -> R) -> R {
                let owner = Owner::new();
                let r = owner.with(f);
                owner.cleanup();
                r
            }

            pub fn create(n: u64) -> (Duration, Check) {
                owned(|| {
                    let t = Instant::now();
                    for i in 0..n {
                        black_box($new(i));
                    }
                    (t.elapsed(), (n, 0))
                })
            }

            pub fn set_get(n: u64) -> (Duration, Check) {
                owned(|| {
                    let s = $new(0u64);
                    let mut acc = 0u64;
                    let t = Instant::now();
                    for i in 0..n {
                        s.set(black_box(i));
                        acc = acc.wrapping_add(s.get_untracked());
                    }
                    (t.elapsed(), (n, acc))
                })
            }

            pub fn effect_1(n: u64) -> (Duration, Check) {
                owned(|| {
                    let s = $new(0u64);
                    let (runs, sum) = (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
                    let (r, m) = (runs.clone(), sum.clone());
                    let _e = ImmediateEffect::new_isomorphic(move || {
                        r.fetch_add(1, Ordering::Relaxed);
                        m.fetch_add(s.get(), Ordering::Relaxed);
                    });
                    let t = Instant::now();
                    for i in 0..n {
                        s.set(black_box(i));
                    }
                    let e = t.elapsed();
                    (
                        e,
                        (runs.load(Ordering::Relaxed), sum.load(Ordering::Relaxed)),
                    )
                })
            }

            pub fn fan_out(effects: u64, n: u64) -> (Duration, Check) {
                owned(|| {
                    let s = $new(0u64);
                    let runs = Arc::new(AtomicU64::new(0));
                    let mut keep = Vec::with_capacity(effects as usize);
                    for _ in 0..effects {
                        let r = runs.clone();
                        keep.push(ImmediateEffect::new_isomorphic(move || {
                            black_box(s.get());
                            r.fetch_add(1, Ordering::Relaxed);
                        }));
                    }
                    let t = Instant::now();
                    for i in 0..n {
                        s.set(black_box(i));
                    }
                    let e = t.elapsed();
                    (e, (runs.load(Ordering::Relaxed), s.get_untracked()))
                })
            }

            pub fn chain(depth: u64, n: u64) -> (Duration, Check) {
                owned(|| {
                    let s = $new(0u64);
                    let mut tail = Memo::new(move |_| s.get() + 1);
                    for _ in 1..depth {
                        let prev = tail;
                        tail = Memo::new(move |_| prev.get() + 1);
                    }
                    let mut acc = tail.get_untracked();
                    let t = Instant::now();
                    for i in 0..n {
                        s.set(black_box(i));
                        acc = acc.wrapping_add(tail.get_untracked());
                    }
                    (t.elapsed(), (n, acc))
                })
            }

            /// Effect runs, and runs that saw `d` disagree with `a`.
            pub fn diamond(n: u64) -> (u64, u64) {
                owned(|| {
                    let a = $new(0u64);
                    let b = Memo::new(move |_| a.get() * 2);
                    let c = Memo::new(move |_| a.get() + 1);
                    let d = Memo::new(move |_| b.get() + c.get());
                    let (runs, glitches) =
                        (Arc::new(AtomicU64::new(0)), Arc::new(AtomicU64::new(0)));
                    let (r, g) = (runs.clone(), glitches.clone());
                    let _e = ImmediateEffect::new_isomorphic(move || {
                        r.fetch_add(1, Ordering::Relaxed);
                        if d.get() != 3 * a.get_untracked() + 1 {
                            g.fetch_add(1, Ordering::Relaxed);
                        }
                    });
                    for i in 0..n {
                        a.set(i + 1);
                    }
                    (
                        runs.load(Ordering::Relaxed),
                        glitches.load(Ordering::Relaxed),
                    )
                })
            }
        }
    };
}

rg!(rg_sync, reactive_graph::signal::RwSignal::new);
rg!(rg_local, reactive_graph::signal::RwSignal::new_local);

// ------------------------------------------------------------- codecs

mod data {
    use hyve::model;

    #[model]
    #[derive(Debug, Clone, PartialEq)]
    pub struct Item {
        pub id: u64,
        pub title: String,
        pub tags: Vec<String>,
        pub done: bool,
        pub score: f64,
    }

    #[model]
    #[derive(Debug, Clone, PartialEq)]
    pub struct Page {
        pub items: Vec<Item>,
        pub cursor: Option<String>,
    }

    /// The same shape for `serde_json`.
    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    pub struct SItem {
        pub id: u64,
        pub title: String,
        pub tags: Vec<String>,
        pub done: bool,
        pub score: f64,
    }

    #[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
    pub struct SPage {
        pub items: Vec<SItem>,
        pub cursor: Option<String>,
    }

    pub fn page() -> Page {
        Page {
            items: (0..100)
                .map(|i| Item {
                    id: i,
                    title: format!("project number {i} with a longer title"),
                    tags: vec!["alpha".into(), format!("tag-{}", i % 7), "omega".into()],
                    done: i % 3 == 0,
                    score: i as f64 * 1.25,
                })
                .collect(),
            cursor: Some("01JD0000000000000000000000".into()),
        }
    }

    pub fn spage(p: &Page) -> SPage {
        SPage {
            items: p
                .items
                .iter()
                .map(|i| SItem {
                    id: i.id,
                    title: i.title.clone(),
                    tags: i.tags.clone(),
                    done: i.done,
                    score: i.score,
                })
                .collect(),
            cursor: p.cursor.clone(),
        }
    }

    /// A small digest of a page, to check decoders produced the same value.
    pub fn digest(p: &Page) -> u64 {
        p.items
            .iter()
            .map(|i| i.id + i.title.len() as u64 + i.tags.len() as u64 + i.done as u64)
            .sum::<u64>()
            + p.cursor.as_ref().map_or(0, |c| c.len() as u64)
    }

    pub fn sdigest(p: &SPage) -> u64 {
        p.items
            .iter()
            .map(|i| i.id + i.title.len() as u64 + i.tags.len() as u64 + i.done as u64)
            .sum::<u64>()
            + p.cursor.as_ref().map_or(0, |c| c.len() as u64)
    }
}

fn timed<T>(n: u64, mut f: impl FnMut() -> T, check: impl Fn(&T) -> u64) -> (Duration, Check) {
    let mut last = None;
    let t = Instant::now();
    for _ in 0..n {
        last = Some(black_box(f()));
    }
    let e = t.elapsed();
    (e, (n, last.as_ref().map_or(0, check)))
}

fn codecs() {
    use data::*;
    use hyve_serde::Binary;
    use rkyv::rancor::Error as E;

    let page = page();
    let bytes = page.to_binary().unwrap();
    let raw = rkyv::to_bytes::<E>(&page).unwrap();
    assert_eq!(bytes.as_ref(), raw.as_slice(), "the same archive");
    // The same bytes at an odd address.
    let mut shifted = vec![0u8; bytes.as_ref().len() + 1];
    shifted[1..].copy_from_slice(bytes.as_ref());
    let len = bytes.as_ref().len() as u64;

    let (p1, p2) = (page.clone(), page.clone());
    scenario(
        "binary encode (same call: sanity row)",
        2000,
        "page",
        &[
            run("hyve-serde to_binary", move |n| {
                timed(n, || p1.to_binary().unwrap(), |b| b.as_ref().len() as u64)
            }),
            run("rkyv::to_bytes", move |n| {
                timed(n, || rkyv::to_bytes::<E>(&p2).unwrap(), |b| b.len() as u64)
            }),
        ],
    );
    let (b1, b2) = (bytes.clone(), bytes.clone());
    scenario(
        "binary decode, aligned input",
        2000,
        "page",
        &[
            run("hyve-serde from_binary", move |n| {
                timed(n, || Page::from_binary(b1.as_ref()).unwrap(), digest)
            }),
            run("rkyv::from_bytes", move |n| {
                timed(
                    n,
                    || rkyv::from_bytes::<Page, E>(b2.as_ref()).unwrap(),
                    digest,
                )
            }),
        ],
    );
    let (s1, s2) = (shifted.clone(), shifted.clone());
    scenario(
        "binary decode, misaligned input",
        2000,
        "page",
        &[
            run("hyve-serde from_binary (copies)", move |n| {
                timed(n, || Page::from_binary(&s1[1..]).unwrap(), digest)
            }),
            run("copy to AlignedVec + rkyv::from_bytes", move |n| {
                timed(
                    n,
                    || {
                        let mut v = rkyv::util::AlignedVec::<16>::with_capacity(s2.len() - 1);
                        v.extend_from_slice(&s2[1..]);
                        rkyv::from_bytes::<Page, E>(&v).unwrap()
                    },
                    digest,
                )
            }),
        ],
    );
    let (b1, b2) = (bytes.clone(), bytes.clone());
    scenario(
        "binary access (validate, zero-copy)",
        5000,
        "page",
        &[
            run("hyve-serde access", move |n| {
                timed(
                    n,
                    || hyve_serde::access::<Page>(b1.as_ref()).unwrap().items.len(),
                    |l| *l as u64 + len,
                )
            }),
            run("rkyv::access", move |n| {
                timed(
                    n,
                    || {
                        rkyv::access::<ArchivedPage, E>(b2.as_ref())
                            .unwrap()
                            .items
                            .len()
                    },
                    |l| *l as u64 + len,
                )
            }),
        ],
    );

    // JSON.
    let spage = spage(&page);
    let ours = hyve_serde::json::to_json(&page);
    let theirs = serde_json::to_string(&spage).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&ours).unwrap(),
        serde_json::from_str::<serde_json::Value>(&theirs).unwrap(),
        "the same JSON document"
    );
    let (p1, s1) = (page.clone(), spage.clone());
    scenario(
        "JSON encode",
        2000,
        "page",
        &[
            run("hyve-serde to_json (#[model])", move |n| {
                timed(n, || hyve_serde::json::to_json(&p1), |s| s.len() as u64)
            }),
            run("serde_json::to_string", move |n| {
                timed(
                    n,
                    || serde_json::to_string(&s1).unwrap(),
                    |s| s.len() as u64,
                )
            }),
        ],
    );
    // Where the encoding time goes: the page's numbers and strings alone.
    let floats: Vec<f64> = page.items.iter().map(|i| i.score + 0.1).collect();
    let (f1, f2) = (floats.clone(), floats);
    scenario(
        "JSON encode, 100 floats",
        20_000,
        "array",
        &[
            run("hyve-serde to_json", move |n| {
                timed(n, || hyve_serde::json::to_json(&f1), |s| s.len() as u64)
            }),
            run("serde_json::to_string", move |n| {
                timed(
                    n,
                    || serde_json::to_string(&f2).unwrap(),
                    |s| s.len() as u64,
                )
            }),
        ],
    );
    let strings: Vec<String> = page.items.iter().map(|i| i.title.clone()).collect();
    let (t1, t2) = (strings.clone(), strings);
    scenario(
        "JSON encode, 100 strings",
        20_000,
        "array",
        &[
            run("hyve-serde to_json", move |n| {
                timed(n, || hyve_serde::json::to_json(&t1), |s| s.len() as u64)
            }),
            run("serde_json::to_string", move |n| {
                timed(
                    n,
                    || serde_json::to_string(&t2).unwrap(),
                    |s| s.len() as u64,
                )
            }),
        ],
    );
    let (j1, j2) = (ours.clone(), ours);
    scenario(
        "JSON decode",
        2000,
        "page",
        &[
            run("hyve-serde from_json (#[model])", move |n| {
                timed(
                    n,
                    || hyve_serde::json::from_json::<Page>(&j1).unwrap(),
                    digest,
                )
            }),
            run("serde_json::from_str", move |n| {
                timed(n, || serde_json::from_str::<SPage>(&j2).unwrap(), sdigest)
            }),
        ],
    );
}

fn main() {
    println!(
        "hyve-bench: {SAMPLES} interleaved samples per library; median and range of time per operation"
    );
    scenario(
        "signal create",
        10_000,
        "signal",
        &[
            run("hyve-state signal", hy::create),
            run("reactive_graph RwSignal (Sync)", rg_sync::create),
            run("reactive_graph RwSignal (Local)", rg_local::create),
        ],
    );
    scenario(
        "signal set + get_untracked, no subscribers",
        1_000_000,
        "set+get",
        &[
            run("hyve-state signal", hy::set_get),
            run("reactive_graph RwSignal (Sync)", rg_sync::set_get),
            run("reactive_graph RwSignal (Local)", rg_local::set_get),
        ],
    );
    scenario(
        "1 signal → 1 effect (effect runs inside set)",
        100_000,
        "set",
        &[
            run("hyve-state effect", hy::effect_1),
            run("reactive_graph ImmediateEffect (Sync)", rg_sync::effect_1),
            run("reactive_graph ImmediateEffect (Local)", rg_local::effect_1),
        ],
    );
    for (title, effects, n) in [
        ("1 signal → 100 effects", 100u64, 10_000u64),
        ("1 signal → 1000 effects", 1000, 1_000),
    ] {
        scenario(
            title,
            n,
            "set",
            &[
                run("hyve-state effect", move |n| hy::fan_out(effects, n)),
                run("reactive_graph ImmediateEffect (Sync)", move |n| {
                    rg_sync::fan_out(effects, n)
                }),
                run("reactive_graph ImmediateEffect (Local)", move |n| {
                    rg_local::fan_out(effects, n)
                }),
            ],
        );
    }
    // How a set travels down a chain of memos, at three depths.
    for (title, depth, n) in [
        (
            "memo chain, depth 10 (set source, read tail)",
            10u64,
            100_000u64,
        ),
        ("memo chain, depth 100 (set source, read tail)", 100, 2_000),
        ("memo chain, depth 200 (set source, read tail)", 200, 500),
    ] {
        scenario(
            title,
            n,
            "set+read",
            &[
                run("hyve-state memo", move |n| hy::chain(depth, n)),
                run("reactive_graph Memo (Sync signal)", move |n| {
                    rg_sync::chain(depth, n)
                }),
                run("reactive_graph Memo (Local signal)", move |n| {
                    rg_local::chain(depth, n)
                }),
            ],
        );
    }
    // Behavior, not timing: the libraries do different work here.
    if std::env::var("HYVE_BENCH_ONLY").is_ok_and(|o| "diamond".contains(&o))
        || std::env::var("HYVE_BENCH_ONLY").is_err()
    {
        let n = 1000;
        println!(
            "\ndiamond a → (b, c) → d → effect, {n} sets (behavior: effect runs, runs that saw d ≠ 3a + 1)"
        );
        for (name, (runs, glitches)) in [
            ("hyve-state memo + effect", hy::diamond(n)),
            (
                "reactive_graph Memo + ImmediateEffect (Sync)",
                rg_sync::diamond(n),
            ),
            (
                "reactive_graph Memo + ImmediateEffect (Local)",
                rg_local::diamond(n),
            ),
        ] {
            println!("  {name:<46} runs {runs:>5}  inconsistent {glitches:>5}");
        }
    }
    codecs();
}
