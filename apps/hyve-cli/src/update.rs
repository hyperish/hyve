use crate::Fail;
use hyve_update::Release;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::Path,
};
use tempfile::NamedTempFile;
const USAGE: &str = "hyve update keygen PRIVATE.pk8 PUBLIC.key\nhyve update create APP TARGET FROM TO OLD NEW PRIVATE.pk8 DELTA\nhyve update apply APP TARGET INSTALLED_VERSION ARTIFACT PUBLIC.key DELTA";

fn create_file(path: &Path, write: impl FnOnce(&mut File) -> io::Result<()>) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let mut file = NamedTempFile::new_in(parent)?;
    write(file.as_file_mut())?;
    file.as_file().sync_all()?;
    file.persist_noclobber(path)?;
    Ok(())
}
fn number(s: &str) -> io::Result<u64> {
    s.parse()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "expected a release number"))
}

pub(super) fn run(args: &[String]) -> Result<(), Fail> {
    let args: Vec<_> = args.iter().map(String::as_str).collect();
    let outcome = || -> io::Result<()> {
        match args.as_slice() {
            ["-h" | "--help" | "help"] => println!("{USAGE}"),
            ["keygen", private, public] => {
                let public_path = Path::new(public);
                let private_path = Path::new(private);
                if public_path == private_path {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "use separate key files",
                    ));
                }
                if !private_path.exists() {
                    if public_path.exists() {
                        return Err(io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "public key already exists",
                        ));
                    }
                    let (secret, _) = hyve_update::generate_key()?;
                    match create_file(private_path, |f| f.write_all(&secret)) {
                        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
                        result => result?,
                    }
                }
                let key = hyve_update::public_key(&fs::read(private_path)?)?;
                match create_file(public_path, |f| f.write_all(&key)) {
                    Err(e)
                        if e.kind() == io::ErrorKind::AlreadyExists
                            && fs::read(public_path)? == key => {}
                    result => result?,
                }
            }
            ["create", app, target, from, to, old, new, private, delta] => {
                let release = Release {
                    app,
                    target,
                    number: number(from)?,
                };
                create_file(Path::new(delta), |file| {
                    hyve_update::create(
                        release,
                        number(to)?,
                        File::open(old)?,
                        File::open(new)?,
                        &fs::read(private)?,
                        file,
                    )
                })?;
            }
            ["apply", app, target, version, installed, public, delta] => {
                let release = Release {
                    app,
                    target,
                    number: number(version)?,
                };
                let updated = hyve_update::apply(
                    release,
                    &fs::read(public)?,
                    Path::new(installed),
                    File::open(delta)?,
                )?;
                println!("installed release {updated}");
            }
            _ => return Err(io::Error::new(io::ErrorKind::InvalidInput, USAGE)),
        }
        Ok(())
    };
    outcome().map_err(|e| {
        if e.kind() == io::ErrorKind::InvalidInput {
            Fail::Usage(e.to_string(), None)
        } else {
            Fail::Failed(e.to_string(), None)
        }
    })
}
