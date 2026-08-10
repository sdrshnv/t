# t

`t` is a fast, local task list that understands dependencies. It is a single Rust process backed by SQLite; it has no daemon, network access, or configuration file.

Priority `1` is highest and priority `3` is lowest. A task is ready when each task it depends on is complete. Urgency flows through dependencies, so a low-priority prerequisite of a priority-1 task is scheduled as priority 1. The list renders that as `[1←3]` (effective priority 1, intrinsic priority 3).

## Install

```sh
cargo install --path .
```

The database is stored at `$XDG_DATA_HOME/t/tasks.db`, falling back to `$HOME/.local/share/t/tasks.db`.

## Use

```text
t                                      show the next task and one context path
t 1 | t 2 | t 3                        show the next task at that effective priority
t add <1|2|3> <description...>          add a task
t done [id]                            complete a task (the next task by default)
t priority <id> <1|2|3>                change intrinsic priority
t dep <parent> <child>                 make parent depend on child
t undep <parent> <child>               remove that dependency
t ls [--all|--blocked|--done]           list tasks
t tree                                 show pending task DAGs
t show <id>                            show task details
t edit [id]                            edit with $VISUAL, $EDITOR, or vi
t find <query...>                      search pending and completed tasks
t rm <id> [--force]                    soft-delete a task
t undo                                 undo the latest successful mutation
t completions <bash|zsh|fish>          print shell completion code
```

Here `parent` is the work that is blocked and `child` is its prerequisite:

```sh
t add 1 File taxes
t add 2 Gather documents
t add 3 Download W-2
t dep 1 2
t dep 2 3
t
```

```text
#1 [1] File taxes
  #2 [1←2] Gather documents
    #3 [1←3] Download W-2
```

All mutations and their undo records commit atomically. Deletion is soft; a connected task requires `rm --force`, which removes its incident dependency edges in the same transaction. `undo` is persistent and multi-level, but there is no redo stack.

Output is unstyled plain text. Empty views are successful and print nothing; domain/runtime errors exit 1, while command-line parsing errors exit 2.

## Develop

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

See [the smoke benchmark](docs/benchmark.md) for the release-mode invocation check.
