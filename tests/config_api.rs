use std::collections::HashMap;

use ai_agent::{AppError, Config, cli::Cli};
use clap::Parser;

#[test]
fn public_config_api_uses_environment_values() {
    let cli = Cli::try_parse_from(["ai-agent", "--working-dir", "."]).unwrap();
    let environment = HashMap::from([
        ("MODEL".to_owned(), "env-model".to_owned()),
        (
            "LITELLM_BASE_URL".to_owned(),
            "http://localhost/v1".to_owned(),
        ),
    ]);

    let config = Config::from_sources(&cli, &environment).unwrap();

    assert_eq!(config.model, "env-model");
    assert_eq!(config.provider, "litellm");
    assert_eq!(config.reasoning_effort, "medium");
}

#[test]
fn public_config_api_does_not_expose_api_key_in_debug() {
    let cli = Cli::try_parse_from(["ai-agent", "--working-dir", "."]).unwrap();
    let environment = HashMap::from([(
        String::from("LITELLM_API_KEY"),
        String::from("secret-value"),
    )]);
    let config = Config::from_sources(&cli, &environment).unwrap();

    assert!(!format!("{config:?}").contains("secret-value"));
}

#[test]
fn public_config_api_formats_readable_json_without_api_key() {
    let cli = Cli::try_parse_from(["ai-agent", "--working-dir", "."]).unwrap();
    let environment = HashMap::from([(
        String::from("LITELLM_API_KEY"),
        String::from("secret-value"),
    )]);
    let config = Config::from_sources(&cli, &environment).unwrap();

    let json = config.to_pretty_json();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(value["provider"], "litellm");
    assert!(value.get("api_key").is_none());
    assert!(!json.contains("secret-value"));
    assert!(json.contains("\n"));
}

#[test]
fn public_config_api_reports_invalid_path() {
    let cli = Cli::try_parse_from(["ai-agent", "--working-dir", "/does/not/exist"]).unwrap();

    assert!(matches!(
        Config::from_sources(&cli, &HashMap::new()),
        Err(AppError::InvalidWorkingDirectory(_))
    ));
}
