use std::collections::HashSet;
use std::fmt::Write as _;

use chrono::{DateTime, Local, Utc};

use crate::model::{Graph, Task};

pub fn task_line(graph: &Graph, task: &Task) -> String {
    let mark = if task.is_completed() { "✓ " } else { "" };
    match graph.effective_priority(task.id) {
        Some(effective) if effective != task.priority => format!(
            "{mark}#{} [{}←{}] {}",
            task.id, effective, task.priority, task.description
        ),
        _ => format!(
            "{mark}#{} [{}] {}",
            task.id, task.priority, task.description
        ),
    }
}

pub fn next(graph: &Graph, task: &Task) -> String {
    let chain = graph.context_chain(task.id);
    let mut output = String::new();
    for (depth, item) in chain.into_iter().enumerate() {
        let _ = writeln!(output, "{}{}", "  ".repeat(depth), task_line(graph, item));
    }
    output
}

pub fn list(graph: &Graph, mode: ListMode) -> String {
    let mut tasks: Vec<_> = match mode {
        ListMode::Ready => graph.actionable(),
        ListMode::Blocked => graph
            .tasks()
            .filter(|task| task.is_pending() && !graph.is_actionable(task.id))
            .collect(),
        ListMode::Done => graph.tasks().filter(|task| task.is_completed()).collect(),
        ListMode::All => graph.tasks().collect(),
    };

    if mode != ListMode::Ready {
        tasks.sort_by_key(|task| {
            if task.is_pending() {
                (
                    0,
                    graph.effective_priority(task.id).unwrap_or(task.priority),
                    task.priority,
                    task.ready_at.unwrap_or(task.created_at),
                    task.id,
                )
            } else {
                (
                    1,
                    task.priority,
                    task.priority,
                    task.completed_at.unwrap_or(task.created_at),
                    task.id,
                )
            }
        });
    }

    let mut output = String::new();
    for task in tasks {
        let _ = writeln!(output, "{}", task_line(graph, task));
    }
    output
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ListMode {
    Ready,
    All,
    Blocked,
    Done,
}

pub fn show(graph: &Graph, task: &Task) -> String {
    let status = if task.is_completed() {
        "done"
    } else if graph.is_actionable(task.id) {
        "ready"
    } else {
        "blocked"
    };
    let mut output = format!("{}\nstatus: {status}\n", task_line(graph, task));
    if let Some(effective) = graph.effective_priority(task.id) {
        let _ = writeln!(output, "effective priority: {effective}");
    }
    let _ = writeln!(output, "created: {}", local_time(task.created_at));
    let _ = writeln!(output, "updated: {}", local_time(task.updated_at));
    if let Some(ready_at) = task.ready_at {
        let _ = writeln!(output, "ready: {}", local_time(ready_at));
    }
    if let Some(completed_at) = task.completed_at {
        let _ = writeln!(output, "completed: {}", local_time(completed_at));
    }
    append_ids(&mut output, "depends on", graph.children(task.id));
    append_ids(&mut output, "needed by", graph.parents(task.id));
    output
}

fn append_ids(output: &mut String, label: &str, ids: &[i64]) {
    if ids.is_empty() {
        return;
    }
    let ids = ids
        .iter()
        .map(|id| format!("#{id}"))
        .collect::<Vec<_>>()
        .join(", ");
    let _ = writeln!(output, "{label}: {ids}");
}

fn local_time(timestamp: i64) -> String {
    DateTime::<Utc>::from_timestamp_micros(timestamp)
        .map(DateTime::<Local>::from)
        .map(|time| time.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| timestamp.to_string())
}

pub fn tree(graph: &Graph) -> String {
    let mut roots: Vec<_> = graph
        .tasks()
        .filter(|task| task.is_pending())
        .filter(|task| {
            graph.parents(task.id).iter().all(|parent| {
                graph
                    .task(*parent)
                    .is_none_or(|parent_task| !parent_task.is_pending())
            })
        })
        .collect();
    roots.sort_by_key(|task| task.id);

    let mut output = String::new();
    let mut expanded = HashSet::new();
    for (index, root) in roots.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        render_tree_node(graph, root.id, "", true, true, &mut expanded, &mut output);
    }
    output
}

fn render_tree_node(
    graph: &Graph,
    id: i64,
    prefix: &str,
    last: bool,
    root: bool,
    expanded: &mut HashSet<i64>,
    output: &mut String,
) {
    let Some(task) = graph.task(id) else {
        return;
    };
    if root {
        output.push_str(&task_line(graph, task));
    } else {
        output.push_str(prefix);
        output.push_str(if last { "└─ " } else { "├─ " });
        output.push_str(&task_line(graph, task));
    }
    if !expanded.insert(id) {
        output.push_str(" [see above]\n");
        return;
    }
    output.push('\n');

    let children = graph.children(id);
    let child_prefix = if root {
        String::new()
    } else {
        format!("{prefix}{}", if last { "   " } else { "│  " })
    };
    for (index, child) in children.iter().enumerate() {
        render_tree_node(
            graph,
            *child,
            &child_prefix,
            index + 1 == children.len(),
            false,
            expanded,
            output,
        );
    }
}

pub fn find(graph: &Graph, query: &str) -> String {
    let query = query.to_lowercase();
    let mut tasks: Vec<_> = graph
        .tasks()
        .filter(|task| task.description.to_lowercase().contains(&query))
        .collect();
    tasks.sort_by_key(|task| task.id);
    let mut output = String::new();
    for task in tasks {
        let _ = writeln!(output, "{}", task_line(graph, task));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Dependency;

    fn task(id: i64, done: bool) -> Task {
        Task {
            id,
            priority: id.min(3) as u8,
            description: format!("task {id}"),
            created_at: id,
            updated_at: id,
            completed_at: done.then_some(10),
            deleted_at: None,
            ready_at: Some(id),
        }
    }

    #[test]
    fn tree_marks_done_and_references_shared_nodes() {
        let graph = Graph::new(
            vec![task(1, false), task(2, false), task(3, true)],
            vec![
                Dependency {
                    parent: 1,
                    child: 3,
                },
                Dependency {
                    parent: 2,
                    child: 3,
                },
            ],
        );
        assert_eq!(
            tree(&graph),
            "#1 [1] task 1\n└─ ✓ #3 [3] task 3\n\n#2 [2] task 2\n└─ ✓ #3 [3] task 3 [see above]\n"
        );
    }

    #[test]
    fn find_is_case_insensitive_and_marks_completed_tasks() {
        let mut first = task(1, false);
        first.description = "File TAXES".into();
        let mut second = task(2, true);
        second.description = "tax receipt".into();
        let graph = Graph::new(vec![first, second], vec![]);
        assert_eq!(
            find(&graph, "TaX"),
            "#1 [1] File TAXES\n✓ #2 [2] tax receipt\n"
        );
    }
}
