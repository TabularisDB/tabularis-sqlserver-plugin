use serde_json::Value;

use crate::driver::query_templates::{self, TemplateRequest};
use crate::rpc::{req_field, respond};

/// Generation is pure: do not acquire a connection or execute the preview.
pub fn get_table_query_template(id: Value, params: &Value) -> Value {
    let result = req_field::<TemplateRequest>(params, "request")
        .and_then(|request| query_templates::build(&request));
    respond(id, result)
}
