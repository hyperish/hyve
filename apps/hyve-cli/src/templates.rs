//! `hyve init` templates (`docs/01-dx-and-dsl.md` §2). The files live in
//! `templates/<name>/` with a `.tmpl` suffix (so cargo does not take a
//! template's `Cargo.toml` for a package of its own) and placeholders:
//! `{{name}}` the package name, `{{crate}}` its Rust name, and the versions
//! the template was verified with.

/// The template names, the default first.
pub(crate) const TEMPLATES: [&str; 3] = ["fullstack", "serge-app", "orbic-service"];

/// The versions the templates pin, verified with this release of `hyve`
/// (`docs/01-dx-and-dsl.md` §2: the versions a template writes are the ones
/// tested with that CLI release).
pub(crate) const HYVE: &str = env!("CARGO_PKG_VERSION");
pub(crate) const ORBIC: &str = "0.1.0-alpha.1";
pub(crate) const SERGE: &str = "0.1.0-alpha.1";
/// serge's `wasm-bindgen`: the CLI that builds the page must match it.
pub(crate) const WASM_BINDGEN: &str = "0.2.130";

macro_rules! files {
    ($dir:literal: $($path:literal),* $(,)?) => {
        &[$(($path, include_str!(concat!("../templates/", $dir, "/", $path, ".tmpl")))),*]
    };
}

const FULLSTACK: &[(&str, &str)] = files!("fullstack":
    "Cargo.toml", "hyve.toml", "README.md", "gitignore", "migrations/0001_notes.sql",
    "src/lib.rs", "src/notes.rs", "src/main.rs", "src/bin/server.rs", "tests/notes.rs",
);
const SERGE_APP: &[(&str, &str)] = files!("serge-app":
    "Cargo.toml", "README.md", "gitignore", "src/lib.rs", "src/main.rs",
);
const ORBIC_SERVICE: &[(&str, &str)] = files!("orbic-service":
    "Cargo.toml", "hyve.toml", "README.md", "gitignore", "migrations/0001_notes.sql",
    "src/lib.rs", "src/main.rs",
);

/// The files of `template` for package `name`: (path, contents).
pub(crate) fn render(template: &str, name: &str) -> Vec<(String, String)> {
    let files = match template {
        "fullstack" => FULLSTACK,
        "serge-app" => SERGE_APP,
        "orbic-service" => ORBIC_SERVICE,
        other => unreachable!("unknown template `{other}`"),
    };
    let krate = name.replace('-', "_");
    files
        .iter()
        .map(|(path, text)| {
            let path = if *path == "gitignore" {
                ".gitignore".to_owned()
            } else {
                (*path).to_owned()
            };
            let text = text
                .replace("{{name}}", name)
                .replace("{{crate}}", &krate)
                .replace("{{hyve}}", HYVE)
                .replace("{{orbic}}", ORBIC)
                .replace("{{serge}}", SERGE)
                .replace("{{wasm_bindgen}}", WASM_BINDGEN);
            (path, text)
        })
        .collect()
}

/// Whether `name` can be a package name (and, with `-` as `_`, a crate
/// name): ASCII letters, digits, `-` and `_`, starting with a letter.
pub(crate) fn valid_name(name: &str) -> bool {
    const RESERVED: [&str; 9] = [
        "test",
        "core",
        "std",
        "alloc",
        "proc_macro",
        "self",
        "super",
        "crate",
        "hyve",
    ];
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !RESERVED.contains(&name.replace('-', "_").as_str())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_placeholder_is_filled() {
        for t in TEMPLATES {
            for (path, text) in render(t, "my-app") {
                assert!(!text.contains("{{"), "{t}/{path}: {text}");
                assert!(!path.ends_with(".tmpl"), "{t}/{path}");
            }
        }
    }

    #[test]
    fn the_package_and_crate_names_are_filled_in() {
        let files = render("orbic-service", "my-app");
        let manifest = &files.iter().find(|(p, _)| p == "Cargo.toml").unwrap().1;
        assert!(manifest.contains("name = \"my-app\""), "{manifest}");
        assert!(
            manifest.contains(&format!("hyve = \"={HYVE}\"")),
            "{manifest}"
        );
        let main = &files.iter().find(|(p, _)| p == "src/main.rs").unwrap().1;
        assert!(main.contains("my_app::app(db)"), "{main}");
        assert!(files.iter().any(|(p, _)| p == ".gitignore"));
    }

    #[test]
    fn names() {
        for ok in ["app", "my-app", "my_app2", "A1"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in [
            "", "1app", "-app", "my app", "my.app", "test", "std", "hyve", "é",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
