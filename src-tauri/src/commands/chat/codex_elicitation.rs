use serde_json::{json, Value};

use super::model::{Elicitation, ElicitationAction, ElicitationResponse};

pub(super) struct McpElicitation {
    pub server: String,
    pub mode: String,
    validator: Option<jsonschema::Validator>,
}

impl McpElicitation {
    pub fn parse(params: &Value) -> Result<(Self, Elicitation), &'static str> {
        let server = params["serverName"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or("Missing MCP server name")?;
        if !params["message"].is_string() {
            return Err("Missing MCP elicitation message");
        }
        let elicitation: Elicitation = serde_json::from_value(params.clone())
            .map_err(|_| "Unsupported or malformed MCP elicitation mode")?;
        let validator = match &elicitation {
            Elicitation::Form { requested_schema } => Some(
                jsonschema::options()
                    .should_validate_formats(true)
                    .build(requested_schema)
                    .map_err(|_| "Invalid or unsupported MCP form schema (external references are unavailable)")?,
            ),
            Elicitation::Url { url, elicitation_id } => {
                let parsed = reqwest::Url::parse(url).map_err(|_| "Invalid MCP elicitation URL")?;
                if !matches!(parsed.scheme(), "http" | "https") || elicitation_id.is_empty() {
                    return Err("Unsupported MCP elicitation URL or missing elicitation id");
                }
                None
            }
        };
        Ok((
            Self {
                server: server.into(),
                mode: params["mode"].as_str().unwrap_or_default().into(),
                validator,
            },
            elicitation,
        ))
    }

    pub fn response(&self, response: &ElicitationResponse) -> Result<Value, String> {
        let action = match response.action {
            ElicitationAction::Accept => {
                if let Some(validator) = &self.validator {
                    if let Err(error) = validator.validate(&response.content) {
                        return Err(format!(
                            "Form value at {} does not satisfy {}",
                            error.instance_path, error.schema_path
                        ));
                    }
                } else if !response.content.is_null() {
                    return Err("URL confirmation must have null content".into());
                }
                "accept"
            }
            ElicitationAction::Decline => "decline",
            ElicitationAction::Cancel => "cancel",
        };
        Ok(json!({
            "action": action,
            "content": if action == "accept" { response.content.clone() } else { Value::Null },
            "_meta": null,
        }))
    }

    pub fn log_resolution(&self, id: &str, resolution: &str) {
        log::info!(
            "MCP elicitation id={id} server={} mode={} resolution={resolution}",
            self.server,
            self.mode
        );
    }
}

pub(super) fn cancelled(id: &Value) -> Value {
    json!({ "id": id, "result": { "action": "cancel", "content": null, "_meta": null } })
}

pub(super) fn request_key(id: &Value) -> String {
    format!("elicitation:{id}")
}

pub(super) fn log_request(id: &Value, params: &Value) {
    let schema = &params["requestedSchema"];
    let fields = schema["properties"].as_object();
    let types: Vec<_> = fields
        .into_iter()
        .flat_map(|fields| fields.values())
        .map(|field| match field["type"].as_str() {
            Some("string") => "string",
            Some("boolean") => "boolean",
            Some("number") => "number",
            Some("integer") => "integer",
            Some("object") => "object",
            Some("array") => "array",
            _ => "other",
        })
        .collect();
    log::info!(
        "MCP request method=mcpServer/elicitation/request id={id} server={} mode={} schema_fields={} schema_types={types:?}",
        params["serverName"].as_str().unwrap_or("missing"),
        params["mode"].as_str().unwrap_or("missing"),
        fields.map_or(0, |fields| fields.len()),
    );
}
