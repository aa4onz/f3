// src/tui/mod.rs
pub mod theme;
pub mod components;
pub mod modals;

use crate::app::state::ActiveModal;
use crate::app::AppState;
use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};

pub fn render(f: &mut Frame, state: &mut AppState) {
    let screen_size = f.size();

    let horizontal_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(15),
            Constraint::Percentage(70),
            Constraint::Percentage(15),
        ])
        .split(screen_size);

    let middle_area = horizontal_chunks[1];

    let vertical_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(middle_area);

    let msg_list = components::render_messages(
        &state.messages,
        &state.self_username,
        state.show_timestamp,
        state.show_latency,
    );
    f.render_stateful_widget(msg_list, vertical_chunks[0], &mut state.list_state);

    let input_box = components::render_input_box(&state.input_text);
    f.render_widget(input_box, vertical_chunks[1]);

    // Render Modal Overlays
    match state.active_modal {
        ActiveModal::LogoutPrompt => {
            modals::render_logout_modal(f, screen_size);
        }
        ActiveModal::SwitchChannelPrompt => {
            modals::render_switch_channel_modal(f, screen_size, &state.modal_input);
        }
        ActiveModal::None => {}
    }
}
