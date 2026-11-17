use std::time::Duration;

use anyhow::Context as _;
use atspi::proxy::accessible::AccessibleProxy;
use atspi::proxy::component::ComponentProxy;
use atspi::proxy::text::TextProxy;
use atspi::{AccessibilityConnection, CoordType, ObjectRefOwned, Role};
use futures_lite::future::block_on;

use crate::support::poll_until;

pub(crate) struct Node {
    pub(crate) role: Role,
    pub(crate) name: String,
    pub(crate) bounds: Bounds,
    pub(crate) text: Option<String>,
}

pub(crate) struct Bounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

pub(crate) fn read(address: &str, app_name: &str) -> anyhow::Result<Vec<Node>> {
    block_on(read_async(address, app_name))
}

pub(crate) fn wait_for_control(address: &str, app_name: &str, name: &str) -> Node {
    let mut found = None;
    let mut last_error = None;

    poll_until(
        Duration::from_secs(30),
        Duration::from_millis(200),
        || match read(address, app_name) {
            Ok(nodes) => {
                found = nodes.into_iter().find(|node| node.name == name);

                found.is_some()
            }
            Err(error) => {
                last_error = Some(error);

                false
            }
        },
    );

    found.unwrap_or_else(|| {
        panic!(
            "no control named {name} appeared in {app_name}: {}",
            last_error.map_or_else(
                || "no node had that name".to_string(),
                |error| format!("{error:#}")
            )
        )
    })
}

async fn read_async(address: &str, app_name: &str) -> anyhow::Result<Vec<Node>> {
    let address = address
        .parse::<zbus::Address>()
        .context("parse the accessibility bus address")?;
    let connection = AccessibilityConnection::from_address(address)
        .await
        .context("connect to the accessibility bus")?;
    let registry = connection
        .root_accessible_on_registry()
        .await
        .context("read the accessibility registry")?;
    let application = application(connection.connection(), &registry, app_name).await?;
    let mut pending = application
        .get_children()
        .await
        .with_context(|| format!("read the children of {app_name}"))?;
    let mut nodes = Vec::new();

    while let Some(object) = pending.pop() {
        let accessible = accessible(connection.connection(), &object).await?;
        let role = accessible.get_role().await.context("read a node's role")?;
        let name = accessible.name().await.context("read a node's name")?;
        // AccessKit keeps a text input's value in a TextRun child, which get_text reads below.
        if role != Role::Entry {
            pending.extend(accessible.get_children().await?);
        }

        let (x, y, width, height) = component(connection.connection(), &object)
            .await?
            .get_extents(CoordType::Window)
            .await
            .with_context(|| format!("read the bounds of {name}"))?;
        let text = if role == Role::Entry {
            Some(
                text(connection.connection(), &object)
                    .await?
                    .get_text(0, -1)
                    .await
                    .with_context(|| format!("read the text of {name}"))?,
            )
        } else {
            None
        };

        nodes.push(Node {
            role,
            name,
            bounds: Bounds {
                x,
                y,
                width,
                height,
            },
            text,
        });
    }

    Ok(nodes)
}

async fn application(
    connection: &zbus::Connection,
    registry: &AccessibleProxy<'_>,
    app_name: &str,
) -> anyhow::Result<AccessibleProxy<'static>> {
    for object in registry
        .get_children()
        .await
        .context("read the registered applications")?
    {
        let candidate = accessible(connection, &object).await?;
        let name = candidate
            .name()
            .await
            .context("read an application's name")?;

        if name == app_name {
            return Ok(candidate);
        }
    }

    anyhow::bail!("the accessibility registry has no application named {app_name}")
}

async fn accessible(
    connection: &zbus::Connection,
    object: &ObjectRefOwned,
) -> anyhow::Result<AccessibleProxy<'static>> {
    let destination = object
        .name()
        .context("an accessible object has a bus name")?;

    AccessibleProxy::builder(connection)
        .destination(destination.to_string())?
        .path(object.path().to_string())?
        .build()
        .await
        .context("reach an accessible object")
}

async fn component(
    connection: &zbus::Connection,
    object: &ObjectRefOwned,
) -> anyhow::Result<ComponentProxy<'static>> {
    let destination = object
        .name()
        .context("an accessible object has a bus name")?;

    ComponentProxy::builder(connection)
        .destination(destination.to_string())?
        .path(object.path().to_string())?
        .build()
        .await
        .context("reach a component")
}

async fn text(
    connection: &zbus::Connection,
    object: &ObjectRefOwned,
) -> anyhow::Result<TextProxy<'static>> {
    let destination = object
        .name()
        .context("an accessible object has a bus name")?;

    TextProxy::builder(connection)
        .destination(destination.to_string())?
        .path(object.path().to_string())?
        .build()
        .await
        .context("reach a text field")
}
