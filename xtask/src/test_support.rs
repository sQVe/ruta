use tempfile::{Builder, TempDir};

pub(crate) fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

pub(crate) fn temporary_directory() -> TempDir {
    Builder::new()
        .prefix("ruta-xtask-test-")
        .tempdir()
        .expect("create a temporary directory")
}
