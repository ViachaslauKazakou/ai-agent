use std::path::Path;

use ai_agent::{AppError, Message, Role, Session, count_items, parse_words};

#[test]
fn public_api_creates_session_and_appends_messages() {
    let mut session = Session::new("/tmp/project", "demo-model").unwrap();
    let message = Message::new(Role::User, "Изучи проект").unwrap();

    assert!(session.id().as_hyphenated().to_string().len() > 10);
    assert_eq!(session.working_dir(), Path::new("/tmp/project"));
    assert_eq!(session.model(), "demo-model");
    assert_eq!(session.add_message(message), 1);
    assert_eq!(session.messages()[0].content(), "Изучи проект");
}

#[test]
fn public_api_exposes_typed_errors() {
    assert_eq!(parse_words("\t").unwrap_err(), AppError::EmptyInput);
    assert_eq!(
        Message::new(Role::Tool, "").unwrap_err(),
        AppError::EmptyMessage
    );
    assert_eq!(Session::new(".", "").unwrap_err(), AppError::EmptyModel);
}

#[test]
fn generic_function_is_available_from_library() {
    assert_eq!(count_items(&[1, 2, 3, 4]), 4);
}
