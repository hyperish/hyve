//! Signed update acceptance, including concurrent installers.
#![cfg(not(target_arch = "wasm32"))]
use hyve_update::{Release, apply, check, create, generate_key};
use std::{
    fs,
    io::{self, Cursor},
    sync::Barrier,
};

#[test]
fn signed_transitions_are_atomic_serialized_and_bound_to_this_installation() {
    let dir = tempfile::tempdir().unwrap();
    let app = dir.path().join("client");
    let base = (0..60_000).map(|i| (i % 251) as u8).collect::<Vec<_>>();
    let mut target = base.clone();
    target[4..12].copy_from_slice(b"artifact");
    fs::write(&app, &base).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&app, fs::Permissions::from_mode(0o751)).unwrap();
    }
    let release = Release {
        app: "org.example.notes",
        target: "linux-arm64",
        number: 7,
    };
    let (key, public) = generate_key().unwrap();
    let mut delta = Vec::new();
    create(release, 8, &base[..], &target[..], &key, &mut delta).unwrap();
    assert!(delta.len() < target.len() / 4);
    assert!(create(release, 6, &base[..], &target[..], &key, Vec::new()).is_err());
    let (_, wrong_key) = generate_key().unwrap();
    for mismatch in [
        Release {
            app: "org.example.chat",
            ..release
        },
        Release {
            target: "windows-amd64",
            ..release
        },
        Release {
            number: 8,
            ..release
        },
    ] {
        assert!(apply(mismatch, &public, &app, &delta[..]).is_err());
    }
    assert!(apply(release, &wrong_key, &app, &delta[..]).is_err());
    let mut broken = delta.clone();
    broken[20] ^= 1;
    assert!(apply(release, &public, &app, &broken[..]).is_err());
    fs::write(&app, b"user-owned original").unwrap();
    assert!(apply(release, &public, &app, &delta[..]).is_err());
    assert_eq!(fs::read(&app).unwrap(), b"user-owned original");
    fs::write(&app, &base).unwrap();
    let gate = Barrier::new(2);
    let success = std::thread::scope(|scope| {
        let worker = || {
            gate.wait();
            apply(release, &public, &app, Cursor::new(&delta)).is_ok()
        };
        let a = scope.spawn(worker);
        let b = scope.spawn(worker);
        [a.join().unwrap(), b.join().unwrap()]
            .into_iter()
            .filter(|ok| *ok)
            .count()
    });
    assert_eq!(success, 1);
    assert_eq!(fs::read(&app).unwrap(), target);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&app).unwrap().permissions().mode() & 0o777,
            0o751
        );
    }
    let current = Release {
        number: 8,
        ..release
    };
    assert!(
        check(current, &public, &app, |_| Ok(None::<&[u8]>))
            .unwrap()
            .is_none()
    );
    let payload = b"third release contents";
    let mut third = Vec::new();
    create(current, 9, &target[..], &payload[..], &key, &mut third).unwrap();
    assert_eq!(
        check(current, &public, &app, |r| -> io::Result<_> {
            assert_eq!(r.number, 8);
            Ok(Some(third.as_slice()))
        })
        .unwrap(),
        Some(9)
    );
    assert_eq!(fs::read(app).unwrap(), payload);
}
