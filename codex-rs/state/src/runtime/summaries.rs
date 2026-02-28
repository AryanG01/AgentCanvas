use super::*;
use reqwest::StatusCode;
use serde::Deserialize;
use serde::Serialize;
use serde_json::Map;
use serde_json::Value;
use sqlx::sqlite::SqliteRow;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;

const SUMMARY_SEMANTIC_EMBEDDING_MODEL: &str = "jina-embeddings-v5-text-nano";
const SUMMARY_SEMANTIC_JINA_API_KEY_ENV: &str = "JINA_API_KEY";
const SUMMARY_SEMANTIC_JINA_API_URL_ENV: &str = "JINA_EMBEDDINGS_API_URL";
const SUMMARY_SEMANTIC_JINA_DEFAULT_API_URL: &str = "https://api.jina.ai/v1/embeddings";
const HYBRID_SEMANTIC_WEIGHT: f64 = 0.7;
const HYBRID_LEXICAL_WEIGHT: f64 = 0.3;
const SEMANTIC_COVERAGE_BASE_WEIGHT: f64 = 0.6;
const LEXICAL_COVERAGE_BASE_WEIGHT: f64 = 0.5;
const LEXICAL_COMMAND_FAILURE_WEIGHT: f64 = 1.25;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IndexedCommand {
    command: String,
    exit_code: Option<i64>,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct IndexedSemanticTerm {
    term_hash: i64,
    weight_milli: i64,
}

#[derive(Clone, Debug)]
struct IndexedSummaryNode {
    node_id: String,
    parent_node_id: Option<String>,
    node_type: String,
    title: Option<String>,
    node_json: String,
    semantic_text: String,
    file_paths: Vec<String>,
    commands: Vec<IndexedCommand>,
    errors: Vec<String>,
    semantic_terms: Vec<IndexedSemanticTerm>,
}

#[derive(Debug, Serialize)]
struct JinaEmbeddingsRequest<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Debug, Deserialize)]
struct JinaEmbeddingsResponse {
    data: Vec<JinaEmbeddingData>,
}

#[derive(Debug, Deserialize)]
struct JinaEmbeddingData {
    index: usize,
    embedding: Vec<f32>,
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
        let embedding_inputs: Vec<String> = indexed_nodes
            .iter()
            .map(|node| node.semantic_text.clone())
            .collect();
        let dense_embeddings = fetch_jina_embeddings(embedding_inputs.as_slice()).await?;
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

        for (node_idx, node) in indexed_nodes.iter().enumerate() {
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

            if let Some(embeddings) = dense_embeddings.as_ref()
                && let Some(embedding) = embeddings.get(node_idx)
            {
                let norm = embedding_l2_norm(embedding.as_slice());
                if norm > f64::EPSILON {
                    sqlx::query(
                        r#"
INSERT INTO summary_node_embeddings (
    summary_id,
    node_id,
    embedding_model,
    dimensions,
    norm,
    embedding
) VALUES (?, ?, ?, ?, ?, ?)
                        "#,
                    )
                    .bind(summary_id.as_str())
                    .bind(node.node_id.as_str())
                    .bind(SUMMARY_SEMANTIC_EMBEDDING_MODEL)
                    .bind(i64::try_from(embedding.len()).unwrap_or(i64::MAX))
                    .bind(norm)
                    .bind(embedding_to_blob(embedding.as_slice()))
                    .execute(&mut *tx)
                    .await?;
                }
            }

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
            for term in &node.semantic_terms {
                sqlx::query(
                    r#"
INSERT INTO summary_node_semantic_terms (
    summary_id,
    node_id,
    term_hash,
    weight
) VALUES (?, ?, ?, ?)
                    "#,
                )
                .bind(summary_id.as_str())
                .bind(node.node_id.as_str())
                .bind(term.term_hash)
                .bind((term.weight_milli as f64) / 1000.0)
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

    pub async fn search_summary_nodes_by_semantic_text(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummarySemanticNodeMatch>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let query_input = vec![query.to_string()];
        if let Some(query_embeddings) = fetch_jina_embeddings(query_input.as_slice()).await?
            && let Some(query_embedding) = query_embeddings.first()
        {
            let dense_matches = self
                .search_summary_nodes_by_dense_embedding(query, query_embedding.as_slice(), limit)
                .await?;
            if !dense_matches.is_empty() {
                return Ok(dense_matches);
            }
        }
        self.search_summary_nodes_by_sparse_semantic_text(query, limit)
            .await
    }

    pub async fn search_summary_nodes_by_hybrid_text(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummarySemanticNodeMatch>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let candidate_limit = limit.saturating_mul(4).max(limit);
        let semantic_matches = self
            .search_summary_nodes_by_semantic_text(query, candidate_limit)
            .await?;
        let lexical_matches = self
            .search_summary_nodes_by_lexical_text(query, candidate_limit)
            .await?;

        let mut ranked = BTreeMap::<(String, String), SessionSummarySemanticNodeMatch>::new();
        for item in semantic_matches {
            let key = (
                item.node_match.summary_id.clone(),
                item.node_match.node_id.clone(),
            );
            ranked.insert(key, item);
        }
        for item in lexical_matches {
            let key = (
                item.node_match.summary_id.clone(),
                item.node_match.node_id.clone(),
            );
            if let Some(existing) = ranked.get_mut(&key) {
                if item.lexical_score > existing.lexical_score {
                    existing.lexical_score = item.lexical_score;
                }
                continue;
            }
            ranked.insert(key, item);
        }

        let mut merged: Vec<SessionSummarySemanticNodeMatch> = ranked.into_values().collect();
        let max_semantic = merged
            .iter()
            .map(|item| item.semantic_score)
            .fold(0.0, f64::max);
        let max_lexical = merged
            .iter()
            .map(|item| item.lexical_score)
            .fold(0.0, f64::max);
        for item in &mut merged {
            let semantic_component = if max_semantic > 0.0 {
                item.semantic_score / max_semantic
            } else {
                0.0
            };
            let lexical_component = if max_lexical > 0.0 {
                item.lexical_score / max_lexical
            } else {
                0.0
            };
            item.hybrid_score = (HYBRID_SEMANTIC_WEIGHT * semantic_component)
                + (HYBRID_LEXICAL_WEIGHT * lexical_component);
        }
        merged.sort_by(|left, right| {
            right
                .hybrid_score
                .total_cmp(&left.hybrid_score)
                .then_with(|| right.semantic_score.total_cmp(&left.semantic_score))
                .then_with(|| right.lexical_score.total_cmp(&left.lexical_score))
                .then_with(|| left.node_match.summary_id.cmp(&right.node_match.summary_id))
                .then_with(|| left.node_match.node_id.cmp(&right.node_match.node_id))
        });
        if merged.len() > limit {
            merged.truncate(limit);
        }
        Ok(merged)
    }

    async fn search_summary_nodes_by_sparse_semantic_text(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummarySemanticNodeMatch>> {
        let query_terms = semantic_terms_from_text(query);
        if query_terms.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let query_term_count = query_terms.len();
        let candidate_limit = limit.saturating_mul(4).max(limit);
        let candidate_limit = i64::try_from(candidate_limit).unwrap_or(i64::MAX);
        let mut sql = QueryBuilder::<Sqlite>::new(
            r#"
WITH query_terms(term_hash, query_weight) AS (
"#,
        );
        sql.push_values(query_terms.iter(), |mut row, term| {
            row.push_bind(term.term_hash)
                .push_bind((term.weight_milli as f64) / 1000.0);
        });
        sql.push(
            r#"
),
scored_nodes AS (
    SELECT
        terms.summary_id,
        terms.node_id,
        SUM(terms.weight * query_terms.query_weight) AS raw_semantic_score,
        COUNT(DISTINCT query_terms.term_hash) AS matched_term_count
    FROM summary_node_semantic_terms AS terms
    INNER JOIN query_terms
        ON query_terms.term_hash = terms.term_hash
    GROUP BY terms.summary_id, terms.node_id
    ORDER BY raw_semantic_score DESC
    LIMIT
"#,
        )
        .push_bind(candidate_limit)
        .push(
            r#"
)
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
    NULL AS matched_error_text,
    scored_nodes.raw_semantic_score AS semantic_score,
    0.0 AS lexical_score,
    scored_nodes.raw_semantic_score AS hybrid_score,
    scored_nodes.raw_semantic_score,
    scored_nodes.matched_term_count
FROM scored_nodes
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = scored_nodes.summary_id
    AND nodes.node_id = scored_nodes.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = scored_nodes.summary_id
ORDER BY scored_nodes.raw_semantic_score DESC, artifacts.updated_at DESC, nodes.node_id ASC
"#,
        );
        let rows = sql.build().fetch_all(self.pool.as_ref()).await?;
        let mut matches = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut node_match = summary_semantic_node_match_from_row(row)?;
            let raw_semantic_score: f64 = row.try_get("raw_semantic_score")?;
            let matched_term_count: i64 = row.try_get("matched_term_count")?;
            node_match.semantic_score = coverage_adjusted_score(
                raw_semantic_score,
                matched_term_count,
                query_term_count,
                SEMANTIC_COVERAGE_BASE_WEIGHT,
            );
            node_match.hybrid_score = node_match.semantic_score;
            matches.push(node_match);
        }
        matches.sort_by(|left, right| {
            right
                .semantic_score
                .total_cmp(&left.semantic_score)
                .then_with(|| left.node_match.summary_id.cmp(&right.node_match.summary_id))
                .then_with(|| left.node_match.node_id.cmp(&right.node_match.node_id))
        });
        if matches.len() > limit {
            matches.truncate(limit);
        }
        Ok(matches)
    }

    async fn search_summary_nodes_by_lexical_text(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummarySemanticNodeMatch>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let mut query_terms = BTreeSet::new();
        for token in semantic_tokens(query) {
            query_terms.insert(token);
        }
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let query_terms: Vec<String> = query_terms.into_iter().collect();
        let query_term_count = query_terms.len();
        let candidate_limit = limit.saturating_mul(4).max(limit);
        let candidate_limit = i64::try_from(candidate_limit).unwrap_or(i64::MAX);
        let mut sql = QueryBuilder::<Sqlite>::new(
            r#"
WITH query_terms(term) AS (
"#,
        );
        sql.push_values(query_terms.iter(), |mut row, term| {
            row.push_bind(term.as_str());
        });
        sql.push(
            r#"
),
lexical_hits AS (
    SELECT
        file_paths.summary_id,
        file_paths.node_id,
        0.8 AS lexical_weight,
        query_terms.term AS matched_term
    FROM summary_node_file_paths AS file_paths
    INNER JOIN query_terms
        ON INSTR(LOWER(file_paths.file_path), query_terms.term) > 0
    UNION ALL
    SELECT
        commands.summary_id,
        commands.node_id,
        CASE
            WHEN commands.exit_code IS NOT NULL AND commands.exit_code != 0 THEN
    "#,
        )
        .push_bind(LEXICAL_COMMAND_FAILURE_WEIGHT)
        .push(
            r#"
            ELSE 1.0
        END AS lexical_weight,
        query_terms.term AS matched_term
    FROM summary_node_commands AS commands
    INNER JOIN query_terms
        ON INSTR(LOWER(commands.command), query_terms.term) > 0
    UNION ALL
    SELECT
        errors.summary_id,
        errors.node_id,
        1.2 AS lexical_weight,
        query_terms.term AS matched_term
    FROM summary_node_errors AS errors
    INNER JOIN query_terms
        ON INSTR(LOWER(errors.error_text), query_terms.term) > 0
    UNION ALL
    SELECT
        nodes.summary_id,
        nodes.node_id,
        0.6 AS lexical_weight,
        query_terms.term AS matched_term
    FROM summary_nodes AS nodes
    INNER JOIN query_terms
        ON INSTR(LOWER(COALESCE(nodes.title, '')), query_terms.term) > 0
),
scored_nodes AS (
    SELECT
        summary_id,
        node_id,
        SUM(lexical_weight) AS raw_lexical_score,
        COUNT(DISTINCT matched_term) AS matched_term_count
    FROM lexical_hits
    GROUP BY summary_id, node_id
    ORDER BY raw_lexical_score DESC
    LIMIT
"#,
        )
        .push_bind(candidate_limit)
        .push(
            r#"
)
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
    NULL AS matched_error_text,
    0.0 AS semantic_score,
    scored_nodes.raw_lexical_score AS lexical_score,
    scored_nodes.raw_lexical_score AS hybrid_score,
    scored_nodes.raw_lexical_score,
    scored_nodes.matched_term_count
FROM scored_nodes
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = scored_nodes.summary_id
    AND nodes.node_id = scored_nodes.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = scored_nodes.summary_id
ORDER BY scored_nodes.raw_lexical_score DESC, artifacts.updated_at DESC, nodes.node_id ASC
"#,
        );
        let rows = sql.build().fetch_all(self.pool.as_ref()).await?;
        let mut matches = Vec::with_capacity(rows.len());
        for row in &rows {
            let mut node_match = summary_semantic_node_match_from_row(row)?;
            let raw_lexical_score: f64 = row.try_get("raw_lexical_score")?;
            let matched_term_count: i64 = row.try_get("matched_term_count")?;
            node_match.lexical_score = coverage_adjusted_score(
                raw_lexical_score,
                matched_term_count,
                query_term_count,
                LEXICAL_COVERAGE_BASE_WEIGHT,
            );
            node_match.hybrid_score = node_match.lexical_score;
            matches.push(node_match);
        }
        matches.sort_by(|left, right| {
            right
                .lexical_score
                .total_cmp(&left.lexical_score)
                .then_with(|| left.node_match.summary_id.cmp(&right.node_match.summary_id))
                .then_with(|| left.node_match.node_id.cmp(&right.node_match.node_id))
        });
        if matches.len() > limit {
            matches.truncate(limit);
        }
        Ok(matches)
    }

    async fn search_summary_nodes_by_dense_embedding(
        &self,
        query: &str,
        query_embedding: &[f32],
        limit: usize,
    ) -> anyhow::Result<Vec<SessionSummarySemanticNodeMatch>> {
        if query_embedding.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let query_norm = embedding_l2_norm(query_embedding);
        if query_norm <= f64::EPSILON {
            return Ok(Vec::new());
        }

        let candidate_limit = limit.saturating_mul(64).max(limit);
        let candidates = self
            .dense_embedding_candidate_keys(query, candidate_limit)
            .await?;
        if candidates.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = QueryBuilder::<Sqlite>::new(
            r#"
WITH candidate_nodes(summary_id, node_id) AS (
"#,
        );
        sql.push_values(candidates.iter(), |mut row, (summary_id, node_id)| {
            row.push_bind(summary_id.as_str())
                .push_bind(node_id.as_str());
        });
        sql.push(
            r#"
)
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
    NULL AS matched_error_text,
    embeddings.embedding_model,
    embeddings.dimensions,
    embeddings.norm,
    embeddings.embedding
FROM summary_node_embeddings AS embeddings
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = embeddings.summary_id
    AND nodes.node_id = embeddings.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = embeddings.summary_id
INNER JOIN candidate_nodes
    ON candidate_nodes.summary_id = embeddings.summary_id
    AND candidate_nodes.node_id = embeddings.node_id
WHERE embeddings.embedding_model = ?
            "#,
        );
        let rows = sql
            .push_bind(SUMMARY_SEMANTIC_EMBEDDING_MODEL)
            .build()
            .fetch_all(self.pool.as_ref())
            .await?;

        let mut scored = Vec::new();
        for row in &rows {
            let dimensions: i64 = row.try_get("dimensions")?;
            let dimensions = usize::try_from(dimensions).unwrap_or(0);
            if dimensions == 0 || dimensions != query_embedding.len() {
                continue;
            }

            let embedding_blob: Vec<u8> = row.try_get("embedding")?;
            let Some(node_embedding) = embedding_from_blob(embedding_blob.as_slice(), dimensions)
            else {
                continue;
            };

            let node_norm: f64 = row.try_get("norm")?;
            if node_norm <= f64::EPSILON {
                continue;
            }

            let semantic_score = embedding_dot_product(query_embedding, node_embedding.as_slice())
                / (query_norm * node_norm);
            if semantic_score <= 0.0 {
                continue;
            }

            scored.push(SessionSummarySemanticNodeMatch {
                node_match: summary_node_match_from_row(row)?,
                embedding_model: row.try_get("embedding_model")?,
                semantic_score,
                lexical_score: 0.0,
                hybrid_score: semantic_score,
            });
        }

        scored.sort_by(|left, right| {
            right
                .semantic_score
                .total_cmp(&left.semantic_score)
                .then_with(|| left.node_match.summary_id.cmp(&right.node_match.summary_id))
                .then_with(|| left.node_match.node_id.cmp(&right.node_match.node_id))
        });
        if scored.len() > limit {
            scored.truncate(limit);
        }
        Ok(scored)
    }

    async fn dense_embedding_candidate_keys(
        &self,
        query: &str,
        limit: usize,
    ) -> anyhow::Result<Vec<(String, String)>> {
        if query.trim().is_empty() || limit == 0 {
            return Ok(Vec::new());
        }

        let query_terms = semantic_terms_from_text(query);
        if query_terms.is_empty() {
            return Ok(Vec::new());
        }

        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let mut sql = QueryBuilder::<Sqlite>::new(
            r#"
WITH query_terms(term_hash, query_weight) AS (
"#,
        );
        sql.push_values(query_terms.iter(), |mut row, term| {
            row.push_bind(term.term_hash)
                .push_bind((term.weight_milli as f64) / 1000.0);
        });
        sql.push(
            r#"
)
SELECT
    terms.summary_id,
    terms.node_id
FROM summary_node_embeddings AS embeddings
INNER JOIN summary_node_semantic_terms AS terms
    ON terms.summary_id = embeddings.summary_id
    AND terms.node_id = embeddings.node_id
INNER JOIN query_terms
    ON query_terms.term_hash = terms.term_hash
WHERE embeddings.embedding_model = ?
GROUP BY terms.summary_id, terms.node_id
ORDER BY SUM(terms.weight * query_terms.query_weight) DESC
LIMIT
"#,
        )
        .push_bind(SUMMARY_SEMANTIC_EMBEDDING_MODEL)
        .push_bind(limit);
        let rows = sql.build().fetch_all(self.pool.as_ref()).await?;
        rows.iter()
            .map(|row| Ok((row.try_get("summary_id")?, row.try_get("node_id")?)))
            .collect()
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

fn summary_semantic_node_match_from_row(
    row: &SqliteRow,
) -> anyhow::Result<SessionSummarySemanticNodeMatch> {
    Ok(SessionSummarySemanticNodeMatch {
        node_match: summary_node_match_from_row(row)?,
        embedding_model: SUMMARY_SEMANTIC_EMBEDDING_MODEL.to_string(),
        semantic_score: row.try_get("semantic_score")?,
        lexical_score: row.try_get("lexical_score")?,
        hybrid_score: row.try_get("hybrid_score")?,
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
                let node_type = string_value(object, &["node_type", "nodeType", "type"])
                    .unwrap_or_else(|| "unknown".to_string());
                let node_title = string_value(object, &["title", "label", "summary"]);
                let semantic_text = semantic_text_for_node(
                    value,
                    node_type.as_str(),
                    node_title.as_deref(),
                    &file_paths,
                    &commands,
                    &errors,
                );
                let semantic_terms = semantic_terms_from_text(semantic_text.as_str());
                let node_json = serde_json::to_string(value)?;
                indexed_nodes.push(IndexedSummaryNode {
                    node_id: node_id.clone(),
                    parent_node_id: string_value(object, &["parent_id", "parentId"])
                        .or_else(|| inherited_parent_id.map(ToOwned::to_owned)),
                    node_type,
                    title: node_title,
                    node_json,
                    semantic_text,
                    file_paths: file_paths.into_iter().collect(),
                    commands: commands.into_iter().collect(),
                    errors: errors.into_iter().collect(),
                    semantic_terms,
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
            if key_hint.is_some_and(is_command_key)
                && let Some(indexed_command) = indexed_command_from_object(object)
            {
                commands.insert(indexed_command);
                return;
            }
            if object_has_command_and_exit_code(object)
                && let Some(indexed_command) = indexed_command_from_object(object)
            {
                commands.insert(indexed_command);
                return;
            }
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
        "file"
            | "files"
            | "file_path"
            | "file_paths"
            | "filepath"
            | "filepaths"
            | "path"
            | "paths"
            | "primary_file_path"
    )
}

fn is_command_key(key: &str) -> bool {
    matches!(key, "command" | "commands" | "cmd" | "primary_command")
}

fn is_error_key(key: &str) -> bool {
    matches!(
        key,
        "error" | "errors" | "error_text" | "errortext" | "last_error" | "stderr" | "primary_error"
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

fn indexed_command_from_object(object: &Map<String, Value>) -> Option<IndexedCommand> {
    let command = string_value(
        object,
        &[
            "command",
            "cmd",
            "primary_command",
            "raw_command",
            "rawCommand",
        ],
    )?;
    if command.trim().is_empty() {
        return None;
    }
    let exit_code = integer_value(
        object,
        &[
            "exit_code",
            "exitCode",
            "return_code",
            "returnCode",
            "status_code",
            "statusCode",
            "code",
        ],
    );
    Some(IndexedCommand { command, exit_code })
}

fn object_has_command_and_exit_code(object: &Map<String, Value>) -> bool {
    let has_command = ["command", "cmd", "primary_command"]
        .iter()
        .any(|key| object.contains_key(*key));
    let has_exit_code = [
        "exit_code",
        "exitCode",
        "return_code",
        "returnCode",
        "status_code",
        "statusCode",
        "code",
    ]
    .iter()
    .any(|key| object.contains_key(*key));
    has_command && has_exit_code
}

fn string_value(object: &Map<String, Value>, keys: &[&str]) -> Option<String> {
    for key in keys {
        if let Some(Value::String(value)) = object.get(*key) {
            return Some(value.clone());
        }
    }
    None
}

fn integer_value(object: &Map<String, Value>, keys: &[&str]) -> Option<i64> {
    for key in keys {
        if let Some(value) = object.get(*key) {
            match value {
                Value::Number(number) => {
                    if let Some(value) = number.as_i64() {
                        return Some(value);
                    }
                }
                Value::String(value) => {
                    if let Ok(parsed) = value.parse::<i64>() {
                        return Some(parsed);
                    }
                }
                Value::Null | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {}
            }
        }
    }
    None
}

fn semantic_text_for_node(
    node: &Value,
    node_type: &str,
    title: Option<&str>,
    file_paths: &BTreeSet<String>,
    commands: &BTreeSet<IndexedCommand>,
    errors: &BTreeSet<String>,
) -> String {
    let mut text_fragments = Vec::new();
    if !node_type.is_empty() {
        text_fragments.push(node_type.to_string());
    }
    if let Some(title) = title
        && !title.trim().is_empty()
    {
        text_fragments.push(title.to_string());
    }
    collect_semantic_text_fragments(node, None, &mut text_fragments);
    for file_path in file_paths {
        text_fragments.push(file_path.clone());
    }
    for command in commands {
        text_fragments.push(command.command.clone());
        if let Some(exit_code) = command.exit_code {
            if exit_code == 0 {
                text_fragments.push("command succeeded".to_string());
            } else {
                text_fragments.push("command failed".to_string());
                text_fragments.push(format!("exit code {exit_code}"));
            }
        }
    }
    for error in errors {
        text_fragments.push(error.clone());
    }
    let joined = text_fragments.join(" ");
    if joined.trim().is_empty() {
        return "summary node".to_string();
    }
    joined
}

fn collect_semantic_text_fragments(
    value: &Value,
    key_hint: Option<&str>,
    text_fragments: &mut Vec<String>,
) {
    match value {
        Value::Object(object) => {
            for (key, child) in object {
                let key_lower = key.to_ascii_lowercase();
                collect_semantic_text_fragments(child, Some(key_lower.as_str()), text_fragments);
            }
        }
        Value::Array(values) => {
            for child in values {
                collect_semantic_text_fragments(child, key_hint, text_fragments);
            }
        }
        Value::String(text) => {
            let Some(key_hint) = key_hint else {
                return;
            };
            if is_semantic_text_key(key_hint) && !text.trim().is_empty() {
                text_fragments.push(text.clone());
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn is_semantic_text_key(key: &str) -> bool {
    matches!(
        key,
        "title"
            | "label"
            | "summary"
            | "description"
            | "message"
            | "status"
            | "reason"
            | "outcome"
            | "result"
            | "decision"
            | "plan"
            | "rationale"
            | "next_step"
            | "next_steps"
            | "action"
            | "actions"
            | "node_type"
            | "nodetype"
            | "type"
            | "command"
            | "commands"
            | "cmd"
            | "primary_command"
            | "error"
            | "errors"
            | "error_text"
            | "last_error"
            | "primary_error"
            | "file"
            | "files"
            | "file_path"
            | "file_paths"
            | "primary_file_path"
            | "path"
            | "paths"
            | "agent_message"
            | "signal"
    )
}

fn semantic_terms_from_text(text: &str) -> Vec<IndexedSemanticTerm> {
    let tokens = semantic_tokens(text);
    if tokens.is_empty() {
        return Vec::new();
    }

    let mut weights_by_hash = BTreeMap::<i64, f64>::new();
    for token in &tokens {
        add_semantic_weight(
            &mut weights_by_hash,
            semantic_term_hash(format!("w:{token}").as_str()),
            1.0,
        );
        let token_bytes = token.as_bytes();
        if token_bytes.len() >= 5 {
            for window in token_bytes.windows(3) {
                let trigram = String::from_utf8_lossy(window);
                add_semantic_weight(
                    &mut weights_by_hash,
                    semantic_term_hash(format!("g:{trigram}").as_str()),
                    0.2,
                );
            }
        }
    }
    for token_pair in tokens.windows(2) {
        let bigram = format!("{} {}", token_pair[0], token_pair[1]);
        add_semantic_weight(
            &mut weights_by_hash,
            semantic_term_hash(format!("b:{bigram}").as_str()),
            0.75,
        );
    }

    let mut weighted_features: Vec<(i64, f64)> = weights_by_hash.into_iter().collect();
    weighted_features.sort_by(|(left_hash, left_weight), (right_hash, right_weight)| {
        right_weight
            .total_cmp(left_weight)
            .then_with(|| left_hash.cmp(right_hash))
    });
    const MAX_FEATURES: usize = 96;
    if weighted_features.len() > MAX_FEATURES {
        weighted_features.truncate(MAX_FEATURES);
    }

    let norm = weighted_features
        .iter()
        .map(|(_, weight)| weight * weight)
        .sum::<f64>()
        .sqrt();
    if norm <= f64::EPSILON {
        return Vec::new();
    }

    let mut terms = Vec::with_capacity(weighted_features.len());
    for (term_hash, raw_weight) in weighted_features {
        let normalized_weight = raw_weight / norm;
        let weight_milli = (normalized_weight * 1000.0).round() as i64;
        if weight_milli == 0 {
            continue;
        }
        terms.push(IndexedSemanticTerm {
            term_hash,
            weight_milli,
        });
    }
    terms.sort_by(|left, right| left.term_hash.cmp(&right.term_hash));
    terms
}

fn coverage_adjusted_score(
    raw_score: f64,
    matched_term_count: i64,
    query_term_count: usize,
    coverage_base_weight: f64,
) -> f64 {
    if raw_score <= 0.0 || query_term_count == 0 {
        return raw_score.max(0.0);
    }
    let matched_term_count = usize::try_from(matched_term_count)
        .unwrap_or(0)
        .min(query_term_count);
    let coverage_ratio = (matched_term_count as f64) / (query_term_count as f64);
    let coverage_base_weight = coverage_base_weight.clamp(0.0, 1.0);
    raw_score * (coverage_base_weight + ((1.0 - coverage_base_weight) * coverage_ratio))
}

fn add_semantic_weight(weights_by_hash: &mut BTreeMap<i64, f64>, term_hash: i64, delta: f64) {
    let entry = weights_by_hash.entry(term_hash).or_insert(0.0);
    *entry += delta;
}

fn semantic_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            current.push(ch.to_ascii_lowercase());
            continue;
        }
        if let Some(token) = normalize_semantic_token(current.as_str()) {
            tokens.push(token);
        }
        current.clear();
    }
    if let Some(token) = normalize_semantic_token(current.as_str()) {
        tokens.push(token);
    }
    tokens
}

fn normalize_semantic_token(token: &str) -> Option<String> {
    if token.len() < 2 {
        return None;
    }
    if token.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some(token.to_string())
}

fn semantic_term_hash(text: &str) -> i64 {
    const FNV_OFFSET_BASIS: u32 = 2_166_136_261;
    const FNV_PRIME: u32 = 16_777_619;

    let mut hash = FNV_OFFSET_BASIS;
    for byte in text.as_bytes() {
        hash ^= u32::from(*byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    i64::from(hash)
}

fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    let mut blob = Vec::with_capacity(std::mem::size_of_val(embedding));
    for value in embedding {
        blob.extend_from_slice(&value.to_le_bytes());
    }
    blob
}

fn embedding_from_blob(blob: &[u8], dimensions: usize) -> Option<Vec<f32>> {
    let expected_bytes = dimensions.checked_mul(std::mem::size_of::<f32>())?;
    if blob.len() != expected_bytes {
        return None;
    }

    let mut embedding = Vec::with_capacity(dimensions);
    let mut chunks = blob.chunks_exact(std::mem::size_of::<f32>());
    for chunk in &mut chunks {
        let bytes = [chunk[0], chunk[1], chunk[2], chunk[3]];
        embedding.push(f32::from_le_bytes(bytes));
    }
    if !chunks.remainder().is_empty() {
        return None;
    }
    Some(embedding)
}

fn embedding_l2_norm(embedding: &[f32]) -> f64 {
    embedding
        .iter()
        .map(|value| f64::from(*value) * f64::from(*value))
        .sum::<f64>()
        .sqrt()
}

fn embedding_dot_product(left: &[f32], right: &[f32]) -> f64 {
    if left.len() != right.len() {
        return 0.0;
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| f64::from(*left) * f64::from(*right))
        .sum::<f64>()
}

async fn fetch_jina_embeddings(inputs: &[String]) -> anyhow::Result<Option<Vec<Vec<f32>>>> {
    if inputs.is_empty() {
        return Ok(Some(Vec::new()));
    }

    let Ok(api_key) = std::env::var(SUMMARY_SEMANTIC_JINA_API_KEY_ENV) else {
        return Ok(None);
    };
    if api_key.trim().is_empty() {
        return Ok(None);
    }

    let api_url = std::env::var(SUMMARY_SEMANTIC_JINA_API_URL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| SUMMARY_SEMANTIC_JINA_DEFAULT_API_URL.to_string());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let response = match client
        .post(api_url)
        .bearer_auth(api_key)
        .json(&JinaEmbeddingsRequest {
            model: SUMMARY_SEMANTIC_EMBEDDING_MODEL,
            input: inputs,
        })
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return Ok(None),
    };
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        return Ok(None);
    }
    if !response.status().is_success() {
        return Ok(None);
    }

    let payload = match response.json::<JinaEmbeddingsResponse>().await {
        Ok(payload) => payload,
        Err(_) => return Ok(None),
    };

    let mut ordered_embeddings = vec![None; inputs.len()];
    for item in payload.data {
        if item.index < ordered_embeddings.len() {
            ordered_embeddings[item.index] = Some(item.embedding);
        }
    }
    let mut embeddings = Vec::with_capacity(ordered_embeddings.len());
    for embedding in ordered_embeddings {
        let Some(embedding) = embedding else {
            return Ok(None);
        };
        if embedding.is_empty() {
            return Ok(None);
        }
        embeddings.push(embedding);
    }
    Ok(Some(embeddings))
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

    fn sample_summary_for_semantic_search() -> Value {
        json!({
            "schema_version": "v1",
            "nodes": [
                {
                    "node_id": "test-node",
                    "node_type": "execution",
                    "title": "Run state tests",
                    "summary": "Executed cargo test for codex-state and validated summary indexing.",
                    "evidence": {
                        "commands": [{"command": "cargo test -p codex-state"}],
                        "errors": []
                    }
                },
                {
                    "node_id": "deploy-node",
                    "node_type": "execution",
                    "title": "Publish release",
                    "summary": "Pushed release artifacts to production.",
                    "evidence": {
                        "commands": [{"command": "deploy --env prod"}],
                        "errors": []
                    }
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
    async fn summary_search_indexes_primary_brief_fields() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");

        let summary = json!({
            "schema_version": "agentcanvas.turn.v2",
            "summary_kind": "agentcanvas_turn_summary",
            "thread_id": "thread-brief",
            "session_id": "session-brief",
            "nodes": [
                {
                    "node_id": "turn:brief-1",
                    "node_type": "turn",
                    "title": "Turn brief-1",
                    "brief": {
                        "signal": "error",
                        "agent_message": "build failed while running tests",
                        "primary_command": "cargo test -p codex-core",
                        "primary_file_path": "/repo/codex-rs/core/src/rollout/recorder.rs",
                        "primary_error": "assertion failed"
                    },
                    "evidence": {
                        "commands": [],
                        "file_paths": [],
                        "errors": []
                    }
                }
            ]
        });
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-brief".to_string(),
                session_id: "session-brief".to_string(),
                schema_version: "agentcanvas.turn.v2".to_string(),
                root_node_id: None,
                summary,
            })
            .await
            .expect("upsert summary");

        let file_matches = runtime
            .search_summary_nodes_by_file_path("recorder.rs", 10)
            .await
            .expect("search by file path");
        assert_eq!(file_matches.len(), 1);
        assert_eq!(file_matches[0].node_id, "turn:brief-1");

        let command_matches = runtime
            .search_summary_nodes_by_command_substring("cargo test -p codex-core", 10)
            .await
            .expect("search by command");
        assert_eq!(command_matches.len(), 1);
        assert_eq!(command_matches[0].node_id, "turn:brief-1");

        let error_matches = runtime
            .search_summary_nodes_by_error_text("assertion failed", 10)
            .await
            .expect("search by error");
        assert_eq!(error_matches.len(), 1);
        assert_eq!(error_matches[0].node_id, "turn:brief-1");

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

    #[tokio::test]
    async fn summary_semantic_search_returns_relevant_nodes() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-semantic".to_string(),
                session_id: "session-semantic".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: sample_summary_for_semantic_search(),
            })
            .await
            .expect("upsert summary");

        let test_matches = runtime
            .search_summary_nodes_by_semantic_text("run codex state tests", 10)
            .await
            .expect("semantic search should succeed");
        assert!(!test_matches.is_empty());
        assert_eq!(test_matches[0].node_match.node_id, "test-node");
        assert_eq!(
            test_matches[0].embedding_model,
            "jina-embeddings-v5-text-nano"
        );
        assert!(test_matches[0].semantic_score > 0.0);
        assert_eq!(test_matches[0].lexical_score, 0.0);
        assert_eq!(test_matches[0].hybrid_score, test_matches[0].semantic_score);

        let deploy_matches = runtime
            .search_summary_nodes_by_semantic_text("deploy production release", 10)
            .await
            .expect("semantic search should succeed");
        assert!(!deploy_matches.is_empty());
        assert_eq!(deploy_matches[0].node_match.node_id, "deploy-node");
        assert!(deploy_matches[0].semantic_score > 0.0);
        assert_eq!(deploy_matches[0].lexical_score, 0.0);
        assert_eq!(
            deploy_matches[0].hybrid_score,
            deploy_matches[0].semantic_score
        );

        let no_matches = runtime
            .search_summary_nodes_by_semantic_text("zxqwy impossible token", 10)
            .await
            .expect("semantic search should succeed");
        assert!(no_matches.is_empty());

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn summary_hybrid_search_combines_semantic_and_lexical_scores() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-hybrid".to_string(),
                session_id: "session-hybrid".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: sample_summary_for_semantic_search(),
            })
            .await
            .expect("upsert summary");

        let matches = runtime
            .search_summary_nodes_by_hybrid_text("cargo test codex state", 10)
            .await
            .expect("hybrid search should succeed");
        assert!(!matches.is_empty());
        assert_eq!(matches[0].node_match.node_id, "test-node");
        assert!(matches[0].hybrid_score > 0.0);
        assert!(matches.iter().any(|item| item.lexical_score > 0.0));

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn summary_index_persists_structured_command_exit_code() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-exit-code".to_string(),
                session_id: "session-exit-code".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: json!({
                    "schema_version": "v1",
                    "nodes": [
                        {
                            "node_id": "cmd-node",
                            "node_type": "execution",
                            "title": "Run failing tests",
                            "evidence": {
                                "commands": [
                                    {
                                        "command": "cargo test -p codex-state",
                                        "exit_code": 101
                                    }
                                ]
                            }
                        }
                    ]
                }),
            })
            .await
            .expect("upsert summary");

        let row = sqlx::query(
            "SELECT exit_code FROM summary_node_commands WHERE summary_id = ? AND node_id = ?",
        )
        .bind("thread-exit-code:session-exit-code")
        .bind("cmd-node")
        .fetch_one(runtime.pool.as_ref())
        .await
        .expect("query indexed command");
        let indexed_exit_code: Option<i64> = row.try_get("exit_code").expect("read exit_code");
        assert_eq!(indexed_exit_code, Some(101));

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }

    #[tokio::test]
    async fn summary_semantic_search_prefers_query_term_coverage() {
        let codex_home = unique_temp_dir();
        let runtime = StateRuntime::init(codex_home.clone(), "test-provider".to_string(), None)
            .await
            .expect("initialize runtime");
        runtime
            .upsert_session_summary(&SessionSummaryPersistParams {
                summary_id: None,
                thread_id: "thread-coverage".to_string(),
                session_id: "session-coverage".to_string(),
                schema_version: "v1".to_string(),
                root_node_id: None,
                summary: json!({
                    "schema_version": "v1",
                    "nodes": [
                        {
                            "node_id": "cargo-spam",
                            "node_type": "execution",
                            "title": "cargo cargo cargo cargo",
                            "summary": "cargo cargo cargo cargo",
                            "evidence": {
                                "commands": [{"command": "cargo"}]
                            }
                        },
                        {
                            "node_id": "focused-node",
                            "node_type": "execution",
                            "title": "Run codex state tests",
                            "summary": "Run cargo test for codex state indexing",
                            "evidence": {
                                "commands": [{"command": "cargo test -p codex-state"}]
                            }
                        }
                    ]
                }),
            })
            .await
            .expect("upsert summary");

        let matches = runtime
            .search_summary_nodes_by_semantic_text("cargo test codex state", 5)
            .await
            .expect("semantic search should succeed");
        assert!(!matches.is_empty());
        assert_eq!(matches[0].node_match.node_id, "focused-node");

        let _ = tokio::fs::remove_dir_all(codex_home).await;
    }
}
