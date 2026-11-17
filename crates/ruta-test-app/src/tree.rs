use std::sync::{Arc, Mutex};

use accesskit::{
    ActionHandler, ActionRequest, ActivationHandler, DeactivationHandler, Node, NodeId, Rect, Role,
    TreeId, TreeInfo, TreeUpdate,
};
use accesskit_unix::Adapter;

use crate::layout;

const WINDOW: NodeId = NodeId(0);
const TEXT_FIELD: NodeId = NodeId(4);
const TEXT_RUN: NodeId = NodeId(5);

#[derive(Default)]
struct State {
    focused: bool,
    text: String,
}

impl State {
    fn press(&mut self) {
        self.focused = false;
    }

    fn focus_text_field(&mut self) {
        self.focused = true;
    }

    fn type_character(&mut self, keysym: u32) -> Option<char> {
        if !self.focused {
            return None;
        }

        let character = layout::printable(keysym)?;
        self.text.push(character);

        Some(character)
    }

    fn text(&self) -> &str {
        &self.text
    }
}

pub(crate) struct Tree {
    state: Arc<Mutex<State>>,
    title: String,
    size: (u16, u16),
    adapter: Adapter,
}

impl Tree {
    pub(crate) fn new(title: &str, size: (u16, u16)) -> Self {
        let state = Arc::new(Mutex::new(State::default()));
        let initial_update = {
            let state = state
                .lock()
                .expect("the accessibility state is not poisoned");

            update(&state, title, size, Some(TreeInfo::new(WINDOW)))
        };
        let activation = Activation {
            state: Arc::clone(&state),
            title: title.to_string(),
            size,
        };
        let mut adapter = Adapter::new(activation, NoActions, NoDeactivation);

        adapter.set_root_window_bounds(window_rect(size), window_rect(size));
        adapter.update_if_active(|| initial_update);

        Self {
            state,
            title: title.to_string(),
            size,
            adapter,
        }
    }

    pub(crate) fn press(&mut self) {
        self.change(State::press);
    }

    pub(crate) fn focus_text_field(&mut self) {
        self.change(State::focus_text_field);
    }

    pub(crate) fn type_character(&mut self, keysym: u32) -> bool {
        let mut appended = None;

        self.change(|state| appended = state.type_character(keysym));

        appended.is_some()
    }

    pub(crate) fn text(&self) -> String {
        let state = self
            .state
            .lock()
            .expect("the accessibility state is not poisoned");

        state.text().to_string()
    }

    fn change(&mut self, change: impl FnOnce(&mut State)) {
        let update = {
            let mut state = self
                .state
                .lock()
                .expect("the accessibility state is not poisoned");

            change(&mut state);
            update(&state, &self.title, self.size, None)
        };

        self.adapter.update_if_active(|| update);
    }
}

struct Activation {
    state: Arc<Mutex<State>>,
    title: String,
    size: (u16, u16),
}

impl ActivationHandler for Activation {
    fn request_initial_tree(&mut self) -> Option<TreeUpdate> {
        let state = self
            .state
            .lock()
            .expect("the accessibility state is not poisoned");

        Some(update(
            &state,
            &self.title,
            self.size,
            Some(TreeInfo::new(WINDOW)),
        ))
    }
}

struct NoActions;

impl ActionHandler for NoActions {
    fn do_action(&mut self, _request: ActionRequest) {}
}

struct NoDeactivation;

impl DeactivationHandler for NoDeactivation {
    fn deactivate_accessibility(&mut self) {}
}

fn update(state: &State, title: &str, size: (u16, u16), tree: Option<TreeInfo>) -> TreeUpdate {
    let mut root = Node::new(Role::Window);
    root.set_label(title);
    root.set_bounds(window_rect(size));
    root.set_children([NodeId(1), NodeId(2), NodeId(3), NodeId(4)]);

    let mut nodes = vec![(WINDOW, root)];

    for (index, button) in layout::BUTTONS.iter().enumerate() {
        let mut node = Node::new(Role::Button);
        node.set_label(button.name);
        node.set_bounds(control_rect(button));
        nodes.push((NodeId(index as u64 + 1), node));
    }

    let mut text_field = Node::new(Role::TextInput);
    text_field.set_label(layout::TEXT_FIELD.name);
    text_field.set_bounds(control_rect(&layout::TEXT_FIELD));
    text_field.set_children([TEXT_RUN]);
    nodes.push((TEXT_FIELD, text_field));

    let mut text_run = Node::new(Role::TextRun);
    text_run.set_value(state.text());
    text_run.set_character_lengths(character_lengths(state.text()));
    nodes.push((TEXT_RUN, text_run));

    TreeUpdate {
        nodes,
        tree,
        tree_id: TreeId::ROOT,
        focus: if state.focused { TEXT_FIELD } else { WINDOW },
    }
}

fn window_rect(size: (u16, u16)) -> Rect {
    Rect {
        x0: 0.0,
        y0: 0.0,
        x1: f64::from(size.0),
        y1: f64::from(size.1),
    }
}

fn control_rect(control: &layout::Control) -> Rect {
    let bounds = &control.bounds;

    Rect {
        x0: f64::from(bounds.x),
        y0: f64::from(bounds.y),
        x1: f64::from(bounds.x + bounds.width),
        y1: f64::from(bounds.y + bounds.height),
    }
}

fn character_lengths(text: &str) -> Vec<u8> {
    text.chars()
        .map(|character| {
            u8::try_from(character.len_utf8()).expect("a character takes at most four UTF-8 bytes")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_appends_only_while_the_text_field_is_focused() {
        let mut state = State::default();

        state.type_character(0x68);
        assert_eq!(state.text(), "");

        state.focus_text_field();
        state.type_character(0x68);
        state.type_character(0x69);
        assert_eq!(state.text(), "hi");

        state.press();
        state.type_character(0x21);
        assert_eq!(state.text(), "hi");
    }

    #[test]
    fn the_text_field_has_a_text_run_with_its_value() {
        let mut state = State::default();
        state.focus_text_field();
        state.type_character(0x68);

        let update = update(&state, "demo", (640, 480), None);
        let text_run = update
            .nodes
            .iter()
            .find(|(id, _)| *id == TEXT_RUN)
            .map(|(_, node)| node)
            .expect("the update holds the text run");

        assert_eq!(text_run.role(), Role::TextRun);
        assert_eq!(text_run.value(), Some("h"));
        assert_eq!(text_run.character_lengths(), [1]);
    }

    #[test]
    fn window_rect_matches_the_window_size() {
        let rect = window_rect((640, 480));

        assert_eq!(
            (rect.x0, rect.y0, rect.x1, rect.y1),
            (0.0, 0.0, 640.0, 480.0)
        );
    }

    #[test]
    fn control_rects_cover_each_control_from_its_origin() {
        let controls = layout::BUTTONS.iter().chain([&layout::TEXT_FIELD]);

        for control in controls {
            let bounds = &control.bounds;
            let rect = control_rect(control);
            let expected = (
                f64::from(bounds.x),
                f64::from(bounds.y),
                f64::from(bounds.x) + f64::from(bounds.width),
                f64::from(bounds.y) + f64::from(bounds.height),
            );

            assert_eq!(
                (rect.x0, rect.y0, rect.x1, rect.y1),
                expected,
                "{}",
                control.name
            );
        }
    }

    #[test]
    fn each_button_node_uses_its_layout_index_as_id() {
        let state = State::default();
        let update = update(&state, "demo", (640, 480), None);

        for (index, button) in layout::BUTTONS.iter().enumerate() {
            let id = NodeId(index as u64 + 1);
            let label = update
                .nodes
                .iter()
                .find(|(node_id, _)| *node_id == id)
                .and_then(|(_, node)| node.label());

            assert_eq!(label, Some(button.name), "{}", button.name);
        }
    }

    #[test]
    fn character_lengths_counts_utf8_bytes() {
        assert_eq!(character_lengths("hi"), [1, 1]);
        assert_eq!(character_lengths("é"), [2]);
        assert_eq!(character_lengths(""), Vec::<u8>::new());
    }
}
