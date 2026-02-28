use super::*;
use serde_json::Map;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IndexedCommand {
    command: String,
    exit_code: Option<i64>,
}

#[derive(Clone, Debug)]
struct IndexedSummaryNode {
    node_id: String,
    parent_node_id: Option<String>,
    node_type: String,
    title: Option<String>,
    node_json: String,
    file_paths: Vec<String>,
    commands: Vec<IndexedCommand>,
    errors: Vec<String>,
}

impl StateRuntime {
    pub async fn upsert_session_summary(
        &self,
        params: &SessionSummaryPersistParams,
    ) -> anyhow::Result<SessionSummaryArtifact> {
        let summary_id = params
            .summary_id
            .clone()
            .unwrap_or_else(|| format!("{}:{}", params.thread_id, params.session_id));
        let summary_path = summary_path_for_thread_and_session(
            self.codex_home(),
            params.thread_id.as_str(),
            params.session_id.as_str(),
        );
        let summary_parent = summary_path.parent().ok_or_else(|| {
            anyhow::anyhow!("summary path has no parent: {}", summary_path.display())
        })?;
        tokio::fs::create_dir_all(summary_parent).await?;
        let summary_bytes = serde_json::to_vec_pretty(&params.summary)?;
        tokio::fs::write(&summary_path, summary_bytes).await?;

        let indexed_nodes = index_summary_nodes(&params.summary)?;
        let root_node_id = params
            .root_node_id
            .clone()
            .or_else(|| indexed_nodes.first().map(|node| node.node_id.clone()));
        let now = Utc::now().timestamp();
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            r#"
INSERT INTO summary_artifacts (
    summary_id,
    thread_id,
    session_id,
    schema_version,
    summary_path,
    root_node_id,
    created_at,
    updated_at
) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
ON CONFLICT(summary_id) DO UPDATE SET
    thread_id = excluded.thread_id,
    session_id = excluded.session_id,
    schema_version = excluded.schema_version,
    summary_path = excluded.summary_path,
    root_node_id = excluded.root_node_id,
    updated_at = excluded.updated_at
            "#,
        )
        .bind(summary_id.as_str())
        .bind(params.thread_id.as_str())
        .bind(params.session_id.as_str())
        .bind(params.schema_version.as_str())
        .bind(summary_path.display().to_string())
        .bind(root_node_id.as_deref())
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM summary_nodes WHERE summary_id = ?")
            .bind(summary_id.as_str())
            .execute(&mut *tx)
            .await?;

        for node in &indexed_nodes {
            sqlx::query(
                r#"
INSERT INTO summary_nodes (
    summary_id,
    node_id,
    parent_node_id,
    node_type,
    title,
    node_json
) VALUES (?, ?, ?, ?, ?, ?)
                "#,
            )
            .bind(summary_id.as_str())
            .bind(node.node_id.as_str())
            .bind(node.parent_node_id.as_deref())
            .bind(node.node_type.as_str())
            .bind(node.title.as_deref())
            .bind(node.node_json.as_str())
            .execute(&mut *tx)
            .await?;

            for file_path in &node.file_paths {
                sqlx::query(
                    r#"
INSERT INTO summary_node_file_paths (
    summary_id,
    node_id,
    file_path
) VALUES (?, ?, ?)
                    "#,
                )
                .bind(summary_id.as_str())
                .bind(node.node_id.as_str())
                .bind(file_path.as_str())
                .execute(&mut *tx)
                .await?;
            }
            for command in &node.commands {
                sqlx::query(
                    r#"
INSERT INTO summary_node_commands (
    summary_id,
    node_id,
    command,
    exit_code
) VALUES (?, ?, ?, ?)
                    "#,
                )
                .bind(summary_id.as_str())
                .bind(node.node_id.as_str())
                .bind(command.command.as_str())
                .bind(command.exit_code)
                .execute(&mut *tx)
                .await?;
            }
            for error_text in &node.errors {
                sqlx::query(
                    r#"
INSERT INTO summary_node_errors (
    summary_id,
    node_id,
    error_text
) VALUES (?, ?, ?)
                    "#,
                )
                .bind(summary_id.as_str())
                .bind(node.node_id.as_str())
                .bind(error_text.as_str())
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        self.get_session_summary_artifact(summary_id.as_str())
            .await?
            .ok_or_else(|| anyhow::anyhow!("stored summary is missing from index: {summary_id}"))
    }

    pub async fn get_session_summary_artifact(
        &self,
        summary_id: &str,
    ) -> anyhow::Result<Option<SessionSummaryArtifact>> {
        let row = sqlx::query(
            r#"
SELECT
    summary_id,
    thread_id,
    session_id,
    schema_version,
    summary_path,
    root_node_id,
    created_at,
    updated_at
FROM summary_artifacts
WHERE summary_id = ?
            "#,
        )
        .bind(summary_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        row.map(|row| session_summary_artifact_from_row(&row))
            .transpose()
    }

    pub async fn get_session_summary_by_thread_and_session(
        &self,
        thread_id: &str,
        session_id: &str,
    ) -> anyhow::Result<Option<SessionSummaryArtifact>> {
        let row = sqlx::query(
            r#"
SELECT
    summary_id,
    thread_id,
    session_id,
    schema_version,
    summary_path,
    root_node_id,
    created_at,
    updated_at
FROM summary_artifacts
WHERE thread_id = ? AND session_id = ?
            "#,
        )
        .bind(thread_id)
        .bind(session_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        row.map(|row| session_summary_artifact_from_row(&row))
            .transpose()
    }

    pub async fn get_latest_session_summary_by_thread(
        &self,
        thread_id: &str,
    ) -> anyhow::Result<Option<SessionSummaryArtifact>> {
        let row = sqlx::query(
            r#"
SELECT
    summary_id,
    thread_id,
    session_id,
    schema_version,
    summary_path,
    root_node_id,
    created_at,
    updated_at
FROM summary_artifacts
WHERE thread_id = ?
ORDER BY updated_at DESC
LIMIT 1
            "#,
        )
        .bind(thread_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        row.map(|row| session_summary_artifact_from_row(&row))
            .transpose()
    }

    pub async fn get_latest_session_summary_by_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<SessionSummaryArtifact>> {
        let row = sqlx::query(
            r#"
SELECT
    summary_id,
    thread_id,
    session_id,
    schema_version,
    summary_path,
    root_node_id,
    created_at,
    updated_at
FROM summary_artifacts
WHERE session_id = ?
ORDER BY updated_at DESC
LIMIT 1
            "#,
        )
        .bind(session_id)
        .fetch_optional(self.pool.as_ref())
        .await?;
        row.map(|row| session_summary_artifact_from_row(&row))
            .transpose()
    }

    pub async fn read_session_summary_by_thread_and_session(
        &self,
        thread_id: &str,
        session_id: &str,
    ) -> anyhow::Result<Option<Value>> {
        let artifact = self
            .get_session_summary_by_thread_and_session(thread_id, session_id)
            .await?;
        read_summary_json_from_artifact(artifact.as_ref()).await
    }

    pub async fn read_latest_session_summary_by_thread(
        &self,
        thread_id: &str,
    ) -> anyhow::Result<Option<Value>> {
        let artifact = self.get_latest_session_summary_by_thread(thread_id).await?;
        read_summary_json_from_artifact(artifact.as_ref()).await
    }

    pub async fn read_latest_session_summary_by_session(
        &self,
        session_id: &str,
    ) -> anyhow::Result<Option<Value>> {
        let artifact = self
            .get_latest_session_summary_by_session(session_id)
            .await?;
        read_summary_json_from_artifact(artifact.as_ref()).await
    }

    pub async fn list_summary_nodes_by_thread_and_session(
        &self,
        thread_id: &str,
        session_id: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummaryNodeMatch>> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = sqlx::query(
            r#"
SELECT
    artifacts.summary_id,
    artifacts.thread_id,
    artifacts.session_id,
    artifacts.summary_path,
    nodes.node_id,
    nodes.parent_node_id,
    nodes.node_type,
    nodes.title,
    nodes.node_json,
    NULL AS matched_file_path,
    NULL AS matched_command,
    NULL AS matched_error_text
FROM summary_nodes AS nodes
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = nodes.summary_id
WHERE artifacts.thread_id = ? AND artifacts.session_id = ?
ORDER BY nodes.node_id ASC
LIMIT ?
            "#,
        )
        .bind(thread_id)
        .bind(session_id)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter().map(summary_node_match_from_row).collect()
    }

    pub async fn search_summary_nodes_by_file_path(
        &self,
        file_path_substring: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummaryNodeMatch>> {
        if file_path_substring.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = sqlx::query(
            r#"
SELECT
    artifacts.summary_id,
    artifacts.thread_id,
    artifacts.session_id,
    artifacts.summary_path,
    nodes.node_id,
    nodes.parent_node_id,
    nodes.node_type,
    nodes.title,
    nodes.node_json,
    file_paths.file_path AS matched_file_path,
    NULL AS matched_command,
    NULL AS matched_error_text
FROM summary_node_file_paths AS file_paths
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = file_paths.summary_id
    AND nodes.node_id = file_paths.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = nodes.summary_id
WHERE INSTR(LOWER(file_paths.file_path), LOWER(?)) > 0
ORDER BY artifacts.updated_at DESC, nodes.node_id ASC
LIMIT ?
            "#,
        )
        .bind(file_path_substring)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter().map(summary_node_match_from_row).collect()
    }

    pub async fn search_summary_nodes_by_command_substring(
        &self,
        command_substring: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummaryNodeMatch>> {
        if command_substring.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = sqlx::query(
            r#"
SELECT
    artifacts.summary_id,
    artifacts.thread_id,
    artifacts.session_id,
    artifacts.summary_path,
    nodes.node_id,
    nodes.parent_node_id,
    nodes.node_type,
    nodes.title,
    nodes.node_json,
    NULL AS matched_file_path,
    commands.command AS matched_command,
    NULL AS matched_error_text
FROM summary_node_commands AS commands
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = commands.summary_id
    AND nodes.node_id = commands.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = nodes.summary_id
WHERE INSTR(LOWER(commands.command), LOWER(?)) > 0
ORDER BY artifacts.updated_at DESC, nodes.node_id ASC
LIMIT ?
            "#,
        )
        .bind(command_substring)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter().map(summary_node_match_from_row).collect()
    }

    pub async fn search_summary_nodes_by_error_text(
        &self,
        error_text_substring: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummaryNodeMatch>> {
        if error_text_substring.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = sqlx::query(
            r#"
SELECT
    artifacts.summary_id,
    artifacts.thread_id,
    artifacts.session_id,
    artifacts.summary_path,
    nodes.node_id,
    nodes.parent_node_id,
    nodes.node_type,
    nodes.title,
    nodes.node_json,
    NULL AS matched_file_path,
    NULL AS matched_command,
    errors.error_text AS matched_error_text
FROM summary_node_errors AS errors
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = errors.summary_id
    AND nodes.node_id = errors.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = nodes.summary_id
WHERE INSTR(LOWER(errors.error_text), LOWER(?)) > 0
ORDER BY artifacts.updated_at DESC, nodes.node_id ASC
LIMIT ?
            "#,
        )
        .bind(error_text_substring)
        .bind(limit)
        .fetch_all(self.pool.as_ref())
        .await?;
        rows.iter().map(summary_node_match_from_row).collect()
    }
}

fn summary_path_for_thread_and_session(
    codex_home: &Path,
    thread_id: &str,
    session_id: &str,
) -> PathBuf {
    codex_home
        .join("agentcanvas")
        .join("summaries")
        .join(sanitize_path_component(thread_id))
        .join(format!("{}.json", sanitize_path_component(session_id)))
}

fn sanitize_path_component(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "_".to_string();
    }
    out
}

fn session_summary_artifact_from_row(row: &SqliteRow) -> anyhow::Result<SessionSummaryArtifact> {
    Ok(SessionSummaryArtifact {
        summary_id: row.try_get("summary_id")?,
        thread_id: row.try_get("thread_id")?,
        session_id: row.try_get("session_id")?,
        schema_version: row.try_get("schema_version")?,
        summary_path: PathBuf::from(row.try_get::<String, _>("summary_path")?),
        root_node_id: row.try_get("root_node_id")?,
        created_at: row.try_get("created_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn read_summary_json_from_artifact(
    artifact: Option<&SessionSummaryArtifact>,
) -> anyhow::Result<Option<Value>> {
    let Some(artifact) = artifact else {
        return Ok(None);
    };
    let summary_json = match tokio::fs::read_to_string(&artifact.summary_path).await {
        Ok(content) => content,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(err) => {
            return Err(err.into());
        }
    };
    let summary = serde_json::from_str::<Value>(summary_json.as_str())?;
    Ok(Some(summary))
}

fn summary_node_match_from_row(row: &SqliteRow) -> anyhow::Result<SessionSummaryNodeMatch> {
    let node_json: String = row.try_get("node_json")?;
    Ok(SessionSummaryNodeMatch {
        summary_id: row.try_get("summary_id")?,
        thread_id: row.try_get("thread_id")?,
        session_id: row.try_get("session_id")?,
        summary_path: PathBuf::from(row.try_get::<String, _>("summary_path")?),
        node_id: row.try_get("node_id")?,
        parent_node_id: row.try_get("parent_node_id")?,
        node_type: row.try_get("node_type")?,
        title: row.try_get("title")?,
        node: serde_json::from_str(node_json.as_str())?,
        matched_file_path: row.try_get("matched_file_path")?,
        matched_command: row.try_get("matched_command")?,
        matched_error_text: row.try_get("matched_error_text")?,
    })
}

fn index_summary_nodes(summary: &Value) -> anyhow::Result<Vec<IndexedSummaryNode>> {
    let mut indexed_nodes = Vec::new();
    collect_indexed_nodes(summary, None, "root", &mut indexed_nodes)?;
    Ok(indexed_nodes)
}

fn collect_indexed_nodes(
    value: &Value,
    inherited_parent_id: Option<&str>,
    auto_node_id: &str,
    indexed_nodes: &mut Vec<IndexedSummaryNode>,
) -> anyhow::Result<()> {
    match value {
        Value::Array(values) => {
            for (idx, child) in values.iter().enumerate() {
                collect_indexed_nodes(
                    child,
                    inherited_parent_id,
                    format!("{auto_node_id}.{idx}").as_str(),
                    indexed_nodes,
                )?;
            }
            Ok(())
        }
        Value::Object(object) => {
            if is_summary_node_object(object) {
                let node_id = node_id_for_object(object, auto_node_id);
                let mut file_paths = BTreeSet::new();
                let mut commands = BTreeSet::new();
                let mut errors = BTreeSet::new();
                collect_node_evidence(value, None, &mut file_paths, &mut commands, &mut errors);
                let node_json = serde_json::to_string(value)?;
                indexed_nodes.push(IndexedSummaryNode {
                    node_id: node_id.clone(),
                    parent_node_id: string_value(object, &["parent_id", "parentId"])
                        .or_else(|| inherited_parent_id.map(ToOwned::to_owned)),
                    node_type: string_value(object, &["node_type", "nodeType", "type"])
                        .unwrap_or_else(|| "unknown".to_string()),
                    title: string_value(object, &["title", "label", "summary"]),
                    node_json,
                    file_paths: file_paths.into_iter().collect(),
                    commands: commands.into_iter().collect(),
                    errors: errors.into_iter().collect(),
                });
                for (idx, child) in summary_child_nodes(object).iter().enumerate() {
                    collect_indexed_nodes(
                        child,
                        Some(node_id.as_str()),
                        format!("{auto_node_id}.{idx}").as_str(),
                        indexed_nodes,
                    )?;
                }
                return Ok(());
            }
            for (idx, child) in object.values().enumerate() {
                collect_indexed_nodes(
                    child,
                    inherited_parent_id,
                    format!("{auto_node_id}.{idx}").as_str(),
                    indexed_nodes,
                )?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => Ok(()),
    }
}

fn summary_child_nodes(object: &Map<String, Value>) -> Vec<&Value> {
    let mut child_nodes = Vec::new();
    for key in ["children", "nodes", "turns", "child_nodes", "childNodes"] {
        if let Some(value) = object.get(key) {
            match value {
                Value::Array(values) => child_nodes.extend(values.iter()),
                Value::Object(_) => child_nodes.push(value),
                Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
            }
        }
    }
    child_nodes
}

fn is_summary_node_object(object: &Map<String, Value>) -> bool {
    if string_value(object, &["node_type", "nodeType"]).is_some() {
        return true;
    }
    if string_value(object, &["node_id", "nodeId"]).is_some() {
        return true;
    }
    string_value(object, &["id"]).is_some()
        && (object.contains_key("parent_id")
            || object.contains_key("parentId")
            || object.contains_key("children")
            || object.contains_key("nodes")
            || object.contains_key("evidence"))
}

fn node_id_for_object(object: &Map<String, Value>, fallback_id: &str) -> String {
    string_value(object, &["node_id", "nodeId", "id"])
        .unwrap_or_else(|| format!("auto-node-{fallback_id}"))
}

fn collect_node_evidence(
    value: &Value,
    key_hint: Option<&str>,
    file_paths: &mut BTreeSet<String>,
    commands: &mut BTreeSet<IndexedCommand>,
    errors: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let key_lower = key.to_ascii_lowercase();
                collect_node_evidence(
                    child,
                    Some(key_lower.as_str()),
                    file_paths,
                    commands,
                    errors,
                );
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_node_evidence(child, key_hint, file_paths, commands, errors);
            }
        }
        Value::String(text) => {
            let Some(key_hint) = key_hint else {
                return;
            };
            if is_file_path_key(key_hint) && looks_like_file_path(text) {
                file_paths.insert(text.to_string());
            }
            if is_command_key(key_hint) {
                commands.insert(IndexedCommand {
                    command: text.to_string(),
                    exit_code: None,
                });
            }
            if is_error_key(key_hint) {
                errors.insert(text.to_string());
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_file_path_key(key: &str) -> bool {
    matches!(
        key,
        "file" | "files" | "file_path" | "file_paths" | "filepath" | "filepaths" | "path" | "paths"
    )
}

fn is_command_key(key: &str) -> bool {
    matches!(key, "command" | "commands" | "cmd")
}

fn is_error_key(key: &str) -> bool {
    matches!(
        key,
        "error" | "errors" | "error_text" | "errortext" | "last_error" | "stderr"
    )
}

fn looks_like_file_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains('\n')
        && (value.contains('/')
            || value.contains('\\')
            || value.starts_with("./")
            || value.starts_with("../"))
}

fn string_value(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(Value::String(value)) = object.get(*key) {
            return Some(value.clone());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::test_support::unique_temp_dir;
    use super::*;
    use pretty_assertions::assert_eq;
    use serde_json::json;

    fn sample_summary_with_evidence() -> Value {
        json!({
            "schema_version": "v1",
            "nodes": [
                {
                    "node_id": "turn-1",
                    "node_type": "turn",
                    "title": "Initial turn",
                    "evidence": {
                        "file_paths": ["/repo/src/main.rs"],
                        "commands": [{"command": "cargo test -p codex-state", "exit_code": 0}],
                        "errors": ["build failed"]
                    },
                    "children": [
                        {
                            "node_id": "turn-1-child",
                            "node_type": "code_changes",
                            "title": "Edited files",
                            "evidence": {
                                "files": ["codex-rs/state/src/runtime/summaries.rs"],
                                "command": "cargo fmt"
                            }
                        }
                    ]
                }
            ]
        })
    }

    #[tokio::test]
    async fn upsert_session_summary_persists_and_reads_by_thread_and_session() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");
        let summary = sample_summary_with_evidence();

        let artifact = runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-a".to_string(),
                session_id: "session-a".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: summary.clone(),
            })
            .await
            .expect("upsert summary");

        let expected_summary_path = codex_home
            .join("agentcanvas")
            .join("summaries")
            .join("thread-a")
            .join("session-a.json");
        assert_eq!(artifact.summary_id, "thread-a:session-a");
        assert_eq!(artifact.thread_id, "thread-a");
        assert_eq!(artifact.session_id, "session-a");
        assert_eq!(artifact.schema_version, "v1");
        assert_eq!(artifact.root_node_id, Some("turn-1".to_string()));
        assert_eq!(artifact.summary_path, expected_summary_path);

        let read_summary = runtime
            .read_session_summary_by_thread_and_session("thread-a", "session-a")
            .await
            .expect("read stored summary");
        assert_eq!(read_summary, Some(summary));

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn summary_search_finds_nodes_by_file_command_and_error() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-a".to_string(),
                session_id: "session-a".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: sample_summary_with_evidence(),
            })
            .await
            .expect("upsert summary");

        let file_matches = runtime
            .search_summary_nodes_by_file_path("src/main.rs", 10)
            .await
            .expect("search by file path");
        assert_eq!(file_matches.len(), 1);
        assert_eq!(file_matches[0].node_id, "turn-1");
        assert_eq!(
            file_matches[0].matched_file_path,
            Some("/repo/src/main.rs".to_string())
        );

        let command_matches = runtime
            .search_summary_nodes_by_command_substring("cargo test -p codex-state", 10)
            .await
            .expect("search by command");
        assert_eq!(command_matches.len(), 1);
        assert_eq!(command_matches[0].node_id, "turn-1");
        assert_eq!(
            command_matches[0].matched_command,
            Some("cargo test -p codex-state".to_string())
        );

        let error_matches = runtime
            .search_summary_nodes_by_error_text("build failed", 10)
            .await
            .expect("search by error");
        assert_eq!(error_matches.len(), 1);
        assert_eq!(error_matches[0].node_id, "turn-1");
        assert_eq!(
            error_matches[0].matched_error_text,
            Some("build failed".to_string())
        );

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn upsert_session_summary_replaces_existing_node_index_without_duplicates() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");

        let first_summary = json!({
            "nodes": [
                {
                    "node_id": "n1",
                    "node_type": "execution",
                    "evidence": { "command": "cargo check" }
                },
                {
                    "node_id": "n2",
                    "node_type": "execution",
                    "evidence": { "error": "first error" }
                }
            ]
        });
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-a".to_string(),
                session_id: "session-a".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: first_summary,
            })
            .await
            .expect("upsert first summary");

        let second_summary = json!({
            "nodes": [
                {
                    "node_id": "n1",
                    "node_type": "execution",
                    "evidence": { "command": "cargo test" }
                },
                {
                    "node_id": "n3",
                    "node_type": "code_changes",
                    "evidence": { "file_path": "/repo/src/lib.rs" }
                }
            ]
        });
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-a".to_string(),
                session_id: "session-a".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: second_summary,
            })
            .await
            .expect("upsert second summary");

        let old_command_matches = runtime
            .search_summary_nodes_by_command_substring("cargo check", 10)
            .await
            .expect("search old command");
        assert_eq!(old_command_matches, Vec::new());

        let new_command_matches = runtime
            .search_summary_nodes_by_command_substring("cargo test", 10)
            .await
            .expect("search new command");
        assert_eq!(new_command_matches.len(), 1);
        assert_eq!(new_command_matches[0].node_id, "n1");

        let nodes = runtime
            .list_summary_nodes_by_thread_and_session("thread-a", "session-a", 10)
            .await
            .expect("list nodes");
        let node_ids: Vec<String> = nodes.into_iter().map(|node| node.node_id).collect();
        assert_eq!(node_ids, vec!["n1".to_string(), "n3".to_string()]);

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }
}
