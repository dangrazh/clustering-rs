//! Editable review metadata, independent of incident analysis.
use crate::model::AnalysisRun;
use anyhow::{bail, ensure, Context, Result};
use chrono::{NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewStatus {
    #[default]
    Review,
    Reviewed,
    GetSmeFeedback,
    PendingSmeFeedback,
    NoOpportunity,
    ToBeAddressed,
    InProgress,
    Solved,
}

impl ReviewStatus {
    pub fn next(self) -> &'static [Self] {
        use ReviewStatus::*;
        match self {
            Review => &[Reviewed],
            Reviewed => &[NoOpportunity, GetSmeFeedback],
            GetSmeFeedback => &[PendingSmeFeedback],
            PendingSmeFeedback => &[NoOpportunity, ToBeAddressed],
            ToBeAddressed => &[InProgress],
            InProgress => &[Solved],
            NoOpportunity | Solved => &[],
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Review => "Review",
            Self::Reviewed => "Reviewed",
            Self::GetSmeFeedback => "Get SME feedback",
            Self::PendingSmeFeedback => "Pending SME feedback",
            Self::NoOpportunity => "No opportunity",
            Self::ToBeAddressed => "To be addressed",
            Self::InProgress => "In progress",
            Self::Solved => "Solved",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actor {
    pub name: String,
    pub email: String,
}
impl Actor {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            !self.name.trim().is_empty() && self.name.len() <= 200,
            "Enter an author name (up to 200 characters)."
        );
        ensure!(valid_email(&self.email), "Enter a valid author email.");
        Ok(())
    }
}
pub fn valid_email(value: &str) -> bool {
    if value.len() > 254 || value.chars().any(char::is_whitespace) {
        return false;
    }
    let parts: Vec<_> = value.split('@').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && parts[1].split('.').count() >= 2
        && parts[1].split('.').all(|part| {
            !part.is_empty() && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Assignments {
    pub sme_email: Option<String>,
    pub owner_email: Option<String>,
    pub eta: Option<NaiveDate>,
}
impl Assignments {
    fn validate(&self, status: ReviewStatus) -> Result<()> {
        for email in [&self.sme_email, &self.owner_email].into_iter().flatten() {
            ensure!(valid_email(email), "Enter a valid assignment email.");
        }
        if matches!(
            status,
            ReviewStatus::PendingSmeFeedback
                | ReviewStatus::ToBeAddressed
                | ReviewStatus::InProgress
                | ReviewStatus::Solved
        ) {
            ensure!(self.sme_email.is_some(), "SME email is required.");
        }
        if matches!(status, ReviewStatus::InProgress | ReviewStatus::Solved) {
            ensure!(
                self.owner_email.is_some() && self.eta.is_some(),
                "Owner email and ETA are required."
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workflow {
    pub status: ReviewStatus,
    pub assignments: Assignments,
    /// Active forward path. Audit events (including undo) are stored separately.
    pub path: Vec<ReviewStatus>,
}
impl Default for Workflow {
    fn default() -> Self {
        Self {
            status: ReviewStatus::Review,
            assignments: Assignments::default(),
            path: vec![ReviewStatus::Review],
        }
    }
}
impl Workflow {
    fn validate(&self) -> Result<()> {
        ensure!(
            self.path.first() == Some(&ReviewStatus::Review)
                && self.path.last() == Some(&self.status),
            "Invalid active workflow path."
        );
        ensure!(
            self.path.windows(2).all(|p| p[0].next().contains(&p[1])),
            "Invalid workflow transition in history."
        );
        self.assignments.validate(self.status)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Comment {
    pub id: String,
    pub text: String,
    pub author: Actor,
    pub created_at: String,
    pub updated_at: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub workflow: Workflow,
    pub inherited: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEvent {
    pub id: String,
    pub target: String,
    pub actor: Actor,
    pub timestamp: String,
    pub action: String,
    pub before: Snapshot,
    pub after: Snapshot,
    pub parent_revision: Option<u64>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    /// None is permitted only on themes and means dynamic parent inheritance.
    pub workflow: Option<Workflow>,
    pub comments: Vec<Comment>,
    pub history: Vec<HistoryEvent>,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotations {
    pub revision: u64,
    pub entries: BTreeMap<String, Annotation>,
    #[serde(default)]
    pub labels: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mutation {
    pub revision: u64,
    pub target: String,
    pub actor: Actor,
    pub action: Action,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    Rename {
        label: String,
    },
    Transition {
        status: ReviewStatus,
        assignments: Assignments,
    },
    Undo,
    Assign {
        assignments: Assignments,
    },
    Inherit,
    AddComment {
        text: String,
    },
    EditComment {
        id: String,
        text: String,
    },
    DeleteComment {
        id: String,
    },
}

impl Annotations {
    pub fn new(run: &AnalysisRun) -> Self {
        let mut entries = BTreeMap::new();
        for cluster in &run.clusters {
            entries.insert(
                cluster.id.0.to_string(),
                Annotation {
                    workflow: Some(Workflow::default()),
                    ..Default::default()
                },
            );
            for theme in &cluster.subgroups {
                entries.insert(
                    format!("{}:{}", cluster.id.0, theme.id),
                    Annotation::default(),
                );
            }
        }
        Self {
            revision: 0,
            entries,
            labels: BTreeMap::new(),
        }
    }
    pub fn label<'a>(&'a self, key: &str, generated: &'a str) -> &'a str {
        self.labels
            .get(key)
            .map(String::as_str)
            .unwrap_or(generated)
    }
    pub fn effective(&self, key: &str) -> Result<&Workflow> {
        let entry = self.entries.get(key).context("Unknown cluster or theme.")?;
        if let Some(workflow) = &entry.workflow {
            return Ok(workflow);
        }
        let (parent, _) = key
            .split_once(':')
            .context("Cluster must have a workflow.")?;
        self.entries
            .get(parent)
            .and_then(|a| a.workflow.as_ref())
            .context("Theme parent missing.")
    }
    pub fn validate(&self, run: &AnalysisRun) -> Result<()> {
        let expected = Self::new(run);
        ensure!(
            self.entries.keys().eq(expected.entries.keys()),
            "Review entities do not match this analysis."
        );
        let mut ids = BTreeSet::new();
        for (key, label) in &self.labels {
            ensure!(
                self.entries.contains_key(key),
                "Label refers to an unknown cluster or theme."
            );
            validate_label(label)?;
        }
        for (key, entry) in &self.entries {
            self.effective(key)?.validate()?;
            for comment in &entry.comments {
                ensure!(ids.insert(comment.id.as_str()), "Duplicate comment ID.");
                validate_comment(&comment.text)?;
                comment.author.validate()?;
                chrono::DateTime::parse_from_rfc3339(&comment.created_at)?;
                if let Some(updated) = &comment.updated_at {
                    chrono::DateTime::parse_from_rfc3339(updated)?;
                }
            }
            for event in &entry.history {
                ensure!(
                    ids.insert(event.id.as_str()) && event.target == *key,
                    "Invalid history reference."
                );
                event.actor.validate()?;
                chrono::DateTime::parse_from_rfc3339(&event.timestamp)?;
                event.before.workflow.validate()?;
                event.after.workflow.validate()?;
                ensure!(
                    !event.before.inherited || key.contains(':'),
                    "Cluster cannot inherit."
                );
                ensure!(
                    !event.after.inherited || key.contains(':'),
                    "Cluster cannot inherit."
                );
                ensure!(
                    matches!(
                        event.action.as_str(),
                        "transition" | "undo" | "assign" | "inherit"
                    ),
                    "Invalid history action."
                );
                let before = &event.before.workflow;
                let after = &event.after.workflow;
                match event.action.as_str() {
                    "transition" => {
                        let mut expected = before.path.clone();
                        expected.push(after.status);
                        ensure!(
                            !event.after.inherited
                                && before.status.next().contains(&after.status)
                                && after.path == expected,
                            "Invalid recorded forward transition."
                        );
                    }
                    "undo" => {
                        ensure!(
                            before.path.len() > 1
                                && !event.after.inherited
                                && after.path == before.path[..before.path.len() - 1]
                                && after.assignments == before.assignments,
                            "Invalid recorded undo."
                        );
                    }
                    "assign" => ensure!(
                        !event.after.inherited && before.path == after.path,
                        "Assignment event changed the state path."
                    ),
                    "inherit" => ensure!(
                        key.contains(':') && !event.before.inherited && event.after.inherited,
                        "Invalid inheritance event."
                    ),
                    _ => unreachable!(),
                }
            }
            if let Some(last) = entry.history.last() {
                ensure!(
                    last.after.inherited == entry.workflow.is_none(),
                    "Workflow inheritance does not match history."
                );
                if let Some(current) = &entry.workflow {
                    ensure!(
                        *current == last.after.workflow,
                        "Current workflow does not match history."
                    );
                }
            }
        }
        Ok(())
    }
    /// Validate and edit one entry on a private copy, then commit atomically.
    pub fn mutate(&mut self, mutation: Mutation) -> Result<()> {
        ensure!(
            mutation.revision == self.revision,
            "Review data changed. Reload the current review data and retry."
        );
        mutation.actor.validate()?;
        if let Action::Rename { label } = &mutation.action {
            ensure!(
                self.entries.contains_key(&mutation.target),
                "Unknown cluster or theme."
            );
            let label = label.trim();
            validate_label(label)?;
            let next = self
                .revision
                .checked_add(1)
                .context("Review revision exhausted.")?;
            self.labels.insert(mutation.target, label.to_owned());
            self.revision = next;
            return Ok(());
        }
        let before = Snapshot {
            workflow: self.effective(&mutation.target)?.clone(),
            inherited: self.entries[&mutation.target].workflow.is_none(),
        };
        let mut entry = self.entries[&mutation.target].clone();
        let timestamp = Utc::now().to_rfc3339();
        let mut workflow = before.workflow.clone();
        let action_name;
        match mutation.action {
            Action::Rename { .. } => unreachable!("Renames handled before workflow edits"),
            Action::AddComment { text } => {
                validate_comment(&text)?;
                entry.comments.push(Comment {
                    id: uuid::Uuid::new_v4().to_string(),
                    text,
                    author: mutation.actor.clone(),
                    created_at: timestamp.clone(),
                    updated_at: None,
                });
                action_name = None;
            }
            Action::EditComment { id, text } => {
                validate_comment(&text)?;
                let comment = entry
                    .comments
                    .iter_mut()
                    .find(|c| c.id == id)
                    .context("Comment not found.")?;
                comment.text = text;
                comment.updated_at = Some(timestamp.clone());
                action_name = None;
            }
            Action::DeleteComment { id } => {
                let index = entry
                    .comments
                    .iter()
                    .position(|c| c.id == id)
                    .context("Comment not found.")?;
                entry.comments.remove(index);
                action_name = None;
            }
            Action::Transition {
                status,
                assignments,
            } => {
                ensure!(
                    workflow.status.next().contains(&status),
                    "This state transition is not allowed."
                );
                workflow.status = status;
                workflow.path.push(status);
                workflow.assignments = assignments;
                workflow.validate()?;
                entry.workflow = Some(workflow);
                action_name = Some("transition");
            }
            Action::Undo => {
                ensure!(workflow.path.len() > 1, "No transition to undo.");
                workflow.path.pop();
                workflow.status = *workflow.path.last().unwrap();
                workflow.validate()?;
                entry.workflow = Some(workflow);
                action_name = Some("undo");
            }
            Action::Assign { assignments } => {
                workflow.assignments = assignments;
                workflow.validate()?;
                entry.workflow = Some(workflow);
                action_name = Some("assign");
            }
            Action::Inherit => {
                ensure!(
                    mutation.target.contains(':') && !before.inherited,
                    "Only overridden themes can return to inheritance."
                );
                entry.workflow = None;
                action_name = Some("inherit");
            }
        }
        if let Some(action) = action_name {
            let after_workflow = match &entry.workflow {
                Some(w) => w.clone(),
                None => self
                    .effective(mutation.target.split_once(':').unwrap().0)?
                    .clone(),
            };
            entry.history.push(HistoryEvent {
                id: uuid::Uuid::new_v4().to_string(),
                target: mutation.target.clone(),
                actor: mutation.actor,
                timestamp,
                action: action.into(),
                after: Snapshot {
                    workflow: after_workflow,
                    inherited: entry.workflow.is_none(),
                },
                parent_revision: mutation.target.contains(':').then_some(self.revision),
                before,
            });
        }
        let next = self
            .revision
            .checked_add(1)
            .context("Review revision exhausted.")?;
        self.entries.insert(mutation.target, entry);
        self.revision = next;
        Ok(())
    }
}
fn validate_comment(text: &str) -> Result<()> {
    if text.trim().is_empty() || text.chars().count() > 100_000 {
        bail!("Enter a comment between 1 and 100,000 characters.");
    }
    Ok(())
}
fn validate_label(label: &str) -> Result<()> {
    ensure!(
        !label.trim().is_empty()
            && label.chars().count() <= 500
            && !label.chars().any(char::is_control),
        "Enter a label of 1–500 characters on a single line."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn renaming_preserves_inheritance_and_rejects_invalid_labels_atomically() {
        let mut data = data();
        edit(
            &mut data,
            "1:1",
            Action::Rename {
                label: "  Network failures  ".into(),
            },
        )
        .unwrap();
        assert_eq!(data.label("1:1", "Generated"), "Network failures");
        assert!(data.entries["1:1"].workflow.is_none());
        assert!(data.entries["1:1"].history.is_empty());
        let before = data.clone();
        for label in [" ".to_owned(), "a\nb".to_owned(), "a".repeat(501)] {
            assert!(edit(&mut data, "1:1", Action::Rename { label }).is_err());
            assert_eq!(before, data);
        }
        assert!(edit(
            &mut data,
            "unknown",
            Action::Rename {
                label: "Valid".into()
            }
        )
        .is_err());
    }
    fn data() -> Annotations {
        Annotations {
            labels: BTreeMap::new(),
            revision: 0,
            entries: BTreeMap::from([
                (
                    "1".into(),
                    Annotation {
                        workflow: Some(Workflow::default()),
                        ..Default::default()
                    },
                ),
                ("1:1".into(), Annotation::default()),
            ]),
        }
    }
    fn edit(data: &mut Annotations, key: &str, action: Action) -> Result<()> {
        data.mutate(Mutation {
            revision: data.revision,
            target: key.into(),
            actor: Actor {
                name: "Reviewer".into(),
                email: "reviewer@example.org".into(),
            },
            action,
        })
    }
    fn forward(data: &mut Annotations, key: &str, status: ReviewStatus) -> Result<()> {
        edit(
            data,
            key,
            Action::Transition {
                status,
                assignments: Assignments {
                    sme_email: Some("sme@example.org".into()),
                    owner_email: Some("owner@example.org".into()),
                    eta: Some(NaiveDate::from_ymd_opt(2020, 1, 1).unwrap()),
                },
            },
        )
    }
    #[test]
    fn every_transition_and_undo_path() {
        use ReviewStatus::*;
        let paths = [
            vec![Review, Reviewed, NoOpportunity],
            vec![
                Review,
                Reviewed,
                GetSmeFeedback,
                PendingSmeFeedback,
                NoOpportunity,
            ],
            vec![
                Review,
                Reviewed,
                GetSmeFeedback,
                PendingSmeFeedback,
                ToBeAddressed,
                InProgress,
                Solved,
            ],
        ];
        let all = [
            Review,
            Reviewed,
            NoOpportunity,
            GetSmeFeedback,
            PendingSmeFeedback,
            ToBeAddressed,
            InProgress,
            Solved,
        ];
        for path in paths {
            let mut data = data();
            for state in &path {
                if *state != Review {
                    forward(&mut data, "1", *state).unwrap();
                }
                for next in all {
                    let mut copy = data.clone();
                    assert_eq!(
                        forward(&mut copy, "1", next).is_ok(),
                        state.next().contains(&next)
                    );
                }
            }
            for expected in path.iter().rev().skip(1) {
                edit(&mut data, "1", Action::Undo).unwrap();
                assert_eq!(data.effective("1").unwrap().status, *expected);
            }
            assert!(edit(&mut data, "1", Action::Undo).is_err());
            assert!(data.effective("1").unwrap().assignments.eta.is_some());
        }
    }
    #[test]
    fn inheritance_override_and_comments() {
        let mut data = data();
        forward(&mut data, "1", ReviewStatus::Reviewed).unwrap();
        assert_eq!(
            data.effective("1:1").unwrap().status,
            ReviewStatus::Reviewed
        );
        forward(&mut data, "1:1", ReviewStatus::NoOpportunity).unwrap();
        edit(&mut data, "1", Action::Undo).unwrap();
        assert_eq!(
            data.effective("1:1").unwrap().status,
            ReviewStatus::NoOpportunity
        );
        edit(&mut data, "1:1", Action::Inherit).unwrap();
        assert_eq!(data.effective("1:1").unwrap().status, ReviewStatus::Review);
        let count = data.entries["1:1"].history.len();
        edit(
            &mut data,
            "1:1",
            Action::AddComment {
                text: "Test".into(),
            },
        )
        .unwrap();
        let comment = data.entries["1:1"].comments[0].clone();
        edit(
            &mut data,
            "1:1",
            Action::EditComment {
                id: comment.id.clone(),
                text: "Updated".into(),
            },
        )
        .unwrap();
        assert_eq!(data.entries["1:1"].comments[0].author, comment.author);
        edit(&mut data, "1:1", Action::DeleteComment { id: comment.id }).unwrap();
        assert_eq!(data.entries["1:1"].history.len(), count);
        assert!(data.entries["1:1"].workflow.is_none());
    }
    #[test]
    fn required_fields_fail_atomically() {
        let mut data = data();
        forward(&mut data, "1", ReviewStatus::Reviewed).unwrap();
        forward(&mut data, "1", ReviewStatus::GetSmeFeedback).unwrap();
        let before = data.clone();
        assert!(edit(
            &mut data,
            "1:1",
            Action::Transition {
                status: ReviewStatus::PendingSmeFeedback,
                assignments: Assignments::default()
            }
        )
        .is_err());
        assert_eq!(data, before);
    }
}
