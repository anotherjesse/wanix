mod command_record;
mod message;
mod protocol;
mod request;
mod root_changes;
mod session;
mod shell_activity;
pub(in crate::serve) mod shell_history;
mod shell_observation;

#[cfg(test)]
pub(super) use message::parse_terminal_resize_message;
pub(super) use request::{
    QJS_SHELL_WEBSOCKET_PATH, is_qjs_shell_websocket_path, qjs_shell_cwd_from_target,
};
pub(super) use session::serve_terminal_websocket_connection;
