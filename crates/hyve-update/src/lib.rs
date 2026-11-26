//! Signed desktop artifact updates. The installed application pins the key,
//! identity, target and current release. The caller supplies a file or stream
//! from its chosen distribution channel.
#![cfg(not(target_arch = "wasm32"))]
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

use fs2::FileExt;
use qbsdiff::{Bsdiff, Bspatch};
use ring::{
    digest,
    rand::SystemRandom,
    signature::{self, Ed25519KeyPair, KeyPair},
};
use tempfile::NamedTempFile;

const MAGIC: &[u8; 8] = b"HYVEDL01";
const SIG: usize = 64;
/// Maximum accepted artifact and delta length, including network input bounds.
pub const MAX_BYTES: u64 = 512 * 1024 * 1024;

/// The application's trusted, locally configured release identity.
#[derive(Clone, Copy)]
pub struct Release<'a> {
    /// Stable application identifier.
    pub app: &'a str,
    /// Distribution target, including the CPU architecture.
    pub target: &'a str,
    /// Monotonically increasing build number compiled into the current app.
    pub number: u64,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn bounded(reader: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("artifact exceeds the update limit"));
    }
    Ok(bytes)
}

fn label(out: &mut Vec<u8>, s: &str) -> io::Result<()> {
    let len = u16::try_from(s.len()).map_err(|_| invalid("release identifier too long"))?;
    if len == 0 {
        return Err(invalid("missing release identifier"));
    }
    out.extend(len.to_be_bytes());
    out.extend(s.as_bytes());
    Ok(())
}

fn identity(release: Release<'_>) -> io::Result<Vec<u8>> {
    let mut header = MAGIC.to_vec();
    label(&mut header, release.app)?;
    label(&mut header, release.target)?;
    header.extend(release.number.to_be_bytes());
    Ok(header)
}

/// Creates an Ed25519 PKCS#8 signing key and its raw 32-byte verification key.
pub fn generate_key() -> io::Result<(Vec<u8>, Vec<u8>)> {
    let key = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())
        .map_err(|_| invalid("key generation failed"))?;
    let public = public_key(key.as_ref())?;
    Ok((key.as_ref().to_vec(), public))
}
/// Derives the public verification key from an existing PKCS#8 release key.
pub fn public_key(secret: &[u8]) -> io::Result<Vec<u8>> {
    Ok(signing_key(secret)?.public_key().as_ref().to_vec())
}
fn signing_key(secret: &[u8]) -> io::Result<Ed25519KeyPair> {
    Ed25519KeyPair::from_pkcs8(secret).map_err(|_| invalid("invalid Ed25519 signing key"))
}

/// Produces an authenticated BSDIFF40 delta between two packaged artifacts.
pub fn create(
    release: Release<'_>,
    next: u64,
    source: impl Read,
    target: impl Read,
    secret: &[u8],
    mut out: impl Write,
) -> io::Result<()> {
    if next <= release.number {
        return Err(invalid("the target release must be newer"));
    }
    let key = signing_key(secret)?;
    let mut bytes = identity(release)?;
    let source = bounded(source)?;
    let target = bounded(target)?;
    if target.is_empty() {
        return Err(invalid("empty release artifact"));
    }
    bytes.extend(next.to_be_bytes());
    bytes.extend((target.len() as u64).to_be_bytes());
    bytes.extend(digest::digest(&digest::SHA256, &source).as_ref());
    bytes.extend(digest::digest(&digest::SHA256, &target).as_ref());
    Bsdiff::new(&source, &target).compare(&mut bytes)?;
    if bytes.len() as u64 + SIG as u64 > MAX_BYTES {
        return Err(invalid("delta exceeds the update limit"));
    }
    let signature = key.sign(&bytes);
    out.write_all(&bytes)?;
    out.write_all(signature.as_ref())
}

fn take<'a, const N: usize>(bytes: &mut &'a [u8]) -> io::Result<&'a [u8; N]> {
    let (v, rest) = bytes
        .split_at_checked(N)
        .ok_or_else(|| invalid("truncated delta header"))?;
    *bytes = rest;
    Ok(v.try_into().expect("fixed field length"))
}

struct Limited<'a> {
    file: &'a mut File,
    remaining: u64,
}
impl Write for Limited<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if buf.len() as u64 > self.remaining {
            return Err(invalid("delta output exceeds signed length"));
        }
        let n = self.file.write(buf)?;
        self.remaining -= n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

/// Applies one newer release to a regular file. Verifies the signature, both
/// artifact hashes, application, target, build numbers and output size.
/// A private output file is synced and atomically renamed in the same directory.
/// Concurrent processes updating the same install directory are serialized.
pub fn apply(
    release: Release<'_>,
    public: &[u8],
    installed: &Path,
    delta: impl Read,
) -> io::Result<u64> {
    let envelope = bounded(delta)?;
    let signed_len = envelope
        .len()
        .checked_sub(SIG)
        .ok_or_else(|| invalid("truncated delta"))?;
    let (signed, sig) = envelope.split_at(signed_len);
    signature::UnparsedPublicKey::new(&signature::ED25519, public)
        .verify(signed, sig)
        .map_err(|_| invalid("update signature verification failed"))?;
    let header = identity(release)?;
    let mut bytes = signed
        .strip_prefix(header.as_slice())
        .ok_or_else(|| invalid("release identity or source version mismatch"))?;
    let next = u64::from_be_bytes(*take(&mut bytes)?);
    let size = u64::from_be_bytes(*take(&mut bytes)?);
    if next <= release.number || size == 0 || size > MAX_BYTES {
        return Err(invalid("invalid target release or artifact size"));
    }
    let old = take::<32>(&mut bytes)?;
    let new = take::<32>(&mut bytes)?;
    let patch = Bspatch::new(bytes)?;
    if patch.hint_target_size() != size {
        return Err(invalid("delta size does not match the signed manifest"));
    }
    let dir = installed
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let lock_path = dir.join(".hyve-update.lock");
    if installed.file_name() == lock_path.file_name() {
        return Err(invalid("invalid artifact name"));
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    FileExt::lock_exclusive(&lock)?;
    let meta = installed.symlink_metadata()?;
    if !meta.is_file() {
        return Err(invalid("expected a regular installed artifact"));
    }
    let source = bounded(File::open(installed)?)?;
    if digest::digest(&digest::SHA256, &source).as_ref() != old {
        return Err(invalid("installed artifact differs from the signed base"));
    }
    let mut temp = NamedTempFile::new_in(dir)?;
    let written = patch.apply(
        &source,
        Limited {
            file: temp.as_file_mut(),
            remaining: size,
        },
    )?;
    if written != size {
        return Err(invalid("incomplete output artifact"));
    }
    temp.seek(SeekFrom::Start(0))?;
    let mut sha = digest::Context::new(&digest::SHA256);
    let mut chunk = [0; 65536];
    loop {
        let n = temp.read(&mut chunk)?;
        if n == 0 {
            break;
        }
        sha.update(&chunk[..n]);
    }
    if sha.finish().as_ref() != new {
        return Err(invalid("updated artifact hash mismatch"));
    }
    temp.as_file().set_permissions(meta.permissions())?;
    temp.as_file().sync_all()?;
    temp.persist(installed)?;
    #[cfg(unix)]
    File::open(dir)?.sync_all()?;
    Ok(next)
}

/// Automatically applies an available delta from the application's own channel.
/// A channel returns `None` when the installed version is already current.
pub fn check<R: Read>(
    release: Release<'_>,
    public: &[u8],
    installed: &Path,
    fetch: impl FnOnce(Release<'_>) -> io::Result<Option<R>>,
) -> io::Result<Option<u64>> {
    fetch(release)?
        .map(|delta| apply(release, public, installed, delta))
        .transpose()
}
