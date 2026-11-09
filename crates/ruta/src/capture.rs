use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::Context as _;
use serde_json::{Value, json};
use thiserror::Error;

use crate::command::require_program_in;
use crate::process::{TOOL_TIMEOUT, run_with_timeout};

const GEOMETRY_FIELDS: [&str; 3] = ["origin", "size", "scale"];
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const PNG_HEADER_LENGTH: usize = 24;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Geometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Crop {
    x: i64,
    y: i64,
    width: i64,
    height: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PixelSize {
    width: u32,
    height: u32,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum CaptureError {
    #[error("The ready line gives only part of the crop geometry; it lacks {}.", missing.join(", "))]
    PartialGeometry { missing: Vec<&'static str> },
    #[error("The ready line field {field} is not a number.")]
    NotANumber { field: &'static str },
    #[error("The ready line field {field} must be greater than zero.")]
    NotPositive { field: &'static str },
    #[error(
        "The crop {} falls outside the {}x{} shot.",
        crop_geometry(*crop),
        shot.width,
        shot.height
    )]
    CropOutside { crop: Crop, shot: PixelSize },
    #[error("The crop {} is empty.", crop_geometry(*crop))]
    EmptyCrop { crop: Crop },
    #[error("magick failed: {stderr}")]
    MagickFailed { stderr: String },
    #[error("The shot is too short to be a PNG.")]
    ShortPng,
    #[error("The shot is not a PNG.")]
    NotPng,
}

pub(crate) fn write_capture(
    folder: &Path,
    ready_line: Option<&str>,
    environment: &[(String, String)],
) -> anyhow::Result<()> {
    let ready: Option<Value> = ready_line
        .map(|line| serde_json::from_str(line).context("The saved ready line is not JSON"))
        .transpose()?;
    let geometry = match &ready {
        Some(ready) => parse_geometry(ready)?,
        None => None,
    };
    let shot_path = folder.join("shot.png");
    let shot = read_shot_size(&shot_path)?;

    if let Some(geometry) = geometry {
        let crop = crop_box(geometry);

        if crop.width == 0 || crop.height == 0 {
            return Err(CaptureError::EmptyCrop { crop }.into());
        }

        if !crop_fits(crop, shot) {
            return Err(CaptureError::CropOutside { crop, shot }.into());
        }

        crop_image(&shot_path, &folder.join("crop.png"), crop, environment)?;
    }

    let record = capture_record(shot, geometry, ready);
    let path = folder.join("capture.json");

    fs::write(&path, format!("{record:#}\n"))
        .with_context(|| format!("Could not write {}", path.display()))
}

fn parse_geometry(ready: &Value) -> Result<Option<Geometry>, CaptureError> {
    let missing: Vec<&'static str> = GEOMETRY_FIELDS
        .into_iter()
        .filter(|field| ready.get(field).is_none())
        .collect();

    if missing.len() == GEOMETRY_FIELDS.len() {
        return Ok(None);
    }

    if !missing.is_empty() {
        return Err(CaptureError::PartialGeometry { missing });
    }

    Ok(Some(Geometry {
        x: number(ready, "origin.x")?,
        y: number(ready, "origin.y")?,
        width: positive_number(ready, "size.width")?,
        height: positive_number(ready, "size.height")?,
        scale: positive_number(ready, "scale")?,
    }))
}

fn number(ready: &Value, field: &'static str) -> Result<f64, CaptureError> {
    let pointer = format!("/{}", field.replace('.', "/"));

    ready
        .pointer(&pointer)
        .and_then(Value::as_f64)
        .ok_or(CaptureError::NotANumber { field })
}

fn positive_number(ready: &Value, field: &'static str) -> Result<f64, CaptureError> {
    let value = number(ready, field)?;

    if value <= 0.0 {
        return Err(CaptureError::NotPositive { field });
    }

    Ok(value)
}

fn crop_box(geometry: Geometry) -> Crop {
    let pixels = |logical: f64| (logical * geometry.scale).round() as i64;

    Crop {
        x: pixels(geometry.x),
        y: pixels(geometry.y),
        width: pixels(geometry.width),
        height: pixels(geometry.height),
    }
}

fn crop_fits(crop: Crop, shot: PixelSize) -> bool {
    let starts_inside = crop.x >= 0 && crop.y >= 0;
    let fits_across = crop.x.saturating_add(crop.width) <= i64::from(shot.width);
    let fits_down = crop.y.saturating_add(crop.height) <= i64::from(shot.height);

    starts_inside && fits_across && fits_down
}

fn crop_geometry(crop: Crop) -> String {
    format!("{}x{}+{}+{}", crop.width, crop.height, crop.x, crop.y)
}

fn read_shot_size(path: &Path) -> anyhow::Result<PixelSize> {
    let bytes = fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;

    png_size(&bytes).with_context(|| format!("Could not read the size of {}", path.display()))
}

fn png_size(bytes: &[u8]) -> Result<PixelSize, CaptureError> {
    if bytes.len() < PNG_HEADER_LENGTH {
        return Err(CaptureError::ShortPng);
    }

    let has_signature = bytes.starts_with(PNG_SIGNATURE);
    let starts_with_header = &bytes[12..16] == b"IHDR";

    if !(has_signature && starts_with_header) {
        return Err(CaptureError::NotPng);
    }

    let width = u32::from_be_bytes(bytes[16..20].try_into().expect("the range is four bytes"));
    let height = u32::from_be_bytes(bytes[20..24].try_into().expect("the range is four bytes"));

    Ok(PixelSize { width, height })
}

fn crop_image(
    shot: &Path,
    crop_path: &Path,
    crop: Crop,
    environment: &[(String, String)],
) -> anyhow::Result<()> {
    let path = environment
        .iter()
        .find(|(name, _)| name == "PATH")
        .map_or("", |(_, value)| value.as_str());
    require_program_in("magick", path)?;

    let geometry = crop_geometry(crop);
    let mut command = Command::new("magick");

    command
        .arg(shot)
        .args(["-crop", &geometry, "+repage"])
        .arg(crop_path)
        .env_clear()
        .envs(environment.iter().cloned());

    let output = run_with_timeout(command, TOOL_TIMEOUT)?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return Err(CaptureError::MagickFailed { stderr }.into());
    }

    Ok(())
}

fn capture_record(shot: PixelSize, geometry: Option<Geometry>, ready: Option<Value>) -> Value {
    let mut record = json!({
        "shot": { "width": shot.width, "height": shot.height },
    });

    if let Some(geometry) = geometry {
        let crop = crop_box(geometry);

        record["crop"] = json!({
            "x": crop.x,
            "y": crop.y,
            "width": crop.width,
            "height": crop.height,
            "geometry": crop_geometry(crop),
        });
        record["scale"] = json!(geometry.scale);
    }

    if let Some(ready) = ready {
        record["ready"] = ready;
    }

    record
}

#[cfg(test)]
mod tests;
