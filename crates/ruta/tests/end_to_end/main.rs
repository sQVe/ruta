mod accessibility;
mod capture;
mod display;
mod escape;
mod input;
mod lifecycle;
mod modes;
#[path = "../../src/test_support/namespace.rs"]
mod namespace;

pub(crate) mod test_support {
    pub(crate) use super::namespace;
}
mod output;
mod preflight;
mod support;
mod version;
