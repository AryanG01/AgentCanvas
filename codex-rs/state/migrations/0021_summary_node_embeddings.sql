CREATE TABLE summary_node_embeddings (
    summary_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    embedding_model TEXT NOT NULL,
    dimensions INTEGER NOT NULL,
    norm REAL NOT NULL,
    embedding BLOB NOT NULL,
    PRIMARY KEY (summary_id, node_id),
    FOREIGN KEY(summary_id, node_id) REFERENCES summary_nodes(summary_id, node_id) ON DELETE CASCADE
);

CREATE INDEX idx_summary_node_embeddings_model
ON summary_node_embeddings(embedding_model);
