CREATE TABLE summary_node_semantic_terms (
    summary_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    term_hash INTEGER NOT NULL,
    weight REAL NOT NULL,
    PRIMARY KEY (summary_id, node_id, term_hash),
    FOREIGN KEY(summary_id, node_id) REFERENCES summary_nodes(summary_id, node_id) ON DELETE CASCADE
);

CREATE INDEX idx_summary_node_semantic_terms_hash
ON summary_node_semantic_terms(term_hash);

CREATE INDEX idx_summary_node_semantic_terms_summary_node
ON summary_node_semantic_terms(summary_id, node_id);
