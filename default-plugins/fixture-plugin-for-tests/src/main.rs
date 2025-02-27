use serde::{Deserialize, Serialize};
use zellij_tile::prelude::*;

// This is a fixture plugin used only for tests in Zellij
// it is not (and should not!) be included in the mainline executable
// it's included here for convenience so that it will be built by the CI

#[derive(Default)]
struct State {
    received_events: Vec<Event>,
    received_payload: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct TestWorker {
    number_of_messages_received: usize,
}

impl<'de> ZellijWorker<'de> for TestWorker {
    fn on_message(&mut self, message: String, payload: String) {
        if message == "ping" {
            self.number_of_messages_received += 1;
            post_message_to_plugin(
                "pong".into(),
                format!(
                    "{}, received {} messages",
                    payload, self.number_of_messages_received
                ),
            );
        }
    }
}

register_plugin!(State);
register_worker!(TestWorker, test_worker, TEST_WORKER);

impl ZellijPlugin for State {
    fn load(&mut self) {
        subscribe(&[
            EventType::InputReceived,
            EventType::Key,
            EventType::SystemClipboardFailure,
            EventType::CustomMessage,
            EventType::FileSystemCreate,
            EventType::FileSystemUpdate,
            EventType::FileSystemDelete,
        ]);
    }

    fn update(&mut self, event: Event) -> bool {
        match &event {
            Event::Key(key) => match key {
                Key::Char('a') => {
                    switch_to_input_mode(&InputMode::Tab);
                },
                Key::Char('b') => {
                    new_tabs_with_layout(
                        "layout {
                        tab {
                            pane
                            pane
                        }
                        tab split_direction=\"vertical\" {
                            pane
                            pane
                        }
                    }",
                    );
                },
                Key::Char('c') => new_tab(),
                Key::Char('d') => go_to_next_tab(),
                Key::Char('e') => go_to_previous_tab(),
                Key::Char('f') => {
                    let resize = Resize::Increase;
                    resize_focused_pane(resize)
                },
                Key::Char('g') => {
                    let resize = Resize::Increase;
                    let direction = Direction::Left;
                    resize_focused_pane_with_direction(resize, direction);
                },
                Key::Char('h') => focus_next_pane(),
                Key::Char('i') => focus_previous_pane(),
                Key::Char('j') => {
                    let direction = Direction::Left;
                    move_focus(direction)
                },
                Key::Char('k') => {
                    let direction = Direction::Left;
                    move_focus_or_tab(direction)
                },
                Key::Char('l') => detach(),
                Key::Char('m') => edit_scrollback(),
                Key::Char('n') => {
                    let bytes = vec![102, 111, 111];
                    write(bytes)
                },
                Key::Char('o') => {
                    let chars = "foo";
                    write_chars(chars);
                },
                Key::Char('p') => toggle_tab(),
                Key::Char('q') => move_pane(),
                Key::Char('r') => {
                    let direction = Direction::Left;
                    move_pane_with_direction(direction)
                },
                Key::Char('s') => clear_screen(),
                Key::Char('t') => scroll_up(),
                Key::Char('u') => scroll_down(),
                Key::Char('v') => scroll_to_top(),
                Key::Char('w') => scroll_to_bottom(),
                Key::Char('x') => page_scroll_up(),
                Key::Char('y') => page_scroll_down(),
                Key::Char('z') => toggle_focus_fullscreen(),
                Key::Char('1') => toggle_pane_frames(),
                Key::Char('2') => toggle_pane_embed_or_eject(),
                Key::Char('3') => undo_rename_pane(),
                Key::Char('4') => close_focus(),
                Key::Char('5') => toggle_active_tab_sync(),
                Key::Char('6') => close_focused_tab(),
                Key::Char('7') => undo_rename_tab(),
                Key::Char('8') => quit_zellij(),
                Key::Ctrl('a') => previous_swap_layout(),
                Key::Ctrl('b') => next_swap_layout(),
                Key::Ctrl('c') => {
                    let tab_name = "my tab name";
                    go_to_tab_name(tab_name)
                },
                Key::Ctrl('d') => {
                    let tab_name = "my tab name";
                    focus_or_create_tab(tab_name)
                },
                Key::Ctrl('e') => {
                    let tab_index = 2;
                    go_to_tab(tab_index)
                },
                Key::Ctrl('f') => {
                    let plugin_url = "file:/path/to/my/plugin.wasm";
                    start_or_reload_plugin(plugin_url)
                },
                Key::Ctrl('g') => {
                    open_file(std::path::PathBuf::from("/path/to/my/file.rs").as_path());
                },
                Key::Ctrl('h') => {
                    open_file_floating(std::path::PathBuf::from("/path/to/my/file.rs").as_path());
                },
                Key::Ctrl('i') => {
                    open_file_with_line(
                        std::path::PathBuf::from("/path/to/my/file.rs").as_path(),
                        42,
                    );
                },
                Key::Ctrl('j') => {
                    open_file_with_line_floating(
                        std::path::PathBuf::from("/path/to/my/file.rs").as_path(),
                        42,
                    );
                },
                Key::Ctrl('k') => {
                    open_terminal(std::path::PathBuf::from("/path/to/my/file.rs").as_path());
                },
                Key::Ctrl('l') => {
                    open_terminal_floating(
                        std::path::PathBuf::from("/path/to/my/file.rs").as_path(),
                    );
                },
                Key::Ctrl('m') => {
                    open_command_pane(
                        std::path::PathBuf::from("/path/to/my/file.rs").as_path(),
                        vec!["arg1".to_owned(), "arg2".to_owned()],
                    );
                },
                Key::Ctrl('n') => {
                    open_command_pane_floating(
                        std::path::PathBuf::from("/path/to/my/file.rs").as_path(),
                        vec!["arg1".to_owned(), "arg2".to_owned()],
                    );
                },
                Key::Ctrl('o') => {
                    switch_tab_to(1);
                },
                Key::Ctrl('p') => {
                    hide_self();
                },
                _ => {},
            },
            Event::CustomMessage(message, payload) => {
                if message == "pong" {
                    self.received_payload = Some(payload.clone());
                }
            },
            Event::SystemClipboardFailure => {
                // this is just to trigger the worker message
                post_message_to(
                    "test",
                    "ping".to_owned(),
                    "gimme_back_my_payload".to_owned(),
                );
            },
            _ => {},
        }
        let should_render = true;
        self.received_events.push(event);
        should_render
    }

    fn render(&mut self, rows: usize, cols: usize) {
        if let Some(payload) = self.received_payload.as_ref() {
            println!("Payload from worker: {:?}", payload);
        } else {
            println!(
                "Rows: {:?}, Cols: {:?}, Received events: {:?}",
                rows, cols, self.received_events
            );
        }
    }
}
