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
//! - `hyve-core`'s `Ulid` against `ulid` `3.0.0` (generation, encoding,
//!   parsing) and its `SharedRingBuf` against `crossbeam-queue`'s
//!   `ArrayQueue` and `rtrb` (pushes alone and with a concurrent reader;
//!   the originals are consuming queues, so the readers differ).
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

// ------------------------------------------------------------- identifiers

fn ids() {
    use hyve_core::Ulid;
    use std::sync::Mutex;

    // Monotonic generation, as hyve does it for the whole process. The
    // original's `Generator` is an owned value; shared by a process it sits
    // behind a lock, so both forms are measured. Each run counts the IDs
    // that were greater than the one before (all of them, or it aborts).
    fn increasing<T: Ord + Copy>(n: u64, mut next: impl FnMut() -> T) -> (Duration, Check) {
        let mut prev = next();
        let mut up = 0u64;
        let t = Instant::now();
        for _ in 0..n {
            let id = black_box(next());
            up += u64::from(id > prev);
            prev = id;
        }
        (t.elapsed(), (n, up))
    }
    static SHARED: Mutex<ulid::Generator> = Mutex::new(ulid::Generator::new());
    scenario(
        "ULID generate, monotonic",
        1_000_000,
        "id",
        &[
            run("hyve-core Ulid::new (process-wide)", |n| {
                increasing(n, || Ulid::new().to_u128())
            }),
            run("ulid Generator (owned, &mut)", |n| {
                let mut g = ulid::Generator::new();
                increasing(n, move || g.generate().unwrap().0)
            }),
            run("ulid Generator behind a Mutex", |n| {
                increasing(n, || SHARED.lock().unwrap().generate().unwrap().0)
            }),
        ],
    );

    // The same 1000 values through both encoders and parsers.
    let mut g = ulid::Generator::new();
    let values: Vec<u128> = (0..1000).map(|_| g.generate().unwrap().0).collect();
    let texts: Vec<String> = values
        .iter()
        .map(|v| Ulid::from_u128(*v).to_string())
        .collect();
    for (v, s) in values.iter().zip(&texts) {
        assert_eq!(*s, ulid::Ulid(*v).to_string(), "the same text");
        assert_eq!(
            s.parse::<Ulid>().unwrap().to_u128(),
            *v,
            "hyve parses it back"
        );
        assert_eq!(
            ulid::Ulid::from_string(s).unwrap().0,
            *v,
            "ulid parses it back"
        );
    }
    fn each<T>(
        n: u64,
        len: usize,
        mut f: impl FnMut(usize) -> T,
        check: impl Fn(&T) -> u64,
    ) -> (Duration, Check) {
        let mut sum = 0u64;
        let t = Instant::now();
        for i in 0..n as usize {
            sum = sum.wrapping_add(check(&black_box(f(i % len))));
        }
        (t.elapsed(), (n, sum))
    }
    let (v1, v2) = (values.clone(), values);
    scenario(
        "ULID encode (26 chars, no allocation)",
        1_000_000,
        "id",
        &[
            run("hyve-core Ulid::encode", move |n| {
                each(
                    n,
                    v1.len(),
                    |i| Ulid::from_u128(v1[i]).encode(),
                    |b| u64::from(b[25]),
                )
            }),
            run("ulid Ulid::array_to_str", move |n| {
                each(
                    n,
                    v2.len(),
                    |i| {
                        let mut buf = [0u8; 26];
                        ulid::Ulid(v2[i]).array_to_str(&mut buf);
                        buf
                    },
                    |b| u64::from(b[25]),
                )
            }),
        ],
    );
    let (t1, t2) = (texts.clone(), texts);
    scenario(
        "ULID parse",
        1_000_000,
        "id",
        &[
            run("hyve-core str::parse::<Ulid>", move |n| {
                each(
                    n,
                    t1.len(),
                    |i| t1[i].parse::<Ulid>().unwrap().to_u128(),
                    |v| *v as u64,
                )
            }),
            run("ulid Ulid::from_string", move |n| {
                each(
                    n,
                    t2.len(),
                    |i| ulid::Ulid::from_string(&t2[i]).unwrap().0,
                    |v| *v as u64,
                )
            }),
        ],
    );
}

// ------------------------------------------------------------------- rings

/// A 32-byte sample that knows when it is torn.
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
#[repr(C)]
struct Sample {
    seq: u64,
    check: u64,
    x: f32,
    y: f32,
    z: u32,
    _pad: u32,
}

impl Sample {
    fn new(seq: u64) -> Self {
        Sample {
            seq,
            check: !seq,
            x: seq as f32,
            y: -(seq as f32),
            z: seq as u32,
            _pad: 0,
        }
    }
    fn intact(&self) -> bool {
        self.check == !self.seq && self.z == self.seq as u32
    }
}

const RING: usize = 1024;

fn rings() {
    use bytemuck::Zeroable;
    use hyve_core::SharedRingBuf;

    // What the ring holds afterwards: how many samples and the sum of their
    // sequence numbers (the latest `RING` of them in every library).
    fn held(samples: impl Iterator<Item = Sample>) -> u64 {
        samples.map(|s| s.seq).sum()
    }
    scenario(
        "ring push, one thread (keeps the latest 1024)",
        1_000_000,
        "push",
        &[
            run("hyve-core SharedRingBuf::push", |n| {
                let ring = SharedRingBuf::<Sample>::new(RING);
                let t = Instant::now();
                for i in 0..n {
                    ring.push(black_box(Sample::new(i)));
                }
                let e = t.elapsed();
                let mut out = vec![Sample::zeroed(); RING];
                let k = ring.read_latest_snapshot(&mut out);
                (e, (k as u64, held(out[..k].iter().copied())))
            }),
            run("crossbeam ArrayQueue::force_push", |n| {
                let q = crossbeam_queue::ArrayQueue::new(RING);
                let t = Instant::now();
                for i in 0..n {
                    black_box(q.force_push(black_box(Sample::new(i))));
                }
                let e = t.elapsed();
                let k = q.len() as u64;
                (e, (k, held(std::iter::from_fn(|| q.pop()))))
            }),
            run("rtrb push (pop the oldest when full)", |n| {
                let (mut p, mut c) = rtrb::RingBuffer::new(RING);
                let t = Instant::now();
                for i in 0..n {
                    if p.is_full() {
                        black_box(c.pop().unwrap());
                    }
                    p.push(black_box(Sample::new(i))).unwrap();
                }
                let e = t.elapsed();
                let k = c.slots() as u64;
                (e, (k, held(std::iter::from_fn(|| c.pop().ok()))))
            }),
        ],
    );

    // No counterpart: the originals have no read that leaves the samples in
    // place.
    scenario(
        "ring snapshot of the latest 64 (hyve only: the originals consume)",
        1_000_000,
        "snapshot",
        &[run("hyve-core read_latest_snapshot", |n| {
            let ring = SharedRingBuf::<Sample>::new(RING);
            for i in 0..RING as u64 * 2 {
                ring.push(Sample::new(i));
            }
            let mut out = [Sample::zeroed(); 64];
            let mut sum = 0u64;
            let t = Instant::now();
            for _ in 0..n {
                let k = ring.read_latest_snapshot(black_box(&mut out));
                sum = sum.wrapping_add(k as u64 + out[0].seq);
            }
            (t.elapsed(), (n, sum))
        })],
    );

    // A writer thread with a reader thread reading as fast as it can: the
    // time is the writer's, per push. Readers do what each library offers
    // (hyve copies the latest 64 in place; the queues pop). The check is the
    // number of pushes and of torn samples a reader saw (0).
    fn contended<R: Send + 'static>(
        n: u64,
        push: impl FnMut(u64),
        reader: R,
        read: fn(&mut R) -> (u64, u64),
    ) -> (Duration, Check) {
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = std::thread::spawn(move || {
            let mut reader = reader;
            let (mut seen, mut torn) = (0u64, 0u64);
            while !stop2.load(Ordering::Relaxed) {
                let (s, t) = read(&mut reader);
                seen += s;
                torn += t;
            }
            (seen, torn)
        });
        let mut push = push;
        let t = Instant::now();
        for i in 0..n {
            push(i);
        }
        let e = t.elapsed();
        stop.store(true, Ordering::Relaxed);
        let (seen, torn) = handle.join().unwrap();
        assert!(seen > 0, "the reader saw samples");
        (e, (n, torn))
    }
    scenario(
        "ring push with a concurrent reader (cap 1024)",
        1_000_000,
        "push",
        &[
            run("hyve-core push / read_latest_snapshot(64)", |n| {
                let ring = SharedRingBuf::<Sample>::new(RING);
                let reader = (ring.clone(), [Sample::zeroed(); 64]);
                contended(
                    n,
                    move |i| ring.push(Sample::new(i)),
                    reader,
                    |(r, out)| {
                        let k = r.read_latest_snapshot(out);
                        (
                            k as u64,
                            out[..k].iter().filter(|s| !s.intact()).count() as u64,
                        )
                    },
                )
            }),
            run("crossbeam force_push / pop", |n| {
                let q = Arc::new(crossbeam_queue::ArrayQueue::new(RING));
                let reader = q.clone();
                contended(
                    n,
                    move |i| {
                        black_box(q.force_push(Sample::new(i)));
                    },
                    reader,
                    |q| match q.pop() {
                        Some(s) => (1, u64::from(!s.intact())),
                        None => (0, 0),
                    },
                )
            }),
            run("rtrb push (spins when full) / pop", |n| {
                let (mut p, c) = rtrb::RingBuffer::new(RING);
                contended(
                    n,
                    move |i| {
                        let mut s = Sample::new(i);
                        while let Err(rtrb::PushError::Full(back)) = p.push(s) {
                            s = back;
                            std::hint::spin_loop();
                        }
                    },
                    c,
                    |c| match c.pop() {
                        Ok(s) => (1, u64::from(!s.intact())),
                        Err(_) => (0, 0),
                    },
                )
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
    ids();
    rings();
}
