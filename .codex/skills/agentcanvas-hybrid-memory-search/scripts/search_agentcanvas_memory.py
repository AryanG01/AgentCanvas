#!/usr/bin/env python3
"""Search AgentCanvas session summaries with sparse semantic + lexical hybrid ranking.

This script mirrors the hybrid scoring approach in:
- codex-rs/state/src/runtime/summaries.rs

It reads the local Codex SQLite state DB and searches summary index tables:
- summary_artifacts
- summary_nodes
- summary_node_semantic_terms
- summary_node_file_paths
- summary_node_commands
- summary_node_errors
"""

from __future__ import annotations

import argparse
import json
import os
import sqlite3
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

HYBRID_SEMANTIC_WEIGHT = 0.7
HYBRID_LEXICAL_WEIGHT = 0.3
MAX_FEATURES = 96


@dataclass
class RankedNode:
    summary_id: str
    thread_id: str
    session_id: str
    summary_path: str
    node_id: str
    parent_node_id: str | None
    node_type: str
    title: str | None
    node_json: str
    sample_file_path: str | None
    sample_command: str | None
    sample_error_text: str | None
    semantic_score: float
    lexical_score: float
    hybrid_score: float


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Search past Codex chat memory via AgentCanvas hybrid summary index"
    )
    parser.add_argument("query", help="Search text")
    parser.add_argument(
        "--mode",
        choices=("hybrid", "semantic", "lexical"),
        default="hybrid",
        help="Ranking strategy (default: hybrid)",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=8,
        help="Maximum number of results to return (default: 8)",
    )
    parser.add_argument(
        "--candidate-multiplier",
        type=int,
        default=4,
        help="Candidate pool size = limit * multiplier (default: 4)",
    )
    parser.add_argument(
        "--thread-id",
        help="Optional thread id filter",
    )
    parser.add_argument(
        "--codex-home",
        type=Path,
        help="Override CODEX home (defaults: CODEX_SQLITE_HOME, CODEX_HOME, ~/.codex)",
    )
    parser.add_argument(
        "--db",
        type=Path,
        help="Direct SQLite DB path (overrides --codex-home)",
    )
    parser.add_argument(
        "--json",
        action="store_true",
        help="Emit JSON output",
    )
    parser.add_argument(
        "--show-node-json",
        action="store_true",
        help="Include parsed node JSON in output",
    )
    return parser.parse_args()


def resolve_codex_home(args: argparse.Namespace) -> Path:
    if args.codex_home:
        return args.codex_home.expanduser()

    for env_name in ("CODEX_SQLITE_HOME", "CODEX_HOME"):
        env_value = os.getenv(env_name)
        if env_value:
            return Path(env_value).expanduser()

    return Path.home() / ".codex"


def resolve_db_path(args: argparse.Namespace, codex_home: Path) -> Path:
    if args.db:
        return args.db.expanduser()

    versioned: list[tuple[int, Path]] = []
    if codex_home.is_dir():
        for child in codex_home.iterdir():
            name = child.name
            if not child.is_file() or not name.startswith("state_") or not name.endswith(".sqlite"):
                continue
            version_part = name[len("state_") : -len(".sqlite")]
            if version_part.isdigit():
                versioned.append((int(version_part), child))

    if versioned:
        versioned.sort(key=lambda item: item[0], reverse=True)
        return versioned[0][1]

    legacy = codex_home / "state.sqlite"
    if legacy.exists():
        return legacy

    return codex_home / "state_5.sqlite"


def ensure_summary_tables(conn: sqlite3.Connection) -> None:
    required = {
        "summary_artifacts",
        "summary_nodes",
        "summary_node_semantic_terms",
        "summary_node_file_paths",
        "summary_node_commands",
        "summary_node_errors",
    }
    rows = conn.execute(
        "SELECT name FROM sqlite_master WHERE type='table'"
    ).fetchall()
    existing = {row[0] for row in rows}
    missing = sorted(required - existing)
    if missing:
        missing_text = ", ".join(missing)
        raise RuntimeError(
            f"Missing summary tables: {missing_text}. "
            "Run Codex with sqlite enabled and generate some session summaries first."
        )


def semantic_tokens(text: str) -> list[str]:
    tokens: list[str] = []
    current: list[str] = []
    for ch in text:
        if ch.isascii() and ch.isalnum():
            current.append(ch.lower())
            continue
        token = normalize_semantic_token("".join(current))
        if token:
            tokens.append(token)
        current.clear()

    token = normalize_semantic_token("".join(current))
    if token:
        tokens.append(token)
    return tokens


def normalize_semantic_token(token: str) -> str | None:
    if len(token) < 2:
        return None
    if token.isdigit():
        return None
    return token


def semantic_term_hash(text: str) -> int:
    # FNV-1a 32-bit, aligned with codex-rs/state/src/runtime/summaries.rs
    fnv_offset_basis = 2_166_136_261
    fnv_prime = 16_777_619

    hashed = fnv_offset_basis
    for byte in text.encode("utf-8"):
        hashed ^= byte
        hashed = (hashed * fnv_prime) & 0xFFFFFFFF
    return int(hashed)


def semantic_terms_from_text(text: str) -> list[tuple[int, float]]:
    tokens = semantic_tokens(text)
    if not tokens:
        return []

    weights_by_hash: dict[int, float] = {}

    def add_weight(term_hash: int, delta: float) -> None:
        weights_by_hash[term_hash] = weights_by_hash.get(term_hash, 0.0) + delta

    for token in tokens:
        add_weight(semantic_term_hash(f"w:{token}"), 1.0)

        token_bytes = token.encode("utf-8")
        if len(token_bytes) >= 5:
            for idx in range(len(token_bytes) - 2):
                trigram = token_bytes[idx : idx + 3].decode("utf-8", errors="ignore")
                add_weight(semantic_term_hash(f"g:{trigram}"), 0.2)

    for idx in range(len(tokens) - 1):
        bigram = f"{tokens[idx]} {tokens[idx + 1]}"
        add_weight(semantic_term_hash(f"b:{bigram}"), 0.75)

    weighted_features = list(weights_by_hash.items())
    weighted_features.sort(key=lambda item: (-item[1], item[0]))
    weighted_features = weighted_features[:MAX_FEATURES]

    norm = sum(weight * weight for _, weight in weighted_features) ** 0.5
    if norm <= sys.float_info.epsilon:
        return []

    result: list[tuple[int, float]] = []
    for term_hash, raw_weight in weighted_features:
        normalized_weight = raw_weight / norm
        weight_milli = round(normalized_weight * 1000.0)
        if weight_milli == 0:
            continue
        result.append((term_hash, weight_milli / 1000.0))

    result.sort(key=lambda item: item[0])
    return result


def lexical_terms_from_text(text: str) -> list[str]:
    return sorted(set(semantic_tokens(text)))


def values_clause(rows: int, columns: int) -> str:
    single = "(" + ", ".join(["?"] * columns) + ")"
    return ", ".join([single] * rows)


def build_thread_filter_sql(thread_id: str | None) -> tuple[str, list[Any]]:
    if not thread_id:
        return "", []
    return "WHERE artifacts.thread_id = ?", [thread_id]


def search_semantic(
    conn: sqlite3.Connection,
    query: str,
    limit: int,
    thread_id: str | None,
) -> dict[tuple[str, str], RankedNode]:
    terms = semantic_terms_from_text(query)
    if not terms:
        return {}

    values_sql = values_clause(len(terms), 2)
    thread_filter_sql, thread_params = build_thread_filter_sql(thread_id)

    sql = f"""
WITH query_terms(term_hash, query_weight) AS (
    VALUES {values_sql}
),
scored_nodes AS (
    SELECT
        terms.summary_id,
        terms.node_id,
        SUM(terms.weight * query_terms.query_weight) AS semantic_score
    FROM summary_node_semantic_terms AS terms
    INNER JOIN query_terms
        ON query_terms.term_hash = terms.term_hash
    INNER JOIN summary_artifacts AS artifacts
        ON artifacts.summary_id = terms.summary_id
    {thread_filter_sql}
    GROUP BY terms.summary_id, terms.node_id
    ORDER BY semantic_score DESC
    LIMIT ?
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
    file_paths.sample_file_path,
    commands.sample_command,
    errors.sample_error_text,
    scored_nodes.semantic_score
FROM scored_nodes
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = scored_nodes.summary_id
    AND nodes.node_id = scored_nodes.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = scored_nodes.summary_id
LEFT JOIN (
    SELECT summary_id, node_id, MIN(file_path) AS sample_file_path
    FROM summary_node_file_paths
    GROUP BY summary_id, node_id
) AS file_paths
    ON file_paths.summary_id = nodes.summary_id
    AND file_paths.node_id = nodes.node_id
LEFT JOIN (
    SELECT summary_id, node_id, MIN(command) AS sample_command
    FROM summary_node_commands
    GROUP BY summary_id, node_id
) AS commands
    ON commands.summary_id = nodes.summary_id
    AND commands.node_id = nodes.node_id
LEFT JOIN (
    SELECT summary_id, node_id, MIN(error_text) AS sample_error_text
    FROM summary_node_errors
    GROUP BY summary_id, node_id
) AS errors
    ON errors.summary_id = nodes.summary_id
    AND errors.node_id = nodes.node_id
ORDER BY scored_nodes.semantic_score DESC, artifacts.updated_at DESC, nodes.node_id ASC
"""

    params: list[Any] = []
    for term_hash, query_weight in terms:
        params.extend([term_hash, query_weight])
    params.extend(thread_params)
    params.append(limit)

    rows = conn.execute(sql, params).fetchall()
    out: dict[tuple[str, str], RankedNode] = {}
    for row in rows:
        key = (row[0], row[4])
        out[key] = RankedNode(
            summary_id=row[0],
            thread_id=row[1],
            session_id=row[2],
            summary_path=row[3],
            node_id=row[4],
            parent_node_id=row[5],
            node_type=row[6],
            title=row[7],
            node_json=row[8],
            sample_file_path=row[9],
            sample_command=row[10],
            sample_error_text=row[11],
            semantic_score=float(row[12] or 0.0),
            lexical_score=0.0,
            hybrid_score=float(row[12] or 0.0),
        )
    return out


def search_lexical(
    conn: sqlite3.Connection,
    query: str,
    limit: int,
    thread_id: str | None,
) -> dict[tuple[str, str], RankedNode]:
    terms = lexical_terms_from_text(query)
    if not terms:
        return {}

    values_sql = values_clause(len(terms), 1)
    thread_filter_sql, thread_params = build_thread_filter_sql(thread_id)

    sql = f"""
WITH query_terms(term) AS (
    VALUES {values_sql}
),
lexical_hits AS (
    SELECT
        file_paths.summary_id,
        file_paths.node_id,
        0.8 AS lexical_weight
    FROM summary_node_file_paths AS file_paths
    INNER JOIN query_terms
        ON INSTR(LOWER(file_paths.file_path), query_terms.term) > 0
    UNION ALL
    SELECT
        commands.summary_id,
        commands.node_id,
        1.0 AS lexical_weight
    FROM summary_node_commands AS commands
    INNER JOIN query_terms
        ON INSTR(LOWER(commands.command), query_terms.term) > 0
    UNION ALL
    SELECT
        errors.summary_id,
        errors.node_id,
        1.2 AS lexical_weight
    FROM summary_node_errors AS errors
    INNER JOIN query_terms
        ON INSTR(LOWER(errors.error_text), query_terms.term) > 0
    UNION ALL
    SELECT
        nodes.summary_id,
        nodes.node_id,
        0.6 AS lexical_weight
    FROM summary_nodes AS nodes
    INNER JOIN query_terms
        ON INSTR(LOWER(COALESCE(nodes.title, '')), query_terms.term) > 0
),
scored_nodes AS (
    SELECT
        lexical_hits.summary_id,
        lexical_hits.node_id,
        SUM(lexical_hits.lexical_weight) AS lexical_score
    FROM lexical_hits
    INNER JOIN summary_artifacts AS artifacts
        ON artifacts.summary_id = lexical_hits.summary_id
    {thread_filter_sql}
    GROUP BY lexical_hits.summary_id, lexical_hits.node_id
    ORDER BY lexical_score DESC
    LIMIT ?
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
    file_paths.sample_file_path,
    commands.sample_command,
    errors.sample_error_text,
    scored_nodes.lexical_score
FROM scored_nodes
INNER JOIN summary_nodes AS nodes
    ON nodes.summary_id = scored_nodes.summary_id
    AND nodes.node_id = scored_nodes.node_id
INNER JOIN summary_artifacts AS artifacts
    ON artifacts.summary_id = scored_nodes.summary_id
LEFT JOIN (
    SELECT summary_id, node_id, MIN(file_path) AS sample_file_path
    FROM summary_node_file_paths
    GROUP BY summary_id, node_id
) AS file_paths
    ON file_paths.summary_id = nodes.summary_id
    AND file_paths.node_id = nodes.node_id
LEFT JOIN (
    SELECT summary_id, node_id, MIN(command) AS sample_command
    FROM summary_node_commands
    GROUP BY summary_id, node_id
) AS commands
    ON commands.summary_id = nodes.summary_id
    AND commands.node_id = nodes.node_id
LEFT JOIN (
    SELECT summary_id, node_id, MIN(error_text) AS sample_error_text
    FROM summary_node_errors
    GROUP BY summary_id, node_id
) AS errors
    ON errors.summary_id = nodes.summary_id
    AND errors.node_id = nodes.node_id
ORDER BY scored_nodes.lexical_score DESC, artifacts.updated_at DESC, nodes.node_id ASC
"""

    params: list[Any] = []
    for term in terms:
        params.append(term)
    params.extend(thread_params)
    params.append(limit)

    rows = conn.execute(sql, params).fetchall()
    out: dict[tuple[str, str], RankedNode] = {}
    for row in rows:
        key = (row[0], row[4])
        out[key] = RankedNode(
            summary_id=row[0],
            thread_id=row[1],
            session_id=row[2],
            summary_path=row[3],
            node_id=row[4],
            parent_node_id=row[5],
            node_type=row[6],
            title=row[7],
            node_json=row[8],
            sample_file_path=row[9],
            sample_command=row[10],
            sample_error_text=row[11],
            semantic_score=0.0,
            lexical_score=float(row[12] or 0.0),
            hybrid_score=float(row[12] or 0.0),
        )
    return out


def merge_hybrid(
    semantic: dict[tuple[str, str], RankedNode],
    lexical: dict[tuple[str, str], RankedNode],
    limit: int,
) -> list[RankedNode]:
    merged = dict(semantic)

    for key, lex_item in lexical.items():
        existing = merged.get(key)
        if existing:
            if lex_item.lexical_score > existing.lexical_score:
                existing.lexical_score = lex_item.lexical_score
            continue
        merged[key] = lex_item

    merged_list = list(merged.values())
    max_semantic = max((item.semantic_score for item in merged_list), default=0.0)
    max_lexical = max((item.lexical_score for item in merged_list), default=0.0)

    for item in merged_list:
        semantic_component = item.semantic_score / max_semantic if max_semantic > 0.0 else 0.0
        lexical_component = item.lexical_score / max_lexical if max_lexical > 0.0 else 0.0
        item.hybrid_score = (HYBRID_SEMANTIC_WEIGHT * semantic_component) + (
            HYBRID_LEXICAL_WEIGHT * lexical_component
        )

    merged_list.sort(
        key=lambda item: (
            -item.hybrid_score,
            -item.semantic_score,
            -item.lexical_score,
            item.summary_id,
            item.node_id,
        )
    )
    return merged_list[:limit]


def serialize_item(item: RankedNode, include_node_json: bool) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "summary_id": item.summary_id,
        "thread_id": item.thread_id,
        "session_id": item.session_id,
        "summary_path": item.summary_path,
        "node_id": item.node_id,
        "parent_node_id": item.parent_node_id,
        "node_type": item.node_type,
        "title": item.title,
        "sample_file_path": item.sample_file_path,
        "sample_command": item.sample_command,
        "sample_error_text": item.sample_error_text,
        "semantic_score": round(item.semantic_score, 6),
        "lexical_score": round(item.lexical_score, 6),
        "hybrid_score": round(item.hybrid_score, 6),
    }
    if include_node_json:
        try:
            payload["node"] = json.loads(item.node_json)
        except json.JSONDecodeError:
            payload["node"] = item.node_json
    return payload


def print_text(results: list[RankedNode]) -> None:
    if not results:
        print("No matches found.")
        return

    for idx, item in enumerate(results, start=1):
        title = item.title.strip() if item.title else "(no title)"
        print(f"[{idx}] score={item.hybrid_score:.4f} sem={item.semantic_score:.4f} lex={item.lexical_score:.4f}")
        print(
            "    "
            f"thread={item.thread_id} session={item.session_id} "
            f"summary={item.summary_id} node={item.node_id}"
        )
        print(f"    type={item.node_type} title={title}")
        if item.sample_file_path:
            print(f"    file={item.sample_file_path}")
        if item.sample_command:
            print(f"    command={item.sample_command}")
        if item.sample_error_text:
            error_text = " ".join(item.sample_error_text.splitlines()).strip()
            if len(error_text) > 180:
                error_text = f"{error_text[:177]}..."
            print(f"    error={error_text}")
        print(f"    summary_path={item.summary_path}")


def run() -> int:
    args = parse_args()

    if args.limit <= 0:
        print("--limit must be > 0", file=sys.stderr)
        return 2
    if args.candidate_multiplier <= 0:
        print("--candidate-multiplier must be > 0", file=sys.stderr)
        return 2

    query = args.query.strip()
    if not query:
        print("query must not be empty", file=sys.stderr)
        return 2

    codex_home = resolve_codex_home(args)
    db_path = resolve_db_path(args, codex_home)

    if not db_path.exists():
        print(
            f"State DB not found at {db_path}. "
            "Set --db/--codex-home or ensure SQLite state is initialized.",
            file=sys.stderr,
        )
        return 1

    conn = sqlite3.connect(str(db_path))
    conn.row_factory = sqlite3.Row

    try:
        ensure_summary_tables(conn)

        count_row = conn.execute("SELECT COUNT(*) AS c FROM summary_artifacts").fetchone()
        if not count_row or int(count_row["c"]) == 0:
            print(
                "Summary index is empty. Run Codex sessions with sqlite enabled to populate AgentCanvas summaries.",
                file=sys.stderr,
            )
            return 1

        candidate_limit = max(args.limit, args.limit * args.candidate_multiplier)

        semantic = search_semantic(conn, query, candidate_limit, args.thread_id)
        lexical = search_lexical(conn, query, candidate_limit, args.thread_id)

        if args.mode == "semantic":
            results = sorted(
                semantic.values(),
                key=lambda item: (
                    -item.semantic_score,
                    item.summary_id,
                    item.node_id,
                ),
            )[: args.limit]
            for item in results:
                item.hybrid_score = item.semantic_score
        elif args.mode == "lexical":
            results = sorted(
                lexical.values(),
                key=lambda item: (
                    -item.lexical_score,
                    item.summary_id,
                    item.node_id,
                ),
            )[: args.limit]
            for item in results:
                item.hybrid_score = item.lexical_score
        else:
            results = merge_hybrid(semantic, lexical, args.limit)

        if args.json:
            payload = {
                "query": query,
                "mode": args.mode,
                "db_path": str(db_path),
                "count": len(results),
                "results": [
                    serialize_item(item, include_node_json=args.show_node_json)
                    for item in results
                ],
            }
            print(json.dumps(payload, indent=2))
        else:
            print(f"query={query!r} mode={args.mode} db={db_path}")
            print_text(results)

        return 0
    except RuntimeError as err:
        print(str(err), file=sys.stderr)
        return 1
    finally:
        conn.close()


if __name__ == "__main__":
    raise SystemExit(run())
