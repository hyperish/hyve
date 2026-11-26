//! Endpoint ownership, byte exchange and listener cleanup on the host OS.
#![cfg(all(feature = "ipc", any(unix, windows), not(target_arch = "wasm32")))]
use hyve::{Ulid, ipc};
use std::{path::PathBuf, time::Duration};

#[test]
fn same_user_streams_exchange_bytes_and_release_the_endpoint() {
    #[cfg(unix)]
    let endpoint = {
        use std::os::unix::fs::DirBuilderExt;
        let dir = std::env::temp_dir().join(format!("hi-{}", Ulid::new()));
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
        dir.join("s")
    };
    #[cfg(windows)]
    let endpoint = PathBuf::from(format!(r"\\.\pipe\hyve-{}", Ulid::new()));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut listener = ipc::Listener::bind(&endpoint).unwrap();
        assert!(ipc::Listener::bind(&endpoint).is_err());
        let path = endpoint.clone();
        let client = tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut stream = ipc::connect(&path).await.unwrap();
            stream.write_all(&[7]).await.unwrap();
            let mut reply = [0];
            stream.read_exact(&mut reply).await.unwrap();
            assert_eq!(reply, [9]);
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut stream = listener.accept().await.unwrap();
            let mut request = [0];
            stream.read_exact(&mut request).await.unwrap();
            assert_eq!(request, [7]);
            stream.write_all(&[9]).await.unwrap();
            client.await.unwrap();
        })
        .await
        .unwrap();
        drop(listener);
        let _rebound = ipc::Listener::bind(&endpoint).unwrap();
    });
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let parent = endpoint.parent().unwrap();
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(runtime.block_on(async { ipc::Listener::bind(&endpoint).is_err() }));
        std::fs::remove_dir(parent).unwrap();
    }
    assert!(
        runtime
            .block_on(ipc::connect(&PathBuf::from("relative")))
            .is_err()
    );
}
