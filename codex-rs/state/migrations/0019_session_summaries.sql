CREATE TABLE summary_artifacts (
    summary_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    schema_version TEXT NOT NULL,
    summary_path TEXT NOT NULL,
    root_node_id TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE UNIQUE INDEX idx_summary_artifacts_thread_session
ON summary_artifacts(thread_id, session_id);

CREATE INDEX idx_summary_artifacts_thread_updated_at
ON summary_artifacts(thread_id, updated_at DESC);

CREATE INDEX idx_summary_artifacts_session_updated_at
ON summary_artifacts(session_id, updated_at DESC);

CREATE TABLE summary_nodes (
    summary_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    parent_node_id TEXT,
    node_type TEXT NOT NULL,
    title TEXT,
    node_json TEXT NOT NULL,
    PRIMARY KEY (summary_id, node_id),
    FOREIGN KEY(summary_id) REFERENCES summary_artifacts(summary_id) ON DELETE CASCADE
);

CREATE INDEX idx_summary_nodes_type
ON summary_nodes(node_type);

CREATE TABLE summary_node_file_paths (
    summary_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    file_path TEXT NOT NULL,
    PRIMARY KEY (summary_id, node_id, file_path),
    FOREIGN KEY(summary_id, node_id) REFERENCES summary_nodes(summary_id, node_id) ON DELETE CASCADE
);

CREATE INDEX idx_summary_node_file_paths_file_path
ON summary_node_file_paths(file_path);

CREATE TABLE summary_node_commands (
    summary_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    command TEXT NOT NULL,
    exit_code INTEGER,
    FOREIGN KEY(summary_id, node_id) REFERENCES summary_nodes(summary_id, node_id) ON DELETE CASCADE
);

CREATE INDEX idx_summary_node_commands_summary_node
ON summary_node_commands(summary_id, node_id);

CREATE INDEX idx_summary_node_commands_command
ON summary_node_commands(command);

CREATE TABLE summary_node_errors (
    summary_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    error_text TEXT NOT NULL,
    PRIMARY KEY (summary_id, node_id, error_text),
    FOREIGN KEY(summary_id, node_id) REFERENCES summary_nodes(summary_id, node_id) ON DELETE CASCADE
);

CREATE INDEX idx_summary_node_errors_error_text
ON summary_node_errors(error_text);
