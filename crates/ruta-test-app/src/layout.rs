use x11rb::protocol::xproto::Rectangle;

pub(crate) struct Bounds {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
}

pub(crate) struct Control {
    pub(crate) name: &'static str,
    pub(crate) bounds: Bounds,
}

pub(crate) const BUTTONS: [Control; 3] = [
    Control {
        name: "alpha",
        bounds: Bounds {
            x: 20,
            y: 20,
            width: 120,
            height: 40,
        },
    },
    Control {
        name: "beta",
        bounds: Bounds {
            x: 20,
            y: 80,
            width: 120,
            height: 40,
        },
    },
    Control {
        name: "gamma",
        bounds: Bounds {
            x: 20,
            y: 140,
            width: 120,
            height: 40,
        },
    },
];

pub(crate) const TEXT_FIELD: Control = Control {
    name: "text",
    bounds: Bounds {
        x: 20,
        y: 200,
        width: 300,
        height: 40,
    },
};

pub(crate) fn hit_button(x: i32, y: i32) -> Option<&'static str> {
    BUTTONS
        .iter()
        .find(|button| contains(&button.bounds, x, y))
        .map(|button| button.name)
}

pub(crate) fn hit_text_field(x: i32, y: i32) -> bool {
    contains(&TEXT_FIELD.bounds, x, y)
}

fn contains(bounds: &Bounds, x: i32, y: i32) -> bool {
    x >= bounds.x && x < bounds.x + bounds.width && y >= bounds.y && y < bounds.y + bounds.height
}

pub(crate) fn drawn_rectangles() -> Vec<Rectangle> {
    BUTTONS
        .iter()
        .chain([&TEXT_FIELD])
        .map(|control| rectangle(&control.bounds))
        .collect()
}

fn rectangle(bounds: &Bounds) -> Rectangle {
    Rectangle {
        x: i16::try_from(bounds.x).expect("the control bounds are small constants"),
        y: i16::try_from(bounds.y).expect("the control bounds are small constants"),
        width: u16::try_from(bounds.width).expect("the control bounds are small constants"),
        height: u16::try_from(bounds.height).expect("the control bounds are small constants"),
    }
}

pub(crate) fn printable(keysym: u32) -> Option<char> {
    match keysym {
        0x20..=0x7e => char::from_u32(keysym),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rectangle_fields(rectangle: &Rectangle) -> (i16, i16, u16, u16) {
        (rectangle.x, rectangle.y, rectangle.width, rectangle.height)
    }

    fn bounds_fields(bounds: &Bounds) -> (i16, i16, u16, u16) {
        (
            i16::try_from(bounds.x).expect("a small coordinate"),
            i16::try_from(bounds.y).expect("a small coordinate"),
            u16::try_from(bounds.width).expect("a small size"),
            u16::try_from(bounds.height).expect("a small size"),
        )
    }

    #[test]
    fn hit_button_names_the_control_under_the_point() {
        for button in &BUTTONS {
            let bounds = &button.bounds;
            let centre = (bounds.x + bounds.width / 2, bounds.y + bounds.height / 2);

            assert_eq!(
                hit_button(centre.0, centre.1),
                Some(button.name),
                "{centre:?}"
            );
            assert_eq!(
                hit_button(bounds.x, bounds.y),
                Some(button.name),
                "the top-left corner of {}",
                button.name
            );
            assert_eq!(
                hit_button(bounds.x + bounds.width, bounds.y),
                None,
                "the far x edge of {}",
                button.name
            );
            assert_eq!(
                hit_button(bounds.x, bounds.y + bounds.height),
                None,
                "the far y edge of {}",
                button.name
            );
        }

        assert_eq!(hit_button(0, 0), None);
        assert_eq!(hit_button(400, 400), None);
    }

    #[test]
    fn controls_do_not_overlap() {
        let controls = BUTTONS.iter().chain([&TEXT_FIELD]).collect::<Vec<_>>();

        for (index, first) in controls.iter().enumerate() {
            for second in &controls[index + 1..] {
                let overlap_x = first.bounds.x < second.bounds.x + second.bounds.width
                    && second.bounds.x < first.bounds.x + first.bounds.width;
                let overlap_y = first.bounds.y < second.bounds.y + second.bounds.height
                    && second.bounds.y < first.bounds.y + first.bounds.height;

                assert!(
                    !(overlap_x && overlap_y),
                    "{} and {} share a point",
                    first.name,
                    second.name
                );
            }
        }
    }

    #[test]
    fn drawn_rectangles_match_the_control_bounds() {
        let drawn = drawn_rectangles()
            .iter()
            .map(rectangle_fields)
            .collect::<Vec<_>>();
        let expected = BUTTONS
            .iter()
            .chain([&TEXT_FIELD])
            .map(|control| bounds_fields(&control.bounds))
            .collect::<Vec<_>>();

        assert_eq!(drawn, expected);
    }

    #[test]
    fn printable_keeps_ascii_and_drops_other_keysyms() {
        assert_eq!(printable(0x20), Some(' '));
        assert_eq!(printable(0x7e), Some('~'));
        assert_eq!(printable(0x68), Some('h'));
        for keysym in [0x1f, 0x7f, 0xff0d, 0x0] {
            assert_eq!(printable(keysym), None, "{keysym:#x}");
        }
    }
}
