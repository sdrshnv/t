use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};

use crate::error::{Error, Result};
use crate::model::{Dependency, Graph, Task};

const SCHEMA_VERSION: i64 = 1;

pub struct Database {
    connection: Connection,
}

impl Database {
    pub fn open_default() -> Result<Self> {
        Self::open(database_path()?)
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        Self::from_connection(connection)
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    fn from_connection(connection: Connection) -> Result<Self> {
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;",
        )?;
        let mut database = Self { connection };
        database.migrate()?;
        Ok(database)
    }

    fn migrate(&mut self) -> Result<()> {
        let version: i64 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::domain(format!(
                "database schema version {version} is newer than supported version {SCHEMA_VERSION}"
            )));
        }
        if version == 0 {
            let tx = self
                .connection
                .transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(
                "CREATE TABLE schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at INTEGER NOT NULL
                 );

                 CREATE TABLE tasks (
                    id INTEGER PRIMARY KEY,
                    priority INTEGER NOT NULL CHECK (priority BETWEEN 1 AND 3),
                    description TEXT NOT NULL CHECK (length(trim(description)) > 0),
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    completed_at INTEGER,
                    deleted_at INTEGER,
                    ready_at INTEGER
                 );

                 CREATE TABLE dependencies (
                    parent_id INTEGER NOT NULL REFERENCES tasks(id),
                    child_id INTEGER NOT NULL REFERENCES tasks(id),
                    PRIMARY KEY (parent_id, child_id),
                    CHECK (parent_id <> child_id)
                 );
                 CREATE INDEX dependencies_by_parent ON dependencies(parent_id, child_id);
                 CREATE INDEX dependencies_by_child ON dependencies(child_id, parent_id);

                 CREATE TABLE metadata (
                    key TEXT PRIMARY KEY,
                    value INTEGER NOT NULL
                 );

                 CREATE TABLE operations (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    happened_at INTEGER NOT NULL,
                    kind TEXT NOT NULL
                 );

                 CREATE TABLE history_tasks (
                    operation_id INTEGER NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
                    id INTEGER NOT NULL,
                    priority INTEGER NOT NULL,
                    description TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    completed_at INTEGER,
                    deleted_at INTEGER,
                    ready_at INTEGER,
                    PRIMARY KEY (operation_id, id)
                 );

                 CREATE TABLE history_dependencies (
                    operation_id INTEGER NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
                    parent_id INTEGER NOT NULL,
                    child_id INTEGER NOT NULL,
                    PRIMARY KEY (operation_id, parent_id, child_id)
                 );

                 CREATE TABLE history_metadata (
                    operation_id INTEGER NOT NULL REFERENCES operations(id) ON DELETE CASCADE,
                    key TEXT NOT NULL,
                    value INTEGER NOT NULL,
                    PRIMARY KEY (operation_id, key)
                 );

                 INSERT INTO metadata(key, value) VALUES ('next_task_id', 1);
                 INSERT INTO schema_migrations(version, applied_at)
                    VALUES (1, CAST((julianday('now') - 2440587.5) * 86400000000 AS INTEGER));
                 PRAGMA user_version = 1;",
            )?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn graph(&self) -> Result<Graph> {
        Ok(Graph::new(self.tasks()?, self.dependencies()?))
    }

    pub fn tasks(&self) -> Result<Vec<Task>> {
        load_tasks(&self.connection, false)
    }

    pub fn task(&self, id: i64) -> Result<Task> {
        load_task(&self.connection, id)?
            .ok_or_else(|| Error::domain(format!("task {id} not found")))
    }

    pub fn dependencies(&self) -> Result<Vec<Dependency>> {
        load_dependencies(&self.connection)
    }

    pub fn add(&mut self, priority: u8, description: &str, now: i64) -> Result<Task> {
        validate_priority(priority)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id: i64 = tx.query_row(
            "SELECT value FROM metadata WHERE key = 'next_task_id'",
            [],
            |row| row.get(0),
        )?;
        snapshot(&tx, "add", now)?;
        tx.execute(
            "INSERT INTO tasks(
                id, priority, description, created_at, updated_at, completed_at, deleted_at, ready_at
             ) VALUES (?1, ?2, ?3, ?4, ?4, NULL, NULL, ?4)",
            params![id, priority, description, now],
        )?;
        tx.execute(
            "UPDATE metadata SET value = ?1 WHERE key = 'next_task_id'",
            [id + 1],
        )?;
        recalculate_readiness(&tx, now)?;
        let task = load_task(&tx, id)?.expect("inserted task must exist");
        tx.commit()?;
        Ok(task)
    }

    pub fn set_priority(&mut self, id: i64, priority: u8, now: i64) -> Result<bool> {
        validate_priority(priority)?;
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = require_task(&tx, id)?;
        if task.priority == priority {
            return Ok(false);
        }
        snapshot(&tx, "priority", now)?;
        tx.execute(
            "UPDATE tasks SET priority = ?1, updated_at = ?2 WHERE id = ?3",
            params![priority, now, id],
        )?;
        recalculate_readiness(&tx, now)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn add_dependency(&mut self, parent: i64, child: i64, now: i64) -> Result<()> {
        if parent == child {
            return Err(Error::domain("a task cannot depend on itself"));
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let parent_task = require_task(&tx, parent)?;
        require_task(&tx, child)?;
        if parent_task.completed_at.is_some() {
            return Err(Error::domain(format!(
                "completed task {parent} cannot gain a dependency"
            )));
        }
        if tx
            .query_row(
                "SELECT 1 FROM dependencies WHERE parent_id = ?1 AND child_id = ?2",
                params![parent, child],
                |_| Ok(()),
            )
            .optional()?
            .is_some()
        {
            return Err(Error::domain(format!(
                "dependency {parent} -> {child} already exists"
            )));
        }
        let graph = graph_from_connection(&tx)?;
        if let Some(path) = graph.deterministic_path(child, parent) {
            let cycle = std::iter::once(parent)
                .chain(path)
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(Error::domain(format!(
                "dependency would create cycle: {cycle}"
            )));
        }
        snapshot(&tx, "dep", now)?;
        tx.execute(
            "INSERT INTO dependencies(parent_id, child_id) VALUES (?1, ?2)",
            params![parent, child],
        )?;
        recalculate_readiness(&tx, now)?;
        tx.commit()?;
        Ok(())
    }

    pub fn remove_dependency(&mut self, parent: i64, child: i64, now: i64) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_task(&tx, parent)?;
        require_task(&tx, child)?;
        let exists = tx
            .query_row(
                "SELECT 1 FROM dependencies WHERE parent_id = ?1 AND child_id = ?2",
                params![parent, child],
                |_| Ok(()),
            )
            .optional()?
            .is_some();
        if !exists {
            return Err(Error::domain(format!(
                "dependency {parent} -> {child} not found"
            )));
        }
        snapshot(&tx, "undep", now)?;
        tx.execute(
            "DELETE FROM dependencies WHERE parent_id = ?1 AND child_id = ?2",
            params![parent, child],
        )?;
        recalculate_readiness(&tx, now)?;
        tx.commit()?;
        Ok(())
    }

    pub fn complete(&mut self, id: i64, now: i64) -> Result<Task> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = require_task(&tx, id)?;
        if task.completed_at.is_some() {
            return Err(Error::domain(format!("task {id} is already completed")));
        }
        let blocking: Option<i64> = tx
            .query_row(
                "SELECT d.child_id
                   FROM dependencies d
                   JOIN tasks child ON child.id = d.child_id
                  WHERE d.parent_id = ?1
                    AND child.deleted_at IS NULL
                    AND child.completed_at IS NULL
                  ORDER BY d.child_id
                  LIMIT 1",
                [id],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(child) = blocking {
            return Err(Error::domain(format!(
                "task {id} is blocked by pending dependency {child}"
            )));
        }
        snapshot(&tx, "done", now)?;
        tx.execute(
            "UPDATE tasks SET completed_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        recalculate_readiness(&tx, now)?;
        let task = load_task(&tx, id)?.expect("completed task must exist");
        tx.commit()?;
        Ok(task)
    }

    pub fn edit_description(
        &mut self,
        id: i64,
        expected: &str,
        description: &str,
        now: i64,
    ) -> Result<bool> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = require_task(&tx, id)?;
        if task.description != expected {
            return Err(Error::domain(format!(
                "task {id} changed while the editor was open"
            )));
        }
        if expected == description {
            return Ok(false);
        }
        snapshot(&tx, "edit", now)?;
        tx.execute(
            "UPDATE tasks SET description = ?1, updated_at = ?2 WHERE id = ?3",
            params![description, now, id],
        )?;
        recalculate_readiness(&tx, now)?;
        tx.commit()?;
        Ok(true)
    }

    pub fn remove(&mut self, id: i64, force: bool, now: i64) -> Result<Task> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let task = require_task(&tx, id)?;
        let connections: i64 = tx.query_row(
            "SELECT count(*) FROM dependencies WHERE parent_id = ?1 OR child_id = ?1",
            [id],
            |row| row.get(0),
        )?;
        if connections > 0 && !force {
            return Err(Error::domain(format!(
                "task {id} has {connections} dependency connection(s); use --force"
            )));
        }
        snapshot(&tx, "rm", now)?;
        if connections > 0 {
            tx.execute(
                "DELETE FROM dependencies WHERE parent_id = ?1 OR child_id = ?1",
                [id],
            )?;
        }
        tx.execute(
            "UPDATE tasks SET deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
            params![now, id],
        )?;
        recalculate_readiness(&tx, now)?;
        tx.commit()?;
        Ok(task)
    }

    pub fn undo(&mut self) -> Result<String> {
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let operation: Option<(i64, String)> = tx
            .query_row(
                "SELECT id, kind FROM operations ORDER BY id DESC LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        let Some((operation_id, kind)) = operation else {
            return Err(Error::domain("nothing to undo"));
        };

        tx.execute("DELETE FROM dependencies", [])?;
        tx.execute("DELETE FROM tasks", [])?;
        tx.execute("DELETE FROM metadata", [])?;
        tx.execute(
            "INSERT INTO tasks(
                id, priority, description, created_at, updated_at, completed_at, deleted_at, ready_at
             )
             SELECT id, priority, description, created_at, updated_at, completed_at, deleted_at, ready_at
               FROM history_tasks WHERE operation_id = ?1 ORDER BY id",
            [operation_id],
        )?;
        tx.execute(
            "INSERT INTO dependencies(parent_id, child_id)
             SELECT parent_id, child_id FROM history_dependencies
              WHERE operation_id = ?1 ORDER BY parent_id, child_id",
            [operation_id],
        )?;
        tx.execute(
            "INSERT INTO metadata(key, value)
             SELECT key, value FROM history_metadata WHERE operation_id = ?1 ORDER BY key",
            [operation_id],
        )?;
        tx.execute("DELETE FROM operations WHERE id = ?1", [operation_id])?;
        tx.commit()?;
        Ok(kind)
    }

    #[cfg(test)]
    fn history_len(&self) -> Result<i64> {
        Ok(self
            .connection
            .query_row("SELECT count(*) FROM operations", [], |row| row.get(0))?)
    }
}

pub fn database_path() -> Result<PathBuf> {
    if let Some(xdg) = env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(xdg).join("t/tasks.db"));
    }
    if let Some(home) = env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home).join(".local/share/t/tasks.db"));
    }
    Err(Error::DataDirectory)
}

fn validate_priority(priority: u8) -> Result<()> {
    if (1..=3).contains(&priority) {
        Ok(())
    } else {
        Err(Error::domain("priority must be 1, 2, or 3"))
    }
}

fn task_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Task> {
    let priority: i64 = row.get(1)?;
    Ok(Task {
        id: row.get(0)?,
        priority: priority as u8,
        description: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        completed_at: row.get(5)?,
        deleted_at: row.get(6)?,
        ready_at: row.get(7)?,
    })
}

fn load_tasks(connection: &Connection, include_deleted: bool) -> Result<Vec<Task>> {
    let sql = if include_deleted {
        "SELECT id, priority, description, created_at, updated_at, completed_at, deleted_at, ready_at
           FROM tasks ORDER BY id"
    } else {
        "SELECT id, priority, description, created_at, updated_at, completed_at, deleted_at, ready_at
           FROM tasks WHERE deleted_at IS NULL ORDER BY id"
    };
    let mut statement = connection.prepare(sql)?;
    let rows = statement.query_map([], task_from_row)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn load_task(connection: &Connection, id: i64) -> Result<Option<Task>> {
    Ok(connection
        .query_row(
            "SELECT id, priority, description, created_at, updated_at, completed_at, deleted_at, ready_at
               FROM tasks WHERE id = ?1 AND deleted_at IS NULL",
            [id],
            task_from_row,
        )
        .optional()?)
}

fn require_task(connection: &Connection, id: i64) -> Result<Task> {
    load_task(connection, id)?.ok_or_else(|| Error::domain(format!("task {id} not found")))
}

fn load_dependencies(connection: &Connection) -> Result<Vec<Dependency>> {
    let mut statement = connection.prepare(
        "SELECT d.parent_id, d.child_id
           FROM dependencies d
           JOIN tasks parent ON parent.id = d.parent_id
           JOIN tasks child ON child.id = d.child_id
          WHERE parent.deleted_at IS NULL AND child.deleted_at IS NULL
          ORDER BY d.parent_id, d.child_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(Dependency {
            parent: row.get(0)?,
            child: row.get(1)?,
        })
    })?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn graph_from_connection(connection: &Connection) -> Result<Graph> {
    Ok(Graph::new(
        load_tasks(connection, false)?,
        load_dependencies(connection)?,
    ))
}

fn snapshot(tx: &Transaction<'_>, kind: &str, now: i64) -> Result<i64> {
    tx.execute(
        "INSERT INTO operations(happened_at, kind) VALUES (?1, ?2)",
        params![now, kind],
    )?;
    let operation_id = tx.last_insert_rowid();
    tx.execute(
        "INSERT INTO history_tasks(
            operation_id, id, priority, description, created_at, updated_at,
            completed_at, deleted_at, ready_at
         )
         SELECT ?1, id, priority, description, created_at, updated_at,
                completed_at, deleted_at, ready_at
           FROM tasks",
        [operation_id],
    )?;
    tx.execute(
        "INSERT INTO history_dependencies(operation_id, parent_id, child_id)
         SELECT ?1, parent_id, child_id FROM dependencies",
        [operation_id],
    )?;
    tx.execute(
        "INSERT INTO history_metadata(operation_id, key, value)
         SELECT ?1, key, value FROM metadata",
        [operation_id],
    )?;
    Ok(operation_id)
}

fn recalculate_readiness(tx: &Transaction<'_>, now: i64) -> Result<()> {
    let pending: Vec<(i64, Option<i64>, bool)> = {
        let mut statement = tx.prepare(
            "SELECT task.id,
                    task.ready_at,
                    NOT EXISTS (
                        SELECT 1
                          FROM dependencies dependency
                          JOIN tasks child ON child.id = dependency.child_id
                         WHERE dependency.parent_id = task.id
                           AND child.deleted_at IS NULL
                           AND child.completed_at IS NULL
                    ) AS actionable
               FROM tasks task
              WHERE task.deleted_at IS NULL AND task.completed_at IS NULL
              ORDER BY task.id",
        )?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<i64>>(1)?,
                row.get::<_, bool>(2)?,
            ))
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for (id, ready_at, actionable) in pending {
        match (ready_at, actionable) {
            (None, true) => {
                tx.execute(
                    "UPDATE tasks SET ready_at = ?1 WHERE id = ?2",
                    params![now, id],
                )?;
            }
            (Some(_), false) => {
                tx.execute("UPDATE tasks SET ready_at = NULL WHERE id = ?1", [id])?;
            }
            _ => {}
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::validate_description;
    use proptest::prelude::*;

    fn db() -> Database {
        Database::in_memory().unwrap()
    }

    #[test]
    fn readiness_transitions_and_completed_value_is_preserved() {
        let mut db = db();
        let parent = db.add(1, "parent", 10).unwrap();
        let child = db.add(2, "child", 20).unwrap();
        db.add_dependency(parent.id, child.id, 30).unwrap();
        assert_eq!(db.task(parent.id).unwrap().ready_at, None);
        assert_eq!(db.task(child.id).unwrap().ready_at, Some(20));

        db.complete(child.id, 40).unwrap();
        assert_eq!(db.task(child.id).unwrap().ready_at, Some(20));
        assert_eq!(db.task(parent.id).unwrap().ready_at, Some(40));
    }

    #[test]
    fn undep_unlocks_with_the_command_timestamp() {
        let mut db = db();
        let parent = db.add(1, "parent", 10).unwrap();
        let child = db.add(2, "child", 20).unwrap();
        db.add_dependency(parent.id, child.id, 30).unwrap();
        db.remove_dependency(parent.id, child.id, 50).unwrap();
        assert_eq!(db.task(parent.id).unwrap().ready_at, Some(50));
    }

    #[test]
    fn dependencies_validate_duplicates_self_cycles_and_completed_parents() {
        let mut db = db();
        for id in 0..3 {
            db.add(1, &format!("task {id}"), id).unwrap();
        }
        assert!(db.add_dependency(1, 1, 10).is_err());
        db.add_dependency(1, 2, 11).unwrap();
        assert!(db.add_dependency(1, 2, 12).is_err());
        db.add_dependency(2, 3, 13).unwrap();
        let error = db.add_dependency(3, 1, 14).unwrap_err().to_string();
        assert_eq!(error, "dependency would create cycle: 3 -> 1 -> 2 -> 3");

        db.complete(3, 15).unwrap();
        db.complete(2, 16).unwrap();
        db.complete(1, 17).unwrap();
        assert!(db.add_dependency(1, 3, 18).is_err());
    }

    #[test]
    fn pending_parent_can_depend_on_completed_child() {
        let mut db = db();
        let parent = db.add(1, "parent", 1).unwrap();
        let child = db.add(2, "child", 2).unwrap();
        db.complete(child.id, 3).unwrap();
        db.add_dependency(parent.id, child.id, 4).unwrap();
        assert_eq!(db.task(parent.id).unwrap().ready_at, Some(1));
    }

    #[test]
    fn blocked_task_cannot_be_completed() {
        let mut db = db();
        let parent = db.add(1, "parent", 1).unwrap();
        let child = db.add(2, "child", 2).unwrap();
        db.add_dependency(parent.id, child.id, 3).unwrap();
        assert!(db.complete(parent.id, 4).is_err());
        assert_eq!(db.history_len().unwrap(), 3);
    }

    #[test]
    fn forced_deletion_removes_edges_and_unlocks_parents() {
        let mut db = db();
        let parent = db.add(1, "parent", 1).unwrap();
        let child = db.add(2, "child", 2).unwrap();
        db.add_dependency(parent.id, child.id, 3).unwrap();
        assert!(db.remove(child.id, false, 4).is_err());
        db.remove(child.id, true, 5).unwrap();
        assert!(db.task(child.id).is_err());
        assert_eq!(db.task(parent.id).unwrap().ready_at, Some(5));
        assert!(db.dependencies().unwrap().is_empty());
    }

    #[test]
    fn multi_level_undo_restores_fields_edges_and_allocator() {
        let mut db = db();
        let one = db.add(1, "one", 1).unwrap();
        let two = db.add(2, "two", 2).unwrap();
        db.add_dependency(one.id, two.id, 3).unwrap();
        db.complete(two.id, 4).unwrap();
        db.remove(one.id, true, 5).unwrap();

        assert_eq!(db.undo().unwrap(), "rm");
        assert!(db.task(one.id).is_ok());
        assert_eq!(db.undo().unwrap(), "done");
        assert!(db.task(two.id).unwrap().completed_at.is_none());
        assert_eq!(db.task(one.id).unwrap().ready_at, None);
        assert_eq!(db.undo().unwrap(), "dep");
        assert!(db.dependencies().unwrap().is_empty());
        assert_eq!(db.undo().unwrap(), "add");
        assert!(db.task(two.id).is_err());
        let replacement = db.add(3, "replacement", 9).unwrap();
        assert_eq!(replacement.id, two.id);
    }

    #[test]
    fn no_ops_do_not_create_history() {
        let mut db = db();
        let task = db.add(1, "one", 1).unwrap();
        assert!(!db.set_priority(task.id, 1, 2).unwrap());
        assert!(!db.edit_description(task.id, "one", "one", 3).unwrap());
        assert_eq!(db.history_len().unwrap(), 1);
    }

    #[test]
    fn priority_validation_is_shared() {
        let mut db = db();
        assert!(db.add(0, "bad", 1).is_err());
        assert!(db.add(4, "bad", 1).is_err());
        assert_eq!(validate_description("ok").unwrap(), "ok");
    }

    proptest! {
        #[test]
        fn completion_is_safe_and_undo_restores_exact_observable_state(
            priorities in prop::collection::vec(1_u8..=3, 1..8),
            edges in prop::collection::vec(any::<bool>(), 49)
        ) {
            let mut db = db();
            for (index, priority) in priorities.iter().enumerate() {
                db.add(*priority, &format!("task {}", index + 1), index as i64 + 1)
                    .unwrap();
            }
            let count = priorities.len();
            let mut timestamp = count as i64 + 10;
            for parent in 0..count {
                for child in (parent + 1)..count {
                    if edges[parent * count + child] {
                        db.add_dependency(parent as i64 + 1, child as i64 + 1, timestamp)
                            .unwrap();
                        timestamp += 1;
                    }
                }
            }

            let tasks_before = db.tasks().unwrap();
            let dependencies_before = db.dependencies().unwrap();
            let leaf_id = count as i64;
            let leaf_ready_at = db.task(leaf_id).unwrap().ready_at;
            db.complete(leaf_id, timestamp).unwrap();
            prop_assert_eq!(db.task(leaf_id).unwrap().ready_at, leaf_ready_at);
            let graph = db.graph().unwrap();
            for task in graph.tasks().filter(|task| task.is_pending()) {
                prop_assert_eq!(task.ready_at.is_some(), graph.is_actionable(task.id));
            }

            prop_assert_eq!(db.undo().unwrap(), "done");
            prop_assert_eq!(db.tasks().unwrap(), tasks_before);
            prop_assert_eq!(db.dependencies().unwrap(), dependencies_before);
        }
    }
}
