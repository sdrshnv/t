use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::{Command as ProcessCommand, ExitCode};

use chrono::Utc;
use clap::Parser;
use clap_complete::{generate, shells};
use t::cli::{Cli, Command, Shell};
use t::db::Database;
use t::model::validate_description;
use t::render::{self, ListMode};
use t::{Error, Result};

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("t: {error}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<()> {
    if let Some(Command::Completions { shell }) = cli.command {
        print_completions(shell);
        return Ok(());
    }

    let mut database = Database::open_default()?;
    let now = || Utc::now().timestamp_micros();

    match cli.command {
        None => print_next(&database, None),
        Some(Command::One) => print_next(&database, Some(1)),
        Some(Command::Two) => print_next(&database, Some(2)),
        Some(Command::Three) => print_next(&database, Some(3)),
        Some(Command::Add {
            priority,
            description,
        }) => {
            let description = validate_description(&description.join(" "))?;
            let task = database.add(priority, &description, now())?;
            println!("added #{}", task.id);
            Ok(())
        }
        Some(Command::Done { id }) => {
            let id = resolve_id(&database, id)?;
            let task = database.complete(id, now())?;
            println!("done #{}", task.id);
            Ok(())
        }
        Some(Command::Priority { id, priority }) => {
            if database.set_priority(id, priority, now())? {
                println!("priority #{} = {}", id, priority);
            }
            Ok(())
        }
        Some(Command::Dep { parent, child }) => {
            database.add_dependency(parent, child, now())?;
            println!("dependency #{parent} -> #{child}");
            Ok(())
        }
        Some(Command::Undep { parent, child }) => {
            database.remove_dependency(parent, child, now())?;
            println!("removed dependency #{parent} -> #{child}");
            Ok(())
        }
        Some(Command::Ls { all, blocked, done }) => {
            let mode = if all {
                ListMode::All
            } else if blocked {
                ListMode::Blocked
            } else if done {
                ListMode::Done
            } else {
                ListMode::Ready
            };
            print!("{}", render::list(&database.graph()?, mode));
            Ok(())
        }
        Some(Command::Tree) => {
            print!("{}", render::tree(&database.graph()?));
            Ok(())
        }
        Some(Command::Show { id }) => {
            let task = database.task(id)?;
            print!("{}", render::show(&database.graph()?, &task));
            Ok(())
        }
        Some(Command::Edit { id }) => edit(&mut database, id, now()),
        Some(Command::Undo) => {
            let operation = database.undo()?;
            println!("undid {operation}");
            Ok(())
        }
        Some(Command::Find { query }) => {
            let query = validate_description(&query.join(" "))?;
            print!("{}", render::find(&database.graph()?, &query));
            Ok(())
        }
        Some(Command::Rm { id, force }) => {
            database.remove(id, force, now())?;
            println!("removed #{id}");
            Ok(())
        }
        Some(Command::Completions { .. }) => unreachable!("handled before opening database"),
    }
}

fn print_next(database: &Database, priority: Option<u8>) -> Result<()> {
    let graph = database.graph()?;
    let task = match priority {
        Some(priority) => graph.next_at_priority(priority),
        None => graph.next(),
    };
    if let Some(task) = task {
        print!("{}", render::next(&graph, task));
    }
    Ok(())
}

fn resolve_id(database: &Database, id: Option<i64>) -> Result<i64> {
    if let Some(id) = id {
        return Ok(id);
    }
    database
        .graph()?
        .next()
        .map(|task| task.id)
        .ok_or_else(|| Error::domain("no actionable task"))
}

fn edit(database: &mut Database, id: Option<i64>, now: i64) -> Result<()> {
    let id = resolve_id(database, id)?;
    let original = database.task(id)?;
    let mut file = tempfile::Builder::new().prefix("t-edit-").tempfile()?;
    writeln!(file, "{}", original.description)?;
    file.flush()?;

    let editor = env::var("VISUAL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("EDITOR")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| "vi".to_owned());
    let words =
        shell_words::split(&editor).map_err(|error| Error::EditorCommand(error.to_string()))?;
    let (program, arguments) = words
        .split_first()
        .ok_or_else(|| Error::EditorCommand("empty command".to_owned()))?;
    let status = ProcessCommand::new(program)
        .args(arguments)
        .arg(file.path())
        .status()?;
    if !status.success() {
        return Err(Error::domain(format!("editor exited with status {status}")));
    }

    let edited = fs::read_to_string(file.path())?;
    let edited = validate_description(&edited)?;
    if database.edit_description(id, &original.description, &edited, now)? {
        println!("edited #{id}");
    }
    Ok(())
}

fn print_completions(shell: Shell) {
    let mut command = Cli::clap_command();
    let name = command.get_name().to_owned();
    let stdout = io::stdout();
    let mut output = stdout.lock();
    match shell {
        Shell::Bash => generate(shells::Bash, &mut command, name, &mut output),
        Shell::Zsh => generate(shells::Zsh, &mut command, name, &mut output),
        Shell::Fish => generate(shells::Fish, &mut command, name, &mut output),
    }
}
