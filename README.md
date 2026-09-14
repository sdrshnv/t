# t

`t` is a fast, local task list that understands dependencies. It is a single Rust process backed by SQLite; it has no daemon, network access, or configuration file.

Priority `1` is highest and priority `3` is lowest. A task is ready when each task it depends on is complete. Urgency flows through dependencies, so a low-priority prerequisite of a priority-1 task is scheduled as priority 1. The list renders that as `[1←3]` (effective priority 1, intrinsic priority 3).

## Install

Install the latest release on macOS or Linux (Intel/AMD64 or ARM64):

```sh
curl -fsSL https://github.com/sdrshnv/t/releases/latest/download/install.sh | sh
```

The installer verifies the archive's SHA-256 checksum and installs `t` to
`~/.local/bin` without sudo. It requires `curl`, `tar`, `gzip`, and either `sha256sum` or
`shasum`. If needed, add the directory to your PATH (bash/zsh):

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Add that line to `~/.bashrc` or `~/.zshrc` to keep it across sessions. Rerun the
installer to upgrade. To select a version or install directory:

```sh
curl -fsSL https://github.com/sdrshnv/t/releases/download/v0.1.0/install.sh \
  -o /tmp/t-install.sh
sh /tmp/t-install.sh --version v0.1.0 --install-dir "$HOME/.local/bin"
```

### Manual download

Download the archive for your platform and `SHA256SUMS` from
[GitHub Releases](https://github.com/sdrshnv/t/releases/latest):

| Platform | Archive target |
| --- | --- |
| Linux Intel/AMD64 | `x86_64-unknown-linux-musl` |
| Linux ARM64 | `aarch64-unknown-linux-musl` |
| macOS Intel | `x86_64-apple-darwin` |
| macOS Apple Silicon | `aarch64-apple-darwin` |

Linux binaries bundle musl and SQLite. macOS binaries require macOS 11 or later
and are not Apple signed or notarized.

For example, after downloading the Linux Intel/AMD64 archive and checksums into
the current directory:

```sh
grep '  t-v0.1.0-x86_64-unknown-linux-musl.tar.gz$' SHA256SUMS | sha256sum -c -
tar -xzf t-v0.1.0-x86_64-unknown-linux-musl.tar.gz t
mkdir -p "$HOME/.local/bin"
install -m 755 t "$HOME/.local/bin/t"
t --version
```

On macOS, use the matching archive and `shasum -a 256 -c -` in place of
`sha256sum -c -`. Proceed with extraction only if verification succeeds.

Remove the installed executable to uninstall (`rm "$HOME/.local/bin/t"` for the
default directory). Upgrades and uninstalling the executable preserve task data.

### Build from source

With Rust installed, run this from a checkout:

```sh
cargo install --locked --path .
```

The database is stored at `$XDG_DATA_HOME/t/tasks.db`, falling back to `$HOME/.local/share/t/tasks.db`.

## Use

```text
t                                      show the next task and one context path
t shuffle                              cycle within the current effective priority tier
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

`t shuffle` advances through ready tasks in the current task's effective priority tier
and wraps around. The cycle uses the scheduler's existing order: intrinsic priority,
then readiness time, then task ID. Inherited urgency counts, so a `[1←3]` prerequisite
cycles with priority-1 tasks. Each shuffle shows the chosen task and its context path.

The choice persists: `t` shows it again, and `t done` or `t edit` without an ID acts
on it. Numeric views (`t 1`, `t 2`, `t 3`) honor the choice in its tier and otherwise
show the first task in the requested tier; viewing a tier does not change the choice.
Task changes keep the choice while it remains ready in the highest effective tier.
Completing, deleting, or blocking it, or introducing a higher effective priority,
clears the choice and resumes normal scheduling. `t ls` keeps its usual order.
`t undo` reverses a shuffle. With no ready tasks, shuffle prints nothing; with only
one task in the tier, it shows that task. Neither case adds an undo record.

Output is unstyled plain text. Empty views are successful and print nothing; domain/runtime errors exit 1, while command-line parsing errors exit 2.

## Develop

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
```

See [the smoke benchmark](docs/benchmark.md) for the release-mode invocation check.

## Release

GitHub Actions validates changes and builds all four release archives on pull
requests and pushes to `main`. A manual workflow run also validates without
publishing. The workflow pins Rust 1.95.0 and builds with `Cargo.lock`.

To release, update the package version in `Cargo.toml` and `Cargo.lock`, merge the
changes to `main`, and wait for CI to pass. Then tag that commit and push the tag:

```sh
git tag -a v0.1.0 -m 'Release v0.1.0'
git push origin v0.1.0
```

Only stable version tags matching the package version publish releases. After
checks, tests, and packaged-binary smoke tests pass, the workflow uploads all four
archives, the pinned installer, and `SHA256SUMS` to a draft release, then publishes
it. Failed draft uploads can be retried by rerunning the workflow; published
releases are never overwritten. See the
[release workflow](https://github.com/sdrshnv/t/actions/workflows/release.yml).
