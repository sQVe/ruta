use anyhow::Context as _;

use crate::USAGE;
use crate::modes;

#[derive(Debug, PartialEq)]
pub(crate) struct Window {
    pub(crate) title: String,
    pub(crate) width: u16,
    pub(crate) height: u16,
    pub(crate) geometry: Option<Geometry>,
    pub(crate) arguments: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Geometry {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
    pub(crate) scale: f64,
}

#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) window: Window,
    pub(crate) modes: modes::Modes,
    pub(crate) arguments: Vec<String>,
}

pub(crate) fn parse(arguments: Vec<String>) -> anyhow::Result<Options> {
    let mut window = parse_window(arguments.clone())?;
    let (modes, remaining) = modes::parse(&window.arguments)?;
    window.arguments = remaining;

    Ok(Options {
        window,
        modes,
        arguments,
    })
}

pub(crate) fn ready_line(window: &Window) -> serde_json::Value {
    let mut ready = serde_json::json!({
        "title": window.title,
        "width": window.width,
        "height": window.height,
        "arguments": window.arguments,
    });

    if let Some(geometry) = window.geometry {
        ready["origin"] = serde_json::json!({ "x": geometry.x, "y": geometry.y });
        ready["size"] = serde_json::json!({ "width": geometry.width, "height": geometry.height });
        ready["scale"] = serde_json::json!(geometry.scale);
    }

    ready
}

fn parse_window(arguments: Vec<String>) -> anyhow::Result<Window> {
    let [title, width, height, rest @ ..] = arguments.as_slice() else {
        anyhow::bail!(USAGE);
    };

    Ok(Window {
        title: title.clone(),
        width: width.parse().context("The width must be a whole number")?,
        height: height
            .parse()
            .context("The height must be a whole number")?,
        geometry: parse_geometry(rest)?,
        arguments: rest.to_vec(),
    })
}

fn parse_geometry(arguments: &[String]) -> anyhow::Result<Option<Geometry>> {
    let Some(flag) = arguments
        .iter()
        .position(|argument| argument == "--geometry")
    else {
        return Ok(None);
    };
    let numbers: Option<Vec<f64>> = arguments
        .get(flag + 1)
        .map(|value| value.split(',').map(finite_number).collect())
        .unwrap_or_default();
    let Some([x, y, width, height, scale]) = numbers.as_deref() else {
        anyhow::bail!(USAGE);
    };

    Ok(Some(Geometry {
        x: *x,
        y: *y,
        width: *width,
        height: *height,
        scale: *scale,
    }))
}

fn finite_number(text: &str) -> Option<f64> {
    text.parse().ok().filter(|number: &f64| number.is_finite())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn ready_line_without_geometry_keeps_its_shape() {
        let options = parse(strings(&["demo", "300", "200", "extra"])).expect("parse");

        assert_eq!(
            ready_line(&options.window),
            json!({ "title": "demo", "width": 300, "height": 200, "arguments": ["extra"] })
        );
    }

    #[test]
    fn geometry_adds_origin_size_and_scale() {
        let arguments = strings(&["demo", "300", "200", "--geometry", "10,20.5,100,50,2"]);
        let options = parse(arguments).expect("parse");

        assert_eq!(
            ready_line(&options.window),
            json!({
                "title": "demo",
                "width": 300,
                "height": 200,
                "arguments": ["--geometry", "10,20.5,100,50,2"],
                "origin": { "x": 10.0, "y": 20.5 },
                "size": { "width": 100.0, "height": 50.0 },
                "scale": 2.0,
            })
        );
    }

    #[test]
    fn geometry_without_a_good_value_fails_with_the_usage() {
        for value in [
            None,
            Some("10,20,100,50"),
            Some("10,20,100,50,2,3"),
            Some("10,twenty,100,50,2"),
        ] {
            let mut arguments = strings(&["demo", "300", "200", "--geometry"]);
            arguments.extend(value.map(ToString::to_string));

            let error = parse(arguments).expect_err("bad geometry");

            assert_eq!(error.to_string(), USAGE, "{value:?}");
        }
    }

    #[test]
    fn geometry_refuses_values_that_are_not_finite() {
        for bad in ["NaN", "inf", "-inf", "1e999"] {
            for field in 0..5 {
                let mut parts = ["10", "20", "100", "50", "2"];
                parts[field] = bad;
                let value = parts.join(",");
                let arguments = strings(&["demo", "300", "200", "--geometry", &value]);

                let error = parse(arguments).expect_err("not finite");

                assert_eq!(error.to_string(), USAGE, "{value}");
            }
        }
    }
}
