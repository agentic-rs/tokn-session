use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectIdentity {
  pub(crate) id: Option<String>,
  pub(crate) name: String,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ProjectCatalog {
  projects: Vec<CatalogProject>,
  project_indices: HashMap<String, usize>,
  assignments: HashMap<String, String>,
  workspace_labels: Vec<WorkspaceLabel>,
}

impl ProjectCatalog {
  pub(crate) fn load(path: &Path) -> Result<Self, String> {
    let bytes = std::fs::read(path)
      .map_err(|err| format!("failed to read Codex Desktop project catalog {}: {err}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|err| {
      format!(
        "failed to parse Codex Desktop project catalog {}: {err}",
        path.display()
      )
    })?;
    Ok(Self::from_value(&value))
  }

  pub(crate) fn from_value(value: &Value) -> Self {
    let workspace_labels = parse_workspace_labels(value);
    let mut catalog = Self {
      workspace_labels,
      ..Self::default()
    };

    if let Some(projects) = value.get("local-projects").and_then(Value::as_object) {
      for (map_id, value) in projects {
        let Some(project) = parse_project(map_id, value) else {
          continue;
        };
        let index = catalog.projects.len();
        if let Some(id) = project.id.as_deref() {
          catalog.project_indices.insert(id.to_string(), index);
        }
        if !map_id.is_empty() {
          catalog.project_indices.insert(map_id.clone(), index);
        }
        catalog.projects.push(project);
      }
    }

    if let Some(assignments) = value.get("thread-project-assignments").and_then(Value::as_object) {
      for (session_id, value) in assignments {
        let Some(assignment) = value.as_object() else {
          continue;
        };
        if assignment
          .get("projectKind")
          .and_then(non_empty_string)
          .is_some_and(|kind| kind != "local")
        {
          continue;
        }
        if let Some(project_id) = assignment.get("projectId").and_then(non_empty_string) {
          catalog.assignments.insert(session_id.clone(), project_id.to_string());
        }
      }
    }

    catalog
  }

  pub(crate) fn resolve(
    &self,
    session_id: &str,
    parent_session_id: Option<&str>,
    cwd: Option<&str>,
  ) -> Option<ProjectIdentity> {
    self
      .resolve_assignment(session_id)
      .or_else(|| parent_session_id.and_then(|parent_id| self.resolve_assignment(parent_id)))
      .or_else(|| cwd.and_then(|cwd| self.resolve_cwd(Path::new(cwd))))
  }

  fn resolve_assignment(&self, session_id: &str) -> Option<ProjectIdentity> {
    let project_id = self.assignments.get(session_id)?;
    let project = self.project(project_id)?;
    self.identity_for_project(project)
  }

  fn resolve_cwd(&self, cwd: &Path) -> Option<ProjectIdentity> {
    let mut best: Option<RootMatch> = None;

    for project in &self.projects {
      let Some(identity) = self.identity_for_project(project) else {
        continue;
      };
      for root in &project.roots {
        if cwd.starts_with(root) {
          choose_root_match(
            &mut best,
            RootMatch {
              components: root.components().count(),
              source_priority: 1,
              tie_breaker: root.to_string_lossy().into_owned(),
              identity: identity.clone(),
            },
          );
        }
      }
    }

    for label in &self.workspace_labels {
      if cwd.starts_with(&label.root) {
        choose_root_match(
          &mut best,
          RootMatch {
            components: label.root.components().count(),
            source_priority: 0,
            tie_breaker: label.root.to_string_lossy().into_owned(),
            identity: ProjectIdentity {
              id: self.project_id_for_root(&label.root),
              name: label.name.clone(),
            },
          },
        );
      }
    }

    best.map(|matched| matched.identity)
  }

  fn project(&self, id: &str) -> Option<&CatalogProject> {
    self.project_indices.get(id).and_then(|index| self.projects.get(*index))
  }

  fn identity_for_project(&self, project: &CatalogProject) -> Option<ProjectIdentity> {
    let name = project.name.clone().or_else(|| {
      project.roots.iter().find_map(|root| {
        self
          .workspace_labels
          .iter()
          .find(|label| label.root == *root)
          .map(|label| label.name.clone())
      })
    })?;
    Some(ProjectIdentity {
      id: project.id.clone(),
      name,
    })
  }

  fn project_id_for_root(&self, root: &Path) -> Option<String> {
    self
      .projects
      .iter()
      .find(|project| project.roots.iter().any(|project_root| project_root == root))
      .and_then(|project| project.id.clone())
  }
}

#[derive(Clone, Debug)]
struct CatalogProject {
  id: Option<String>,
  name: Option<String>,
  roots: Vec<PathBuf>,
}

#[derive(Clone, Debug)]
struct WorkspaceLabel {
  root: PathBuf,
  name: String,
}

struct RootMatch {
  components: usize,
  source_priority: u8,
  tie_breaker: String,
  identity: ProjectIdentity,
}

fn parse_project(map_id: &str, value: &Value) -> Option<CatalogProject> {
  let project = value.as_object()?;
  let id = project
    .get("id")
    .and_then(non_empty_string)
    .or_else(|| (!map_id.is_empty()).then_some(map_id))
    .map(str::to_string);
  let name = project.get("name").and_then(non_empty_string).map(str::to_string);
  let roots = project
    .get("rootPaths")
    .and_then(Value::as_array)
    .into_iter()
    .flatten()
    .filter_map(non_empty_string)
    .map(PathBuf::from)
    .collect::<Vec<_>>();

  (id.is_some() || name.is_some() || !roots.is_empty()).then_some(CatalogProject { id, name, roots })
}

fn parse_workspace_labels(value: &Value) -> Vec<WorkspaceLabel> {
  value
    .get("electron-workspace-root-labels")
    .and_then(Value::as_object)
    .into_iter()
    .flatten()
    .filter_map(|(root, value)| {
      let name = non_empty_string(value)?;
      (!root.is_empty()).then(|| WorkspaceLabel {
        root: PathBuf::from(root),
        name: name.to_string(),
      })
    })
    .collect()
}

fn non_empty_string(value: &Value) -> Option<&str> {
  value.as_str().filter(|value| !value.is_empty())
}

fn choose_root_match(best: &mut Option<RootMatch>, candidate: RootMatch) {
  let replace = best.as_ref().is_none_or(|current| {
    (
      candidate.components,
      candidate.source_priority,
      candidate.tie_breaker.as_str(),
    ) > (
      current.components,
      current.source_priority,
      current.tie_breaker.as_str(),
    )
  });
  if replace {
    *best = Some(candidate);
  }
}

#[cfg(test)]
mod tests {
  use serde_json::json;
  use tempfile::TempDir;

  use super::{ProjectCatalog, ProjectIdentity};

  #[test]
  fn resolves_session_then_parent_before_cwd() {
    let catalog = ProjectCatalog::from_value(&json!({
      "local-projects": {
        "direct": {
          "id": "direct",
          "name": "direct-project",
          "rootPaths": ["/work/shared"]
        },
        "parent": {
          "id": "parent",
          "name": "parent-project",
          "rootPaths": ["/work/parent"]
        },
        "cwd": {
          "id": "cwd",
          "name": "cwd-project",
          "rootPaths": ["/work/cwd"]
        }
      },
      "thread-project-assignments": {
        "session": {
          "projectKind": "local",
          "projectId": "direct"
        },
        "parent-session": {
          "projectKind": "local",
          "projectId": "parent"
        }
      }
    }));

    assert_eq!(
      catalog.resolve("session", Some("parent-session"), Some("/work/cwd/src")),
      Some(ProjectIdentity {
        id: Some("direct".to_string()),
        name: "direct-project".to_string(),
      })
    );
    assert_eq!(
      catalog.resolve("child", Some("parent-session"), Some("/work/cwd/src")),
      Some(ProjectIdentity {
        id: Some("parent".to_string()),
        name: "parent-project".to_string(),
      })
    );
  }

  #[test]
  fn chooses_longest_component_boundary_root() {
    let catalog = ProjectCatalog::from_value(&json!({
      "local-projects": {
        "outer": {
          "name": "outer",
          "rootPaths": ["/work/llm-router"]
        },
        "inner": {
          "name": "inner",
          "rootPaths": ["/work/llm-router/crates/special"]
        }
      }
    }));

    assert_eq!(
      catalog.resolve("", None, Some("/work/llm-router/crates/special/src")),
      Some(ProjectIdentity {
        id: Some("inner".to_string()),
        name: "inner".to_string(),
      })
    );
    assert_eq!(catalog.resolve("", None, Some("/work/llm-router-other")), None);
  }

  #[test]
  fn uses_workspace_labels_as_a_tolerant_fallback() {
    let catalog = ProjectCatalog::from_value(&json!({
      "local-projects": {
        "labeled": {
          "rootPaths": ["/work/labeled"]
        },
        "named": {
          "name": "catalog-name",
          "rootPaths": ["/work/named"]
        }
      },
      "electron-workspace-root-labels": {
        "/work/labeled": "labeled_2",
        "/work/named": "label-does-not-override",
        "/work/label-only": "standalone_3"
      }
    }));

    assert_eq!(
      catalog.resolve("", None, Some("/work/labeled/src")),
      Some(ProjectIdentity {
        id: Some("labeled".to_string()),
        name: "labeled_2".to_string(),
      })
    );
    assert_eq!(
      catalog.resolve("", None, Some("/work/named")),
      Some(ProjectIdentity {
        id: Some("named".to_string()),
        name: "catalog-name".to_string(),
      })
    );
    assert_eq!(
      catalog.resolve("", None, Some("/work/label-only/src")),
      Some(ProjectIdentity {
        id: None,
        name: "standalone_3".to_string(),
      })
    );
  }

  #[test]
  fn tolerates_missing_and_wrongly_typed_fields() {
    let catalog = ProjectCatalog::from_value(&json!({
      "local-projects": {
        "empty": null,
        "valid": {
          "name": "valid-project",
          "rootPaths": "not-an-array"
        }
      },
      "thread-project-assignments": {
        "session": {
          "projectId": "valid"
        },
        "remote": {
          "projectKind": "remote",
          "projectId": "valid"
        },
        "broken": []
      },
      "electron-workspace-root-labels": {
        "/ignored": 42
      }
    }));

    assert_eq!(
      catalog.resolve("session", None, None),
      Some(ProjectIdentity {
        id: Some("valid".to_string()),
        name: "valid-project".to_string(),
      })
    );
    assert_eq!(catalog.resolve("remote", None, None), None);
    assert_eq!(catalog.resolve("broken", None, Some("/ignored")), None);
  }

  #[test]
  fn loads_catalog_from_disk_and_reports_invalid_json() {
    let fixture = TempDir::new().unwrap();
    let valid = fixture.path().join("state.json");
    std::fs::write(
      &valid,
      r#"{"local-projects":{"project":{"name":"from-disk"}},"thread-project-assignments":{"session":{"projectId":"project"}}}"#,
    )
    .unwrap();
    let catalog = ProjectCatalog::load(&valid).unwrap();
    assert_eq!(
      catalog.resolve("session", None, None),
      Some(ProjectIdentity {
        id: Some("project".to_string()),
        name: "from-disk".to_string(),
      })
    );

    let invalid = fixture.path().join("invalid.json");
    std::fs::write(&invalid, "{not json").unwrap();
    assert!(ProjectCatalog::load(&invalid).is_err());
    assert!(ProjectCatalog::load(&fixture.path().join("missing.json")).is_err());
  }
}
