# Optional table query templates

The plugin advertises `capabilities.table_query_templates: true` and implements
`get_table_query_template` for compatible Tabularis hosts. The request and result
are additive: existing RPC methods and the minimum runtime version are unchanged.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "get_table_query_template",
  "params": {
    "params": { "driver": "sqlserver" },
    "request": {
      "table": "orders",
      "schema": "sales",
      "kind": "select",
      "columns": ["id", "status"],
      "limit": 100
    }
  }
}
```

The result is a string:

```sql
SELECT TOP (100)
  [id],
  [status]
FROM [sales].[orders];
```

- `kind`: `select`, `update` or `delete`.
- Names are unquoted identifiers. The plugin applies SQL Server bracket escaping.
- `columns` defaults to `[]`; SELECT then uses `*`, UPDATE emits a placeholder.
- Missing/null `schema` uses `dbo`; missing/null `limit` leaves SELECT unbounded.
- `limit` is a non-negative u32, including zero, and is rejected for UPDATE/DELETE.
- UPDATE emits unique `:value_N` host-editor placeholders. Both UPDATE and DELETE
  include `WHERE 1 = 0` so the preview cannot accidentally modify all rows.
- This method does not open a database connection or execute SQL.

## Compatibility and rollout

1. Merge the pagination fix in [PR #30](https://github.com/TabularisDB/tabularis-sqlserver-plugin/pull/30).
2. Register the optional capability in Tabularium's driver-kind schema (below).
3. Release Tabularis with the optional template RPC. Drivers without the capability
   retain legacy generation. Only a remote `-32601` selects the legacy fallback;
   real errors and malformed results are surfaced.
4. Release this plugin's template support. The full issue #26 fix requires both
   the host extension and the plugin pagination fix. Older hosts continue working,
   but their Generate SQL dialog still uses the old generation logic.

CREATE TABLE inspection remains unchanged in this extension. It is separate from
these query templates and from the existing DDL RPC contract.

## Tabularium

Tabularium distributes and validates the manifest; it does not participate in
runtime SQL generation. Register the following optional property under the driver
kind's `capabilities.properties` for validation and generated docs:

```json
{
  "table_query_templates": {
    "type": "boolean",
    "default": false,
    "description": "Generate SQL SELECT/UPDATE/DELETE previews through the optional get_table_query_template RPC."
  }
}
```

Do not replace the other capability definitions or add this flag to `required`.
Although the registry's current capabilities schema allows additional properties,
its ingestion calls `validateManifest` with `lenient: true` (AJV
`removeAdditional: 'all'`). Undeclared capability keys are therefore stripped from
the normalized registry metadata. Register the property **before publishing** the
plugin release; if it was already ingested, refresh its manifest afterward.

This is a registry administrator's schema/configuration update, not a backend or
SDK protocol change. No database migration, new API endpoint, historical release
archive rewrite or minimum-host-version increase is needed.
