use proptest::prelude::*;

use super::super::*;

proptest! {
    #![proptest_config(crate::test_support::property_config())]

    #[test]
    fn crop_fits_agrees_with_a_per_edge_oracle(
        x in prop_oneof![any::<i64>(), -2000i64..2000],
        y in prop_oneof![any::<i64>(), -2000i64..2000],
        width in prop_oneof![any::<i64>(), -2000i64..2000],
        height in prop_oneof![any::<i64>(), -2000i64..2000],
        shot_width in 0u32..2000,
        shot_height in 0u32..2000,
    ) {
        let crop = Crop { x, y, width, height };
        let shot = PixelSize { width: shot_width, height: shot_height };
        let starts_inside = i128::from(x) >= 0 && i128::from(y) >= 0;
        let fits_across = i128::from(x) + i128::from(width) <= i128::from(shot_width);
        let fits_down = i128::from(y) + i128::from(height) <= i128::from(shot_height);
        let oracle = starts_inside && fits_across && fits_down;

        prop_assert_eq!(
            crop_fits(crop, shot),
            oracle,
            "crop {:?}, shot {:?}",
            crop,
            shot
        );
    }

    #[test]
    fn crop_box_edges_lie_within_one_pixel_of_the_scaled_geometry(
        x in -10_000.0f64..10_000.0,
        y in -10_000.0f64..10_000.0,
        width in 0.001f64..10_000.0,
        height in 0.001f64..10_000.0,
        scale in 0.1f64..8.0,
    ) {
        let geometry = Geometry { x, y, width, height, scale };
        let crop = crop_box(geometry);
        let edges = [
            ("left", crop.x as f64, x * scale),
            ("top", crop.y as f64, y * scale),
            ("right", (crop.x + crop.width) as f64, (x + width) * scale),
            ("bottom", (crop.y + crop.height) as f64, (y + height) * scale),
        ];

        for (name, crop_edge, geometry_edge) in edges {
            // 1e-6 absorbs the float error of the comparison.
            prop_assert!(
                (crop_edge - geometry_edge).abs() <= 1.0 + 1e-6,
                "{name} edge {crop_edge} is not within one pixel of {geometry_edge} for {geometry:?}"
            );
        }

        prop_assert!(
            (crop.x as f64 - x * scale).abs() <= 0.5 + 1e-6,
            "the left edge {} is within half a pixel of {} for {geometry:?}",
            crop.x,
            x * scale
        );
        prop_assert!(
            (crop.y as f64 - y * scale).abs() <= 0.5 + 1e-6,
            "the top edge {} is within half a pixel of {} for {geometry:?}",
            crop.y,
            y * scale
        );
    }
}
