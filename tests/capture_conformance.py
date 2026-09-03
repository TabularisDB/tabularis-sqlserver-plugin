#!/usr/bin/env python3
"""Capture one successful JSON-RPC response for every implemented plugin RPC.

Run against the SQL Server container started by `just run-sqlserver`:

    cargo build
    python3 tests/capture_conformance.py

Connection and binary paths use the same SQLSERVER_TEST_* and
SQLSERVER_PLUGIN_BIN overrides as tests/live_db.rs. Fixtures never contain
requests, so credentials are not written to the repository.
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import tempfile
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parent.parent
FIXTURE_DIR = ROOT / "tests" / "fixtures" / "conformance"
SCHEMA = "ss044"
USER = "ss044_user"
LOGIN = "ss044_login"
PASSWORD = "Ss044!Conformance9"
NEW_PASSWORD = "Ss044!Conformance10"


class Plugin:
    def __init__(self) -> None:
        binary = os.environ.get(
            "SQLSERVER_PLUGIN_BIN", str(ROOT / "target" / "debug" / "sqlserver-plugin")
        )
        self.process = subprocess.Popen(
            [binary],
            cwd=ROOT,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            text=True,
        )

    def call(self, method: str, params: dict[str, Any]) -> dict[str, Any]:
        assert self.process.stdin is not None
        assert self.process.stdout is not None
        request = {
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        }
        self.process.stdin.write(json.dumps(request, separators=(",", ":")) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError(f"plugin exited while handling {method}")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(f"{method} failed: {response['error']}")
        return response

    def close(self) -> None:
        if self.process.poll() is None:
            self.process.terminate()
        self.process.wait(timeout=5)


def connection_params() -> dict[str, Any]:
    return {
        "driver": "sqlserver",
        "host": os.environ.get("SQLSERVER_TEST_HOST", "127.0.0.1"),
        "port": int(os.environ.get("SQLSERVER_TEST_PORT", "1433")),
        "username": os.environ.get("SQLSERVER_TEST_USER", "sa"),
        "password": os.environ.get("SQLSERVER_TEST_PASSWORD", "Str0ng!Passw0rd"),
        "database": os.environ.get("SQLSERVER_TEST_DATABASE", "tabularis_test"),
        "ssl_mode": "require",
        "connection_id": "ss044-conformance-capture",
    }


def main() -> None:
    plugin = Plugin()
    params = connection_params()
    FIXTURE_DIR.parent.mkdir(parents=True, exist_ok=True)
    staging = Path(
        tempfile.mkdtemp(prefix=".ss044-conformance-", dir=FIXTURE_DIR.parent)
    )
    captured: set[str] = set()

    def rpc_params(**values: Any) -> dict[str, Any]:
        return {"params": params, **values}

    def execute(sql: str) -> Any:
        return plugin.call("execute_query", rpc_params(query=sql))["result"]

    def capture(method: str, values: dict[str, Any]) -> Any:
        if method in captured:
            raise RuntimeError(f"duplicate fixture for {method}")
        response = plugin.call(method, values)
        (staging / f"{method}.json").write_text(
            json.dumps(response, indent=2, sort_keys=True) + "\n", encoding="utf-8"
        )
        captured.add(method)
        return response["result"]

    try:
        capture(
            "initialize",
            {
                "settings": {
                    "application_name": "Tabularis SS-044 conformance capture",
                    "query_timeout_seconds": 30,
                }
            },
        )

        execute(
            f"IF DATABASE_PRINCIPAL_ID(N'{USER}') IS NOT NULL DROP USER [{USER}]; "
            f"IF SUSER_ID(N'{LOGIN}') IS NOT NULL DROP LOGIN [{LOGIN}]; "
            f"DROP VIEW IF EXISTS [{SCHEMA}].[base_view]; "
            f"DROP VIEW IF EXISTS [{SCHEMA}].[created_view]; "
            f"DROP PROCEDURE IF EXISTS [{SCHEMA}].[sample_proc]; "
            f"DROP PROCEDURE IF EXISTS [{SCHEMA}].[drop_proc]; "
            f"DROP TABLE IF EXISTS [{SCHEMA}].[drop_fk_child]; "
            f"DROP TABLE IF EXISTS [{SCHEMA}].[crud_rows]; "
            f"DROP TABLE IF EXISTS [{SCHEMA}].[blob_rows]; "
            f"DROP TABLE IF EXISTS [{SCHEMA}].[child]; "
            f"DROP TABLE IF EXISTS [{SCHEMA}].[parent]; "
            f"IF SCHEMA_ID(N'{SCHEMA}') IS NULL EXEC(N'CREATE SCHEMA [{SCHEMA}]')"
        )
        execute(
            f"CREATE TABLE [{SCHEMA}].[parent] (id INT PRIMARY KEY); "
            f"CREATE TABLE [{SCHEMA}].[child] ("
            "id INT IDENTITY(1,1) PRIMARY KEY, "
            "parent_id INT NULL, "
            "label NVARCHAR(42) NOT NULL CONSTRAINT [df_ss044_label] DEFAULT N'pending', "
            "note NVARCHAR(MAX) NULL, "
            "generated_value AS (id + 1), "
            f"CONSTRAINT [fk_child_parent] FOREIGN KEY (parent_id) REFERENCES [{SCHEMA}].[parent](id) ON DELETE SET NULL); "
            f"CREATE UNIQUE INDEX [ix_child_label] ON [{SCHEMA}].[child] (label); "
            f"CREATE INDEX [ix_drop] ON [{SCHEMA}].[child] (parent_id); "
            f"CREATE TABLE [{SCHEMA}].[drop_fk_child] (id INT PRIMARY KEY, parent_id INT NULL, "
            f"CONSTRAINT [fk_drop] FOREIGN KEY (parent_id) REFERENCES [{SCHEMA}].[parent](id)); "
            f"CREATE TABLE [{SCHEMA}].[crud_rows] (id INT PRIMARY KEY, value NVARCHAR(20) NOT NULL); "
            f"CREATE TABLE [{SCHEMA}].[blob_rows] (id INT PRIMARY KEY, payload VARBINARY(MAX) NOT NULL); "
            f"INSERT INTO [{SCHEMA}].[parent] VALUES (1), (2); "
            f"INSERT INTO [{SCHEMA}].[child] (parent_id, label, note) VALUES (1, N'alpha', NULL), (2, N'beta', N'note'); "
            f"INSERT INTO [{SCHEMA}].[blob_rows] VALUES (1, 0x89504E470D0A1A0A0000000D49484452)"
        )
        execute(f"CREATE VIEW [{SCHEMA}].[base_view] AS SELECT id, label FROM [{SCHEMA}].[child]")
        execute(
            f"CREATE PROCEDURE [{SCHEMA}].[sample_proc] "
            "@input INT, @output NVARCHAR(20) OUTPUT AS BEGIN SET NOCOUNT ON; "
            "SET @output = CONCAT(N'value-', @input); SELECT @input AS input_value; END"
        )
        execute(f"CREATE PROCEDURE [{SCHEMA}].[drop_proc] AS SELECT 1 AS value")
        execute(
            f"CREATE TRIGGER [{SCHEMA}].[base_after] ON [{SCHEMA}].[child] "
            "AFTER INSERT, UPDATE AS BEGIN SET NOCOUNT ON; END"
        )
        execute(
            f"CREATE TRIGGER [{SCHEMA}].[base_instead] ON [{SCHEMA}].[child] "
            "INSTEAD OF DELETE AS BEGIN SET NOCOUNT ON; END"
        )

        capture("ping", rpc_params())
        capture("test_connection", rpc_params())
        capture("get_databases", rpc_params())
        capture("get_schemas", rpc_params())
        capture("get_tables", rpc_params(schema=SCHEMA))
        capture("get_columns", rpc_params(schema=SCHEMA, table="child"))
        capture("get_foreign_keys", rpc_params(schema=SCHEMA, table="child"))
        capture("get_indexes", rpc_params(schema=SCHEMA, table="child"))
        capture("get_schema_snapshot", rpc_params(schema=SCHEMA))
        capture("get_all_columns_batch", rpc_params(schema=SCHEMA))
        capture("get_all_foreign_keys_batch", rpc_params(schema=SCHEMA))
        capture("get_ai_schema_context", rpc_params(schema=SCHEMA, max_tables=3))

        capture("get_views", rpc_params(schema=SCHEMA))
        capture(
            "get_view_definition",
            rpc_params(schema=SCHEMA, view_name="base_view"),
        )
        capture("get_view_columns", rpc_params(schema=SCHEMA, view_name="base_view"))
        capture(
            "create_view",
            rpc_params(
                schema=SCHEMA,
                view_name="created_view",
                definition=f"SELECT id FROM [{SCHEMA}].[parent]",
            ),
        )
        capture(
            "alter_view",
            rpc_params(
                schema=SCHEMA,
                view_name="created_view",
                definition=f"SELECT id FROM [{SCHEMA}].[parent] WHERE id > 0",
            ),
        )
        capture("drop_view", rpc_params(schema=SCHEMA, view_name="created_view"))

        capture("get_routines", rpc_params(schema=SCHEMA))
        capture(
            "get_routine_parameters",
            rpc_params(schema=SCHEMA, routine_name="sample_proc"),
        )
        capture(
            "get_routine_definition",
            rpc_params(
                schema=SCHEMA,
                routine_name="sample_proc",
                routine_type="PROCEDURE",
            ),
        )
        capture(
            "build_routine_call_sql",
            rpc_params(
                schema=SCHEMA,
                routine_name="sample_proc",
                routine_type="PROCEDURE",
                args=[
                    {"name": "@input", "mode": "IN", "value": "7", "is_raw": True},
                    {
                        "name": "@output",
                        "mode": "INOUT",
                        "value": None,
                        "is_raw": False,
                    },
                ],
            ),
        )
        capture("routine_create_template", {"schema": SCHEMA, "routine_type": "FUNCTION"})
        capture(
            "get_routine_edit_script",
            rpc_params(
                schema=SCHEMA,
                routine_name="sample_proc",
                routine_type="PROCEDURE",
            ),
        )
        capture(
            "drop_routine",
            rpc_params(schema=SCHEMA, routine_name="drop_proc", routine_type="PROCEDURE"),
        )

        capture("get_triggers", rpc_params(schema=SCHEMA))
        capture(
            "get_trigger_definition",
            rpc_params(schema=SCHEMA, trigger_name="base_after", table_name="child"),
        )
        capture(
            "create_trigger",
            rpc_params(
                schema=SCHEMA,
                trigger_sql=f"CREATE TRIGGER [{SCHEMA}].[created_trigger] ON [{SCHEMA}].[parent] AFTER UPDATE AS BEGIN SET NOCOUNT ON; END",
            ),
        )
        capture(
            "drop_trigger",
            rpc_params(schema=SCHEMA, trigger_name="created_trigger", table_name="parent"),
        )

        capture("get_db_privilege_catalog", {})
        capture(
            "create_db_user",
            rpc_params(user=USER, host=LOGIN, password=PASSWORD),
        )
        capture(
            "set_db_user_password",
            rpc_params(user=USER, host=LOGIN, password=NEW_PASSWORD),
        )
        capture(
            "apply_db_user_privileges",
            rpc_params(
                user=USER,
                host=LOGIN,
                database=SCHEMA,
                table="parent",
                privileges=["SELECT"],
                grant=True,
            ),
        )
        capture("get_db_users", rpc_params())
        capture("get_db_user_grants", rpc_params(user=USER, host=LOGIN))
        capture("get_db_user_privileges", rpc_params(user=USER, host=LOGIN))
        capture("drop_db_user", rpc_params(user=USER, host=LOGIN))

        capture(
            "execute_query",
            rpc_params(
                query=(
                    f"SELECT id, label FROM [{SCHEMA}].[child] ORDER BY id; "
                    f"SELECT parent_id FROM [{SCHEMA}].[child] ORDER BY id"
                )
            ),
        )
        capture(
            "execute_query_batch",
            rpc_params(
                queries=[
                    f"SELECT id FROM [{SCHEMA}].[child] ORDER BY id",
                    f"SELECT * FROM [{SCHEMA}].[missing_table]",
                ],
                limit=1,
                page=1,
            ),
        )
        capture(
            "explain_query",
            rpc_params(
                query=f"SELECT label FROM [{SCHEMA}].[child] WHERE id = 1",
                analyze=False,
            ),
        )

        capture(
            "insert_record",
            rpc_params(schema=SCHEMA, table="crud_rows", data={"id": 1, "value": "before"}),
        )
        capture(
            "update_record",
            rpc_params(
                schema=SCHEMA,
                table="crud_rows",
                pk_map={"id": 1},
                col_name="value",
                new_val="after",
            ),
        )
        capture(
            "delete_record",
            rpc_params(schema=SCHEMA, table="crud_rows", pk_map={"id": 1}),
        )

        blob_path = Path(tempfile.gettempdir()) / "ss044-conformance-blob.bin"
        blob_path.unlink(missing_ok=True)
        capture(
            "save_blob_to_file",
            rpc_params(
                schema=SCHEMA,
                table="blob_rows",
                col_name="payload",
                pk_map={"id": 1},
                file_path=str(blob_path),
            ),
        )
        blob_path.unlink(missing_ok=True)
        capture(
            "fetch_blob_as_data_url",
            rpc_params(
                schema=SCHEMA,
                table="blob_rows",
                col_name="payload",
                pk_map={"id": 1},
                max_blob_size=1024,
            ),
        )

        column = {
            "name": "value",
            "data_type": "NVARCHAR(40)",
            "is_nullable": True,
            "is_pk": False,
            "is_auto_increment": False,
            "default_value": None,
        }
        capture(
            "get_create_table_sql",
            {
                "schema": SCHEMA,
                "table_name": "generated_table",
                "columns": [
                    {
                        **column,
                        "name": "id",
                        "data_type": "INT",
                        "is_nullable": False,
                        "is_pk": True,
                    },
                    column,
                ],
            },
        )
        capture(
            "get_add_column_sql",
            {"schema": SCHEMA, "table": "generated_table", "column": column},
        )
        capture(
            "get_alter_column_sql",
            {
                "schema": SCHEMA,
                "table": "generated_table",
                "old_column": column,
                "new_column": {**column, "data_type": "NVARCHAR(80)", "is_nullable": False},
            },
        )
        capture(
            "get_create_index_sql",
            {
                "schema": SCHEMA,
                "table": "generated_table",
                "index_name": "ix_generated_value",
                "columns": ["value"],
                "is_unique": True,
            },
        )
        capture(
            "get_create_foreign_key_sql",
            {
                "params": params,
                "schema": SCHEMA,
                "table": "generated_table",
                "fk_name": "fk_generated_parent",
                "column": "id",
                "ref_table": "parent",
                "ref_column": "id",
                "on_delete": "CASCADE",
                "on_update": "NO ACTION",
            },
        )
        capture(
            "drop_index",
            rpc_params(schema=SCHEMA, table="child", index_name="ix_drop"),
        )
        capture(
            "drop_foreign_key",
            rpc_params(schema=SCHEMA, table="drop_fk_child", fk_name="fk_drop"),
        )

        capture("shutdown", {})

        expected = {
            "initialize",
            "ping",
            "test_connection",
            "shutdown",
            "get_databases",
            "get_schemas",
            "get_tables",
            "get_columns",
            "get_foreign_keys",
            "get_indexes",
            "get_schema_snapshot",
            "get_all_columns_batch",
            "get_all_foreign_keys_batch",
            "get_ai_schema_context",
            "get_views",
            "get_view_definition",
            "get_view_columns",
            "create_view",
            "alter_view",
            "drop_view",
            "get_routines",
            "get_routine_parameters",
            "get_routine_definition",
            "build_routine_call_sql",
            "routine_create_template",
            "get_routine_edit_script",
            "drop_routine",
            "get_triggers",
            "get_trigger_definition",
            "create_trigger",
            "drop_trigger",
            "get_db_privilege_catalog",
            "get_db_users",
            "create_db_user",
            "drop_db_user",
            "set_db_user_password",
            "get_db_user_grants",
            "get_db_user_privileges",
            "apply_db_user_privileges",
            "execute_query",
            "execute_query_batch",
            "explain_query",
            "insert_record",
            "update_record",
            "delete_record",
            "save_blob_to_file",
            "fetch_blob_as_data_url",
            "get_create_table_sql",
            "get_add_column_sql",
            "get_alter_column_sql",
            "get_create_index_sql",
            "get_create_foreign_key_sql",
            "drop_index",
            "drop_foreign_key",
        }
        if captured != expected:
            raise RuntimeError(
                f"fixture inventory mismatch; missing={expected - captured}, extra={captured - expected}"
            )

        if FIXTURE_DIR.exists():
            shutil.rmtree(FIXTURE_DIR)
        staging.rename(FIXTURE_DIR)
        print(f"captured {len(captured)} responses in {FIXTURE_DIR}")
    finally:
        if staging.exists():
            shutil.rmtree(staging)
        plugin.close()


if __name__ == "__main__":
    main()
