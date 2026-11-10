use serde_json::json;

use super::*;

const TINY_PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1,
    0x4d, 0x00, 0x00, 0x00, 0x16, 0x49, 0x44, 0x41, 0x54, 0x08, 0xd7, 0x63, 0xfc, 0xff, 0xff, 0x3f,
    0x03, 0x03, 0x03, 0x03, 0x03, 0x03, 0x13, 0x03, 0x0c, 0x00, 0x00, 0x36, 0x06, 0x03, 0x01, 0x97,
    0x0a, 0xd1, 0x64, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

fn geometry_ready() -> Value {
    json!({
        "title": "demo",
        "origin": { "x": 10.5, "y": 20 },
        "size": { "width": 300, "height": 200.25 },
        "scale": 2,
    })
}

fn without(mut ready: Value, fields: &[&str]) -> Value {
    let object = ready.as_object_mut().expect("the ready line is an object");

    for field in fields {
        object.remove(*field);
    }

    ready
}

fn folder_with_shot(shot: &[u8]) -> tempfile::TempDir {
    let folder = crate::test_support::temporary_directory();
    fs::write(folder.path().join("shot.png"), shot).expect("write the shot");

    folder
}

fn fake_magick(folder: &Path, script: &str) -> Vec<(String, String)> {
    let program = folder.join("magick");
    fs::write(&program, format!("#!/bin/sh\n{script}\n")).expect("write the fake magick");
    fs::set_permissions(
        &program,
        std::os::unix::fs::PermissionsExt::from_mode(0o755),
    )
    .expect("make the fake magick runnable");

    vec![("PATH".to_string(), folder.to_string_lossy().into_owned())]
}

fn read_record(folder: &Path) -> Value {
    let text = fs::read_to_string(folder.join("capture.json")).expect("read capture.json");

    serde_json::from_str(&text).expect("capture.json is JSON")
}

#[test]
fn geometry_without_scale_names_scale() {
    let ready = json!({
        "title": "demo",
        "origin": { "x": 10, "y": 20 },
        "size": { "width": 300, "height": 200 },
    });

    assert_eq!(
        parse_geometry(&ready),
        Err(CaptureError::PartialGeometry {
            missing: vec!["scale"]
        })
    );
}

#[test]
fn partial_geometry_names_every_missing_field() {
    let cases: [(&[&str], Vec<&str>); 6] = [
        (&["origin"], vec!["origin"]),
        (&["size"], vec!["size"]),
        (&["scale"], vec!["scale"]),
        (&["origin", "size"], vec!["origin", "size"]),
        (&["origin", "scale"], vec!["origin", "scale"]),
        (&["size", "scale"], vec!["size", "scale"]),
    ];

    for (removed, missing) in cases {
        let error = parse_geometry(&without(geometry_ready(), removed)).expect_err("partial");

        assert_eq!(
            error,
            CaptureError::PartialGeometry {
                missing: missing.clone()
            },
            "{removed:?}"
        );
        for field in missing {
            assert!(error.to_string().contains(field), "{removed:?}: {error}");
        }
    }
}

#[test]
fn no_geometry_gives_no_crop() {
    let ready = without(geometry_ready(), &["origin", "size", "scale"]);

    assert_eq!(parse_geometry(&ready), Ok(None));
}

#[test]
fn full_geometry_gives_a_crop_box_scaled_by_scale() {
    let geometry = parse_geometry(&geometry_ready())
        .expect("the geometry is valid")
        .expect("the ready line has geometry");

    assert_eq!(
        crop_box(geometry),
        Crop {
            x: 21,
            y: 40,
            width: 600,
            height: 401,
        }
    );
}

#[test]
fn geometry_fields_must_be_numbers() {
    let cases = [
        ("origin.x", json!({ "x": "10", "y": 20 }), "origin"),
        ("origin.y", json!({ "x": 10 }), "origin"),
        ("size.width", json!({ "width": null, "height": 2 }), "size"),
        ("size.height", json!({ "width": 2, "height": [] }), "size"),
        ("scale", json!("2"), "scale"),
        ("origin.x", json!(5), "origin"),
    ];

    for (field, value, key) in cases {
        let mut ready = geometry_ready();
        ready[key] = value;

        assert_eq!(
            parse_geometry(&ready),
            Err(CaptureError::NotANumber { field }),
            "{field}"
        );
    }
}

#[test]
fn size_and_scale_must_be_greater_than_zero() {
    let cases = [
        ("size.width", json!({ "width": 0, "height": 2 }), "size"),
        ("size.height", json!({ "width": 2, "height": -1 }), "size"),
        ("scale", json!(0), "scale"),
        ("scale", json!(-0.5), "scale"),
    ];

    for (field, value, key) in cases {
        let mut ready = geometry_ready();
        ready[key] = value;

        assert_eq!(
            parse_geometry(&ready),
            Err(CaptureError::NotPositive { field }),
            "{field}"
        );
    }
}

#[test]
fn origin_may_be_zero_or_negative() {
    let mut ready = geometry_ready();
    ready["origin"] = json!({ "x": 0, "y": -3 });

    let geometry = parse_geometry(&ready).expect("the geometry is valid");

    assert!(matches!(geometry, Some(Geometry { x, y, .. }) if x == 0.0 && y == -3.0));
}

#[test]
fn crop_fits_checks_every_edge() {
    let shot = PixelSize {
        width: 1400,
        height: 900,
    };
    let inside = Crop {
        x: 60,
        y: 60,
        width: 420,
        height: 300,
    };
    let cases = [
        ("inside", inside, true),
        (
            "at the corner",
            Crop {
                x: 0,
                y: 0,
                width: 1400,
                height: 900,
            },
            true,
        ),
        ("left", Crop { x: -1, ..inside }, false),
        ("top", Crop { y: -1, ..inside }, false),
        ("right", Crop { x: 981, ..inside }, false),
        ("bottom", Crop { y: 601, ..inside }, false),
        (
            "huge",
            Crop {
                width: i64::MAX,
                ..inside
            },
            false,
        ),
    ];

    for (name, crop, fits) in cases {
        assert_eq!(crop_fits(crop, shot), fits, "{name}");
    }
}

#[test]
fn crop_outside_error_holds_both_boxes() {
    let crop = Crop {
        x: 60,
        y: 60,
        width: 420,
        height: 300,
    };
    let shot = PixelSize {
        width: 100,
        height: 100,
    };

    assert_eq!(
        CaptureError::CropOutside { crop, shot }.to_string(),
        "The crop 420x300+60+60 falls outside the 100x100 shot."
    );
}

#[test]
fn png_size_reads_the_header() {
    assert_eq!(
        png_size(TINY_PNG),
        Ok(PixelSize {
            width: 3,
            height: 2,
        })
    );
}

#[test]
fn png_size_refuses_a_short_file() {
    assert_eq!(png_size(&TINY_PNG[..23]), Err(CaptureError::ShortPng));
}

#[test]
fn png_size_refuses_a_file_that_is_not_a_png() {
    let mut wrong_signature = TINY_PNG.to_vec();
    wrong_signature[1] = b'J';
    let mut wrong_chunk = TINY_PNG.to_vec();
    wrong_chunk[12] = b'X';

    assert_eq!(png_size(&wrong_signature), Err(CaptureError::NotPng));
    assert_eq!(png_size(&wrong_chunk), Err(CaptureError::NotPng));
}

#[test]
fn capture_record_with_a_crop() {
    let geometry = parse_geometry(&geometry_ready()).expect("the geometry is valid");
    let shot = PixelSize {
        width: 1400,
        height: 900,
    };

    assert_eq!(
        capture_record(shot, geometry, Some(geometry_ready())),
        json!({
            "shot": { "width": 1400, "height": 900 },
            "crop": {
                "x": 21,
                "y": 40,
                "width": 600,
                "height": 401,
                "geometry": "600x401+21+40",
            },
            "scale": 2.0,
            "ready": geometry_ready(),
        })
    );
}

#[test]
fn capture_record_without_a_crop() {
    let shot = PixelSize {
        width: 3,
        height: 2,
    };
    let ready = json!({ "title": "demo" });

    assert_eq!(
        capture_record(shot, None, Some(ready.clone())),
        json!({ "shot": { "width": 3, "height": 2 }, "ready": ready })
    );
    assert_eq!(
        capture_record(shot, None, None),
        json!({ "shot": { "width": 3, "height": 2 } })
    );
}

#[test]
fn write_capture_without_a_ready_line_records_only_the_shot() {
    let folder = folder_with_shot(TINY_PNG);

    write_capture(folder.path(), None, &[]).expect("capture");

    assert_eq!(
        read_record(folder.path()),
        json!({ "shot": { "width": 3, "height": 2 } })
    );
    assert!(!folder.path().join("crop.png").exists());
}

#[test]
fn write_capture_keeps_a_ready_line_without_geometry() {
    let folder = folder_with_shot(TINY_PNG);

    write_capture(folder.path(), Some("{\"title\":\"demo\"}\n"), &[]).expect("capture");

    assert_eq!(
        read_record(folder.path()),
        json!({ "shot": { "width": 3, "height": 2 }, "ready": { "title": "demo" } })
    );
    assert!(!folder.path().join("crop.png").exists());
}

#[test]
fn write_capture_refuses_a_crop_outside_the_shot() {
    let folder = folder_with_shot(TINY_PNG);
    let ready = geometry_ready().to_string();

    let error = write_capture(folder.path(), Some(&ready), &[]).expect_err("outside");

    assert_eq!(
        error.downcast_ref::<CaptureError>(),
        Some(&CaptureError::CropOutside {
            crop: Crop {
                x: 21,
                y: 40,
                width: 600,
                height: 401,
            },
            shot: PixelSize {
                width: 3,
                height: 2,
            },
        })
    );
    assert!(!folder.path().join("crop.png").exists());
    assert!(!folder.path().join("capture.json").exists());
}

#[test]
fn write_capture_refuses_partial_geometry_before_writing() {
    let folder = folder_with_shot(TINY_PNG);
    let ready = without(geometry_ready(), &["scale"]).to_string();

    let error = write_capture(folder.path(), Some(&ready), &[]).expect_err("partial");

    assert_eq!(
        error.downcast_ref::<CaptureError>(),
        Some(&CaptureError::PartialGeometry {
            missing: vec!["scale"]
        })
    );
    assert!(!folder.path().join("capture.json").exists());
}

#[test]
fn write_capture_refuses_a_shot_that_is_not_a_png() {
    let folder = folder_with_shot(b"not a png, but long enough to hold a header");

    let error = write_capture(folder.path(), None, &[]).expect_err("not a PNG");

    assert_eq!(
        error.downcast_ref::<CaptureError>(),
        Some(&CaptureError::NotPng)
    );
    assert!(!folder.path().join("capture.json").exists());
}

#[test]
fn write_capture_refuses_a_crop_that_rounds_to_zero_pixels() {
    let tools = crate::test_support::temporary_directory();
    let environment = fake_magick(tools.path(), "printf '%s\\n' \"$@\" > \"$5\"");
    let cases = [
        ("zero width", json!({ "width": 0.4, "height": 1 }), (0, 1)),
        ("zero height", json!({ "width": 1, "height": 0.4 }), (1, 0)),
    ];

    for (name, size, (width, height)) in cases {
        let folder = folder_with_shot(TINY_PNG);
        let mut ready = geometry_ready();
        ready["origin"] = json!({ "x": 0, "y": 0 });
        ready["size"] = size;
        ready["scale"] = json!(1);

        let error = write_capture(folder.path(), Some(&ready.to_string()), &environment)
            .expect_err("empty crop");
        let crop = Crop {
            x: 0,
            y: 0,
            width,
            height,
        };

        assert_eq!(
            error.downcast_ref::<CaptureError>(),
            Some(&CaptureError::EmptyCrop { crop }),
            "{name}"
        );
        assert!(
            error.to_string().contains(&crop_geometry(crop)),
            "{name}: {error}"
        );
        assert!(!folder.path().join("crop.png").exists(), "{name}");
        assert!(!folder.path().join("capture.json").exists(), "{name}");
    }
}

#[test]
fn write_capture_crops_with_magick_and_records_the_crop() {
    let tools = crate::test_support::temporary_directory();
    let environment = fake_magick(tools.path(), "printf '%s\\n' \"$@\" > \"$5\"");
    let folder = folder_with_shot(TINY_PNG);
    let mut ready = geometry_ready();
    ready["origin"] = json!({ "x": 1, "y": 0 });
    ready["size"] = json!({ "width": 2, "height": 1 });
    ready["scale"] = json!(1);

    write_capture(folder.path(), Some(&ready.to_string()), &environment).expect("capture");

    let shot = folder.path().join("shot.png");
    let crop = folder.path().join("crop.png");
    let arguments = fs::read_to_string(&crop).expect("read crop.png");
    assert_eq!(
        arguments,
        format!(
            "{}\n-crop\n2x1+1+0\n+repage\n{}\n",
            shot.display(),
            crop.display()
        )
    );
    assert_eq!(read_record(folder.path())["crop"]["geometry"], "2x1+1+0");
}

#[test]
fn write_capture_reports_a_failed_magick_and_writes_no_record() {
    let tools = crate::test_support::temporary_directory();
    let environment = fake_magick(tools.path(), "echo 'magick: no crop' >&2\nexit 3");
    let folder = folder_with_shot(TINY_PNG);
    let mut ready = geometry_ready();
    ready["origin"] = json!({ "x": 0, "y": 0 });
    ready["size"] = json!({ "width": 1, "height": 1 });
    ready["scale"] = json!(1);

    let error = write_capture(folder.path(), Some(&ready.to_string()), &environment)
        .expect_err("magick failed");

    assert_eq!(
        error.downcast_ref::<CaptureError>(),
        Some(&CaptureError::MagickFailed {
            stderr: "magick: no crop".to_string()
        })
    );
    assert!(!folder.path().join("capture.json").exists());
}

#[test]
fn write_capture_looks_for_magick_on_the_given_path() {
    let empty = crate::test_support::temporary_directory();
    let environment = vec![(
        "PATH".to_string(),
        empty.path().to_string_lossy().into_owned(),
    )];
    let folder = folder_with_shot(TINY_PNG);
    let mut ready = geometry_ready();
    ready["origin"] = json!({ "x": 0, "y": 0 });
    ready["size"] = json!({ "width": 1, "height": 1 });
    ready["scale"] = json!(1);

    let error = write_capture(folder.path(), Some(&ready.to_string()), &environment)
        .expect_err("no magick");

    assert!(matches!(
        error.downcast_ref::<crate::tool::MissingTool>(),
        Some(crate::tool::MissingTool { tool: Tool::Magick })
    ));
    assert!(!folder.path().join("capture.json").exists());
}
