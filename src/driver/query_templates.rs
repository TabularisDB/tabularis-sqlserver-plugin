//! Pure, opt-in SQL previews for the host's Generate SQL dialog.

use serde::Deserialize;

use super::helpers::{bracket_quote, qualify};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TemplateKind {
    Select,
    Update,
    Delete,
}

#[derive(Debug, Deserialize)]
pub struct TemplateRequest {
    pub table: String,
    pub schema: Option<String>,
    pub kind: TemplateKind,
    #[serde(default)]
    pub columns: Vec<String>,
    pub limit: Option<u32>,
}

pub fn build(request: &TemplateRequest) -> Result<String, String> {
    if request.table.trim().is_empty() || request.columns.iter().any(|name| name.trim().is_empty())
    {
        return Err("Table and column names must not be empty".into());
    }
    let target = qualify(request.schema.as_deref(), &request.table);
    match request.kind {
        TemplateKind::Select => {
            let top = request
                .limit
                .map(|limit| format!(" TOP ({limit})"))
                .unwrap_or_default();
            let fields = if request.columns.is_empty() {
                " *".to_string()
            } else {
                format!(
                    "\n{}",
                    request
                        .columns
                        .iter()
                        .map(|name| format!("  {}", bracket_quote(name)))
                        .collect::<Vec<_>>()
                        .join(",\n")
                )
            };
            Ok(format!("SELECT{top}{fields}\nFROM {target};"))
        }
        TemplateKind::Update | TemplateKind::Delete if request.limit.is_some() => {
            Err("Template limit is only supported for SELECT".into())
        }
        TemplateKind::Update => {
            let assignments = if request.columns.is_empty() {
                "  [column] = :value_1".to_string()
            } else {
                request
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        // Host editor placeholders, not SQL Server @parameters.
                        // Ordinals prevent collisions for similarly named columns.
                        format!("  {} = :value_{}", bracket_quote(name), index + 1)
                    })
                    .collect::<Vec<_>>()
                    .join(",\n")
            };
            Ok(format!("UPDATE {target}\nSET\n{assignments}\nWHERE 1 = 0;"))
        }
        TemplateKind::Delete => Ok(format!("DELETE\nFROM {target}\nWHERE 1 = 0;")),
    }
}

#[cfg(test)]
mod tests;
