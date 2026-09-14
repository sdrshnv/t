use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Task {
    pub id: i64,
    pub priority: u8,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub deleted_at: Option<i64>,
    pub ready_at: Option<i64>,
}

impl Task {
    pub fn is_pending(&self) -> bool {
        self.completed_at.is_none() && self.deleted_at.is_none()
    }

    pub fn is_completed(&self) -> bool {
        self.completed_at.is_some() && self.deleted_at.is_none()
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Dependency {
    pub parent: i64,
    pub child: i64,
}

#[derive(Clone, Debug)]
pub struct Graph {
    tasks: HashMap<i64, Task>,
    children: HashMap<i64, Vec<i64>>,
    parents: HashMap<i64, Vec<i64>>,
    effective: HashMap<i64, u8>,
}

impl Graph {
    pub fn new(tasks: Vec<Task>, dependencies: Vec<Dependency>) -> Self {
        let tasks: HashMap<_, _> = tasks
            .into_iter()
            .filter(|task| task.deleted_at.is_none())
            .map(|task| (task.id, task))
            .collect();
        let mut children: HashMap<i64, Vec<i64>> = HashMap::new();
        let mut parents: HashMap<i64, Vec<i64>> = HashMap::new();
        for dependency in dependencies {
            if tasks.contains_key(&dependency.parent) && tasks.contains_key(&dependency.child) {
                children
                    .entry(dependency.parent)
                    .or_default()
                    .push(dependency.child);
                parents
                    .entry(dependency.child)
                    .or_default()
                    .push(dependency.parent);
            }
        }
        for ids in children.values_mut().chain(parents.values_mut()) {
            ids.sort_unstable();
        }
        let mut graph = Self {
            tasks,
            children,
            parents,
            effective: HashMap::new(),
        };
        let pending_ids: Vec<_> = graph
            .tasks
            .values()
            .filter(|task| task.is_pending())
            .map(|task| task.id)
            .collect();
        for id in pending_ids {
            let mut visiting = HashSet::new();
            graph.effective_priority_inner(id, &mut visiting);
        }
        graph
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn task(&self, id: i64) -> Option<&Task> {
        self.tasks.get(&id)
    }

    pub fn children(&self, id: i64) -> &[i64] {
        self.children.get(&id).map_or(&[], Vec::as_slice)
    }

    pub fn parents(&self, id: i64) -> &[i64] {
        self.parents.get(&id).map_or(&[], Vec::as_slice)
    }

    pub fn effective_priority(&self, id: i64) -> Option<u8> {
        self.effective.get(&id).copied()
    }

    fn effective_priority_inner(&mut self, id: i64, visiting: &mut HashSet<i64>) -> u8 {
        if let Some(priority) = self.effective.get(&id) {
            return *priority;
        }
        let Some(task) = self.tasks.get(&id) else {
            return 3;
        };
        let intrinsic = task.priority;
        if !task.is_pending() || !visiting.insert(id) {
            return intrinsic;
        }
        let pending_parents: Vec<_> = self
            .parents(id)
            .iter()
            .copied()
            .filter(|parent| self.task(*parent).is_some_and(Task::is_pending))
            .collect();
        let mut effective = intrinsic;
        for parent in pending_parents {
            effective = effective.min(self.effective_priority_inner(parent, visiting));
        }
        visiting.remove(&id);
        self.effective.insert(id, effective);
        effective
    }

    pub fn is_actionable(&self, id: i64) -> bool {
        self.task(id).is_some_and(Task::is_pending)
            && self.children(id).iter().all(|child| {
                self.task(*child)
                    .is_none_or(|task| task.completed_at.is_some())
            })
    }

    pub fn actionable(&self) -> Vec<&Task> {
        let mut tasks: Vec<_> = self
            .tasks()
            .filter(|task| self.is_actionable(task.id))
            .collect();
        tasks.sort_by_key(|task| {
            (
                self.effective_priority(task.id).unwrap_or(task.priority),
                task.priority,
                task.ready_at.unwrap_or(task.created_at),
                task.id,
            )
        });
        tasks
    }

    pub fn next(&self) -> Option<&Task> {
        self.actionable().into_iter().next()
    }

    pub fn next_at_priority(&self, priority: u8) -> Option<&Task> {
        self.actionable()
            .into_iter()
            .find(|task| self.effective_priority(task.id) == Some(priority))
    }

    /// Honor a saved selection only while it is actionable in the highest tier.
    pub fn next_with_selection(&self, selected: Option<i64>) -> Option<&Task> {
        let first = self.next()?;
        selected
            .and_then(|id| self.task(id))
            .filter(|task| {
                self.is_actionable(task.id)
                    && self.effective_priority(task.id) == self.effective_priority(first.id)
            })
            .or(Some(first))
    }

    pub fn shuffle_next(&self, selected: Option<i64>) -> Option<&Task> {
        let current = self.next_with_selection(selected)?;
        let tier: Vec<_> = self
            .actionable()
            .into_iter()
            .filter(|task| self.effective_priority(task.id) == self.effective_priority(current.id))
            .collect();
        let index = tier.iter().position(|task| task.id == current.id)?;
        Some(tier[(index + 1) % tier.len()])
    }

    pub fn context_chain(&self, leaf: i64) -> Vec<&Task> {
        let mut chain = Vec::new();
        let mut current = leaf;
        while let Some(task) = self.task(current) {
            chain.push(task);
            let next = self
                .parents(current)
                .iter()
                .filter_map(|id| self.task(*id))
                .filter(|task| task.is_pending())
                .min_by_key(|task| {
                    (
                        self.effective_priority(task.id).unwrap_or(task.priority),
                        task.priority,
                        task.id,
                    )
                });
            let Some(parent) = next else {
                break;
            };
            current = parent.id;
        }
        chain.reverse();
        chain
    }

    pub fn deterministic_path(&self, from: i64, to: i64) -> Option<Vec<i64>> {
        fn visit(
            graph: &Graph,
            current: i64,
            target: i64,
            path: &mut Vec<i64>,
            seen: &mut HashSet<i64>,
        ) -> bool {
            path.push(current);
            if current == target {
                return true;
            }
            seen.insert(current);
            for child in graph.children(current) {
                if !seen.contains(child) && visit(graph, *child, target, path, seen) {
                    return true;
                }
            }
            path.pop();
            false
        }

        let mut path = Vec::new();
        let mut seen = HashSet::new();
        visit(self, from, to, &mut path, &mut seen).then_some(path)
    }
}

pub fn validate_description(raw: &str) -> crate::Result<String> {
    let description = raw.trim();
    if description.is_empty() {
        return Err(crate::Error::domain("description must not be empty"));
    }
    if description.contains(['\n', '\r']) {
        return Err(crate::Error::domain("description must be a single line"));
    }
    Ok(description.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn task(id: i64, priority: u8, ready_at: Option<i64>) -> Task {
        Task {
            id,
            priority,
            description: format!("task {id}"),
            created_at: id,
            updated_at: id,
            completed_at: None,
            deleted_at: None,
            ready_at,
        }
    }

    #[test]
    fn effective_priority_propagates_through_ancestors() {
        let graph = Graph::new(
            vec![task(1, 1, None), task(2, 3, None), task(3, 3, Some(3))],
            vec![
                Dependency {
                    parent: 1,
                    child: 2,
                },
                Dependency {
                    parent: 2,
                    child: 3,
                },
            ],
        );
        assert_eq!(graph.effective_priority(3), Some(1));
        assert_eq!(graph.next().map(|task| task.id), Some(3));
    }

    #[test]
    fn scheduler_uses_all_tie_breakers() {
        let graph = Graph::new(
            vec![
                task(4, 1, Some(20)),
                task(3, 2, Some(10)),
                task(2, 1, Some(10)),
            ],
            vec![],
        );
        let ids: Vec<_> = graph.actionable().iter().map(|task| task.id).collect();
        assert_eq!(ids, vec![2, 4, 3]);
    }

    #[test]
    fn shuffle_cycles_in_scheduler_order_within_the_effective_tier() {
        let mut completed = task(7, 1, Some(1));
        completed.completed_at = Some(30);
        let mut deleted = task(8, 1, Some(1));
        deleted.deleted_at = Some(30);
        let graph = Graph::new(
            vec![
                task(1, 1, None),
                task(2, 1, Some(20)),
                task(3, 3, Some(1)),
                task(4, 2, Some(30)),
                task(6, 1, Some(10)),
                task(5, 1, Some(10)),
                completed,
                deleted,
                task(9, 2, Some(1)),
            ],
            vec![
                Dependency {
                    parent: 1,
                    child: 3,
                },
                Dependency {
                    parent: 1,
                    child: 4,
                },
            ],
        );
        assert_eq!(graph.next().unwrap().id, 5);
        let mut selected = None;
        for expected in [6, 2, 4, 3, 5, 6] {
            selected = graph.shuffle_next(selected).map(|task| task.id);
            assert_eq!(selected, Some(expected));
        }
        for invalid in [1, 7, 8, 9, 99] {
            assert_eq!(graph.next_with_selection(Some(invalid)).unwrap().id, 5);
            assert_eq!(graph.shuffle_next(Some(invalid)).unwrap().id, 6);
        }
    }

    #[test]
    fn representative_context_chooses_best_parent() {
        let graph = Graph::new(
            vec![task(1, 2, None), task(2, 1, None), task(3, 3, Some(3))],
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
        let ids: Vec<_> = graph.context_chain(3).iter().map(|task| task.id).collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn descriptions_are_trimmed_and_single_line() {
        assert_eq!(
            validate_description("  keep  inner   spaces  ").unwrap(),
            "keep  inner   spaces"
        );
        assert!(validate_description(" \t ").is_err());
        assert!(validate_description("one\ntwo").is_err());
    }

    proptest! {
        #[test]
        fn generated_dags_have_expected_actionability_effective_priority_and_selection(
            priorities in prop::collection::vec(1_u8..=3, 1..9),
            edges in prop::collection::vec(any::<bool>(), 64)
        ) {
            let count = priorities.len();
            let tasks = priorities
                .iter()
                .enumerate()
                .map(|(index, priority)| task(index as i64 + 1, *priority, Some(index as i64 + 1)))
                .collect::<Vec<_>>();
            let mut dependencies = Vec::new();
            for parent in 0..count {
                for child in (parent + 1)..count {
                    if edges[parent * count + child] {
                        dependencies.push(Dependency {
                            parent: parent as i64 + 1,
                            child: child as i64 + 1,
                        });
                    }
                }
            }
            let graph = Graph::new(tasks, dependencies.clone());

            let mut expected_effective = priorities.clone();
            for _ in 0..count {
                for dependency in &dependencies {
                    let parent = dependency.parent as usize - 1;
                    let child = dependency.child as usize - 1;
                    expected_effective[child] = expected_effective[child].min(expected_effective[parent]);
                }
            }
            for (index, expected) in expected_effective.iter().enumerate() {
                prop_assert_eq!(graph.effective_priority(index as i64 + 1), Some(*expected));
                let expected_actionable = !dependencies
                    .iter()
                    .any(|dependency| dependency.parent == index as i64 + 1);
                prop_assert_eq!(graph.is_actionable(index as i64 + 1), expected_actionable);
            }

            let expected_next = (0..count)
                .filter(|index| {
                    !dependencies
                        .iter()
                        .any(|dependency| dependency.parent == *index as i64 + 1)
                })
                .min_by_key(|index| {
                    (
                        expected_effective[*index],
                        priorities[*index],
                        *index as i64 + 1,
                        *index as i64 + 1,
                    )
                })
                .map(|index| index as i64 + 1);
            prop_assert_eq!(graph.next().map(|task| task.id), expected_next);

            for dependency in dependencies {
                prop_assert!(dependency.parent < dependency.child);
                prop_assert!(graph
                    .deterministic_path(dependency.child, dependency.parent)
                    .is_none());
            }
        }
    }
}
