//! Консольная точка входа учебного AI-агента.

use std::io::{self, BufRead, Write};

use ai_agent::cli::{Cli, ReplCommand, help_text, parse_repl_command};
use ai_agent::{Message, Role, Session};
use clap::Parser;

fn main() {
    let cli = Cli::parse();

    let mut session = match Session::new(&cli.working_dir, &cli.model) {
        Ok(session) => session,
        Err(error) => {
            eprintln!("Ошибка создания сессии: {error}");
            return;
        }
    };

    if let Some(prompt) = cli.prompt {
        run_once(&mut session, &prompt, cli.verbose);
    } else {
        run_repl(&mut session, cli.verbose);
    }
}

fn run_once(session: &mut Session, prompt: &str, verbose: bool) {
    match add_user_message(session, prompt) {
        Ok(_) => {
            println!("Prompt: {prompt}");
            if verbose {
                print_status(session);
            }
            println!("Ответ LLM пока не подключён: prompt сохранён в сессии.");
        }
        Err(error) => eprintln!("Ошибка prompt: {error}"),
    }
}

fn run_repl(session: &mut Session, verbose: bool) {
    println!("Учебный AI-агент без LLM. Введите /help для справки.");
    if verbose {
        print_status(session);
    }

    let stdin = io::stdin();
    let mut input = String::new();
    loop {
        print!("agent> ");
        if let Err(error) = io::stdout().flush() {
            eprintln!("Ошибка вывода приглашения: {error}");
            return;
        }

        input.clear();
        match stdin.lock().read_line(&mut input) {
            Ok(0) => {
                println!();
                return;
            }
            Ok(_) => {}
            Err(error) => {
                eprintln!("Ошибка чтения ввода: {error}");
                return;
            }
        }

        match parse_repl_command(&input) {
            ReplCommand::Help => println!("{}", help_text()),
            ReplCommand::Quit => {
                println!("Сессия завершена.");
                return;
            }
            ReplCommand::Clear => {
                println!("История очищена: {} сообщений.", session.clear_messages());
            }
            ReplCommand::Status => print_status(session),
            ReplCommand::Model(model) => match session.set_model(model) {
                Ok(()) => println!("Модель изменена: {}", session.model()),
                Err(error) => println!("Ошибка модели: {error}"),
            },
            ReplCommand::Prompt(prompt) => match add_user_message(session, &prompt) {
                Ok(number) => println!(
                    "Prompt сохранён. Сообщений в истории: {number}. Ответ LLM пока не подключён."
                ),
                Err(error) => println!("Ошибка prompt: {error}"),
            },
            ReplCommand::Empty => {}
            ReplCommand::Unknown(command) => {
                println!("Неизвестная команда: {command}. Введите /help.");
            }
        }
    }
}

fn add_user_message(session: &mut Session, prompt: &str) -> Result<usize, ai_agent::AppError> {
    let message = Message::new(Role::User, prompt)?;
    Ok(session.add_message(message))
}

fn print_status(session: &Session) {
    println!("Сессия: {}", session.id());
    println!("Модель: {}", session.model());
    println!("Рабочая директория: {}", session.working_dir().display());
    println!("Сообщений: {}", session.messages().len());
}
