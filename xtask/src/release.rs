use std::path::{Path, PathBuf};
use std::process::ExitCode;

use thiserror::Error;

#[derive(Debug, Error)]
enum ValidationError {
    #[error("Usage: cargo xtask release-check <tag>")]
    Arguments { arguments: Vec<String> },
    #[error("Release tag {tag} must start with v")]
    MissingPrefix { tag: String },
    #[error("Could not read {}", path.display())]
    ReadManifest {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Invalid ruta manifest")]
    InvalidManifest { source: toml::de::Error },
    #[error("Ruta manifest has no package.version string")]
    MissingVersion,
    #[error("Release tag {tag} differs from crate version {version}")]
    VersionMismatch { tag: String, version: String },
}

#[expect(
    clippy::print_stderr,
    reason = "release checks report their result on stderr"
)]
pub(crate) fn run(root: &Path, arguments: &[String]) -> ExitCode {
    match check(root, arguments) {
        Ok(tag) => {
            eprintln!("Release tag {tag} matches the crate version.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            let error = anyhow::Error::from(error);
            eprintln!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

fn check<'a>(root: &Path, arguments: &'a [String]) -> Result<&'a str, ValidationError> {
    let [tag] = arguments else {
        return Err(ValidationError::Arguments {
            arguments: arguments.to_vec(),
        });
    };
    if !tag.starts_with('v') {
        return Err(ValidationError::MissingPrefix { tag: tag.clone() });
    }

    let manifest = root.join("crates/ruta/Cargo.toml");
    let text =
        std::fs::read_to_string(&manifest).map_err(|source| ValidationError::ReadManifest {
            path: manifest,
            source,
        })?;
    let manifest: toml::Table =
        toml::from_str(&text).map_err(|source| ValidationError::InvalidManifest { source })?;
    let version = manifest
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .ok_or(ValidationError::MissingVersion)?;
    if tag != &format!("v{version}") {
        return Err(ValidationError::VersionMismatch {
            tag: tag.clone(),
            version: version.to_string(),
        });
    }

    Ok(tag)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{strings, temporary_directory};

    #[test]
    fn release_reports_each_invalid_manifest() {
        let root = temporary_directory();
        let arguments = strings(&["v1.2.3"]);
        let error = check(root.path(), &arguments).expect_err("missing manifest");
        assert!(
            matches!(error, ValidationError::ReadManifest { path, source }
            if path == root.path().join("crates/ruta/Cargo.toml")
                && source.kind() == std::io::ErrorKind::NotFound)
        );
        assert_eq!(run(root.path(), &arguments), ExitCode::FAILURE);

        let directory = root.path().join("crates/ruta");
        std::fs::create_dir_all(&directory).expect("create manifest folder");
        std::fs::write(directory.join("Cargo.toml"), "[").expect("write manifest");
        let error = check(root.path(), &arguments).expect_err("invalid TOML");
        assert!(matches!(error, ValidationError::InvalidManifest { source }
            if source.span() == Some(1..1)));
        assert_eq!(run(root.path(), &arguments), ExitCode::FAILURE);

        for text in ["", "[package]\nname = 'ruta'", "[package]\nversion = 123"] {
            std::fs::write(directory.join("Cargo.toml"), text).expect("write manifest");
            let error = check(root.path(), &arguments).expect_err("invalid manifest");
            assert!(matches!(error, ValidationError::MissingVersion), "{text}");
            assert_eq!(run(root.path(), &arguments), ExitCode::FAILURE);
        }
    }

    #[test]
    fn release_checks_tag_against_manifest() {
        let root = temporary_directory();
        let directory = root.path().join("crates/ruta");
        std::fs::create_dir_all(&directory).expect("create manifest folder");
        let manifest = directory.join("Cargo.toml");
        std::fs::write(&manifest, "[package]\nversion = '1.2.3'\n").expect("write manifest");

        assert_eq!(run(root.path(), &strings(&["v1.2.3"])), ExitCode::SUCCESS);
        let error = check(root.path(), &strings(&["v1.2.2"])).expect_err("version mismatch");
        assert!(
            matches!(error, ValidationError::VersionMismatch { tag, version }
            if tag == "v1.2.2" && version == "1.2.3")
        );
        assert_eq!(run(root.path(), &strings(&["v1.2.2"])), ExitCode::FAILURE);

        let error = check(root.path(), &strings(&["1.2.3"])).expect_err("missing prefix");
        assert!(matches!(error, ValidationError::MissingPrefix { tag } if tag == "1.2.3"));
        assert_eq!(run(root.path(), &strings(&["1.2.3"])), ExitCode::FAILURE);

        for arguments in [&[][..], &["v1.2.3", "extra"][..]] {
            let arguments = strings(arguments);
            let error = check(root.path(), &arguments).expect_err("invalid tag arguments");
            assert!(
                matches!(error, ValidationError::Arguments { arguments: actual }
                if actual == arguments)
            );
            assert_eq!(
                run(root.path(), &arguments),
                ExitCode::FAILURE,
                "{arguments:?}"
            );
        }
    }
}
