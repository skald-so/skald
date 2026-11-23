//! Validate the A2A 1.0 JSON-RPC request before any Skald I/O.

use axum::Json;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value, error::Category, json};

use super::TEXT;

#[derive(Deserialize)]
pub(super) struct Request {
    jsonrpc: String,
    pub(super) id: Value,
    method: String,
    #[serde(default)]
    params: Value,
}

impl Request {
    pub(super) fn read(bytes: &[u8]) -> Result<Self, Error> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|error| match error.classify() {
                Category::Data => Error::InvalidRequest,
                _ => Error::Parse,
            })?;
        if request.jsonrpc != "2.0"
            || request.method.is_empty()
            || !(request.id.is_null()
                || request.id.is_string()
                || request.id.is_i64()
                || request.id.is_u64())
        {
            return Err(Error::InvalidRequest);
        }
        Ok(request)
    }

    pub(super) fn operation(&self) -> Result<Operation, Error> {
        match self.method.as_str() {
            "SendMessage" => self.send(),
            "GetTask" => {
                let task: GetTask = decode(&self.params)?;
                if task.id.is_empty() || task.history_length.is_some_and(|length| length < 0) {
                    return Err(Error::InvalidParams);
                }
                Ok(Operation::Existing {
                    task: task.id,
                    context: None,
                })
            }
            "CreateTaskPushNotificationConfig"
            | "GetTaskPushNotificationConfig"
            | "ListTaskPushNotificationConfigs"
            | "DeleteTaskPushNotificationConfig" => Err(Error::Push),
            _ => Err(Error::Unsupported),
        }
    }

    fn send(&self) -> Result<Operation, Error> {
        let SendMessage {
            message,
            configuration,
            ..
        } = decode(&self.params)?;
        if let Some(config) = configuration {
            if config.task_push_notification_config.is_some() {
                return Err(Error::Push);
            }
            if config.history_length.is_some_and(|length| length < 0) {
                return Err(Error::InvalidParams);
            }
            if !config.accepted_output_modes.is_empty()
                && !config
                    .accepted_output_modes
                    .iter()
                    .any(|mode| mode.eq_ignore_ascii_case(TEXT))
            {
                return Err(Error::Content);
            }
        }
        if message.message_id.is_empty()
            || message.role != "ROLE_USER"
            || !message.reference_task_ids.is_empty()
            || message.parts.is_empty()
            || message.task_id.as_deref() == Some("")
            || message.context_id.as_deref() == Some("")
            || (message.task_id.is_none() && message.context_id.is_some())
        {
            return Err(Error::InvalidParams);
        }
        for part in message.parts {
            if part
                .extra
                .keys()
                .any(|field| matches!(field.as_str(), "raw" | "url" | "data" | "filename"))
                || part
                    .media_type
                    .is_some_and(|media_type| !media_type.eq_ignore_ascii_case(TEXT))
            {
                return Err(Error::Content);
            }
            if part.text.is_none() || !part.extra.is_empty() {
                return Err(Error::InvalidParams);
            }
        }
        Ok(match message.task_id {
            Some(task) => Operation::Existing {
                task,
                context: message.context_id,
            },
            None => Operation::SendNew,
        })
    }
}

fn decode<'a, T: Deserialize<'a>>(value: &'a Value) -> Result<T, Error> {
    T::deserialize(value).map_err(|_| Error::InvalidParams)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct GetTask {
    id: String,
    history_length: Option<i32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SendMessage {
    message: Message,
    configuration: Option<Configuration>,
    #[serde(rename = "metadata")]
    _metadata: Option<Map<String, Value>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Configuration {
    task_push_notification_config: Option<Value>,
    history_length: Option<i32>,
    #[serde(default)]
    accepted_output_modes: Vec<String>,
    #[serde(rename = "returnImmediately")]
    _return_immediately: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Message {
    message_id: String,
    role: String,
    parts: Vec<Part>,
    context_id: Option<String>,
    task_id: Option<String>,
    #[serde(default)]
    reference_task_ids: Vec<String>,
    #[serde(rename = "metadata")]
    _metadata: Option<Map<String, Value>>,
    #[serde(rename = "extensions")]
    _extensions: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Part {
    text: Option<String>,
    media_type: Option<String>,
    #[serde(rename = "metadata")]
    _metadata: Option<Map<String, Value>>,
    #[serde(flatten)]
    extra: Map<String, Value>,
}

pub(super) enum Operation {
    SendNew,
    Existing {
        task: String,
        context: Option<String>,
    },
}

pub(super) enum Error {
    Parse,
    InvalidRequest,
    InvalidParams,
    TaskNotFound,
    Push,
    Unsupported,
    Content,
    Version,
}

impl Error {
    pub(super) fn answer(self, id: &Value) -> Response {
        let (code, message) = match self {
            Self::Parse => (-32700, "Invalid JSON payload"),
            Self::InvalidRequest => (-32600, "Invalid request"),
            Self::InvalidParams => (-32602, "Invalid parameters"),
            Self::TaskNotFound => (-32001, "Task not found"),
            Self::Push => (-32003, "Push notifications are unsupported"),
            Self::Unsupported => (-32004, "Operation unsupported"),
            Self::Content => (-32005, "Only text/plain is supported"),
            Self::Version => (-32009, "A2A version unsupported"),
        };
        Json(json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }))
            .into_response()
    }
}
