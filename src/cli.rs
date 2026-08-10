use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

#[derive(Debug, Parser)]
#[command(
    name = "t",
    version,
    about = "A dependency-aware task list",
    color = clap::ColorChoice::Never
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Add a task.
    Add {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
        priority: u8,
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        description: Vec<String>,
    },
    /// Complete a task, or the next task when no ID is supplied.
    Done { id: Option<i64> },
    /// Show the next priority-1 task.
    #[command(name = "1")]
    One,
    /// Show the next priority-2 task.
    #[command(name = "2")]
    Two,
    /// Show the next priority-3 task.
    #[command(name = "3")]
    Three,
    /// Change a task's intrinsic priority.
    Priority {
        id: i64,
        #[arg(value_parser = clap::value_parser!(u8).range(1..=3))]
        priority: u8,
    },
    /// Make PARENT depend on CHILD.
    Dep { parent: i64, child: i64 },
    /// Remove a dependency.
    Undep { parent: i64, child: i64 },
    /// List tasks.
    Ls {
        #[arg(long, conflicts_with_all = ["blocked", "done"])]
        all: bool,
        #[arg(long, conflicts_with_all = ["all", "done"])]
        blocked: bool,
        #[arg(long, conflicts_with_all = ["all", "blocked"])]
        done: bool,
    },
    /// Render the task dependency graph.
    Tree,
    /// Show all details for one task.
    Show { id: i64 },
    /// Edit a task, or the next task when no ID is supplied.
    Edit { id: Option<i64> },
    /// Undo the latest successful mutation.
    Undo,
    /// Search task descriptions.
    Find {
        #[arg(required = true, num_args = 1.., trailing_var_arg = true)]
        query: Vec<String>,
    },
    /// Soft-delete a task.
    Rm {
        id: i64,
        #[arg(long)]
        force: bool,
    },
    /// Generate shell completion code.
    Completions { shell: Shell },
}

#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
}

impl Cli {
    pub fn clap_command() -> clap::Command {
        Self::command()
    }
}
