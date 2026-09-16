use std::path::PathBuf;

use ai_agent::cli::{Cli, ReplCommand, parse_repl_command};
use clap::Parser;

#[test]
fn public_cli_api_parses_prompt_and_configuration() {
    let cli = Cli::try_parse_from([
        "ai-agent",
        "--working-dir",
        "workspace",
        "--model",
        "test-model",
        "просмотри файлы",
    ])
    .unwrap();

    assert_eq!(cli.working_dir, Some(PathBuf::from("workspace")));
    assert_eq!(cli.model, Some("test-model".to_owned()));
    assert_eq!(cli.prompt.as_deref(), Some("просмотри файлы"));
}

#[test]
fn public_repl_parser_supports_session_commands() {
    assert_eq!(parse_repl_command("/clear"), ReplCommand::Clear);
    assert_eq!(parse_repl_command("/status"), ReplCommand::Status);
    assert_eq!(parse_repl_command("/quit"), ReplCommand::Quit);
    assert_eq!(parse_repl_command("/models"), ReplCommand::Models);
    assert_eq!(parse_repl_command("/model"), ReplCommand::Model(None));
    assert_eq!(
        parse_repl_command("/stats off"),
        ReplCommand::Stats(Some("off".to_owned()))
    );
}
