use std::fs;
use std::io::{BufRead as _, BufReader};
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

use anyhow::Context as _;
use atspi::proxy::bus::{BusProxy, StatusProxy};
use futures_lite::future::block_on;
use tempfile::TempDir;

pub(crate) struct A11yBus {
    folder: TempDir,
    child: Child,
    session_address: String,
    accessibility_address: String,
}

impl A11yBus {
    pub(crate) fn start() -> anyhow::Result<Self> {
        let folder = tempfile::Builder::new()
            .prefix("ruta-a11y-bus-")
            .tempdir()
            .context("create the accessibility bus folder")?;
        let runtime = folder.path().join("runtime");
        fs::create_dir(&runtime).context("create the accessibility bus runtime folder")?;
        fs::set_permissions(&runtime, fs::Permissions::from_mode(0o700))
            .context("make the accessibility bus runtime folder private")?;
        let log = folder.path().join("dbus-daemon.log");
        let child = start_daemon(&folder.path().join("session-bus"), &runtime, &log)?;
        let mut bus = Self {
            folder,
            child,
            session_address: String::new(),
            accessibility_address: String::new(),
        };

        bus.session_address = read_address(&mut bus.child, &log)?;
        bus.accessibility_address = block_on(accessibility_address(&bus.session_address))?;

        Ok(bus)
    }

    pub(crate) fn enable(&self) -> anyhow::Result<()> {
        block_on(async {
            let connection = connect(&self.session_address).await?;
            let status = StatusProxy::new(&connection)
                .await
                .context("reach org.a11y.Status on the private session bus")?;

            status
                .set_is_enabled(true)
                .await
                .context("enable accessibility on the private session bus")
        })
    }

    pub(crate) fn session_address(&self) -> &str {
        &self.session_address
    }

    pub(crate) fn accessibility_address(&self) -> &str {
        &self.accessibility_address
    }

    pub(crate) fn socket_path(&self) -> PathBuf {
        self.folder.path().join("session-bus")
    }
}

impl Drop for A11yBus {
    #[expect(
        clippy::print_stderr,
        reason = "no caller can take a cleanup error in drop"
    )]
    fn drop(&mut self) {
        if let Err(error) = self.child.kill()
            && error.kind() != std::io::ErrorKind::InvalidInput
        {
            eprintln!("could not stop the private dbus-daemon: {error}");
        }

        if let Err(error) = self.child.wait() {
            eprintln!("could not wait for the private dbus-daemon: {error}");
        }
    }
}

fn start_daemon(socket: &Path, runtime: &Path, log: &Path) -> anyhow::Result<Child> {
    let stderr = fs::File::create(log).context("create the dbus-daemon log")?;
    let mut command = Command::new("dbus-daemon");

    command
        .args([
            "--session",
            "--nofork",
            "--print-address=1",
            &format!("--address=unix:path={}", socket.display()),
        ])
        .env("XDG_RUNTIME_DIR", runtime)
        .env("ATSPI_DBUS_IMPLEMENTATION", "dbus-daemon")
        .stdout(Stdio::piped())
        .stderr(Stdio::from(stderr));

    command
        .spawn()
        .with_context(|| format!("start {command:?} for {}", socket.display()))
}

fn read_address(child: &mut Child, log: &Path) -> anyhow::Result<String> {
    let stdout = child.stdout.take().expect("dbus-daemon stdout is piped");
    let mut address = String::new();

    BufReader::new(stdout)
        .read_line(&mut address)
        .context("read the private session bus address")?;

    let address = address.trim();

    if !address.is_empty() {
        return Ok(address.to_string());
    }

    let status = child
        .try_wait()
        .context("check whether dbus-daemon is still running")?;
    let output = fs::read_to_string(log).unwrap_or_default();

    anyhow::bail!("dbus-daemon printed no address (status {status:?}): {output}")
}

async fn accessibility_address(session_address: &str) -> anyhow::Result<String> {
    let connection = connect(session_address).await?;
    let bus = BusProxy::new(&connection)
        .await
        .context("reach org.a11y.Bus on the private session bus")?;

    bus.get_address()
        .await
        .context("start the accessibility bus launcher")
}

async fn connect(address: &str) -> anyhow::Result<zbus::Connection> {
    zbus::connection::Builder::address(address)
        .context("parse the private session bus address")?
        .build()
        .await
        .context("connect to the private session bus")
}
