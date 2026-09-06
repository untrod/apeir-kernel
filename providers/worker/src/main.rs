use nous_kernel_core::{
    digest_bytes, digest_json, CapabilitySupport, KernelError, ModelCapabilityManifest,
    ModelPrivacy, OperationReceipt, OperationRequest, ProviderCommand, ProviderProbeReport,
    ProviderProbeRequest, ProviderResponse, ProviderRuntimeClass,
};
use nous_types::{ModelInvocationInput, ModelInvocationOutput};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::time::{Duration, Instant};

#[tokio::main]
async fn main() {
    let response = run().await.unwrap_or_else(|error| ProviderResponse {
        ok: false,
        receipt: None,
        probe: None,
        error_code: "PROVIDER_EXECUTION_FAILED".into(),
        error_message: error.to_string(),
    });
    let mut stdout = std::io::stdout().lock();
    if serde_json::to_writer(&mut stdout, &response).is_err() || stdout.flush().is_err() {
        std::process::exit(2);
    }
    if !response.ok {
        std::process::exit(1);
    }
}

async fn run() -> Result<ProviderResponse, KernelError> {
    if std::env::args().nth(1).as_deref() != Some("--stdio-once") {
        return Err(KernelError::Provider("expected --stdio-once".into()));
    }
    let mut body = Vec::new();
    std::io::stdin()
        .read_to_end(&mut body)
        .map_err(|error| KernelError::Provider(error.to_string()))?;
    let command: ProviderCommand = serde_json::from_slice(&body)
        .map_err(|error| KernelError::Serialization(error.to_string()))?;
    match command {
        ProviderCommand::Health => Ok(ProviderResponse {
            ok: true,
            receipt: None,
            probe: None,
            error_code: String::new(),
            error_message: String::new(),
        }),
        ProviderCommand::Probe { request } => Ok(ProviderResponse {
            ok: true,
            receipt: None,
            probe: Some(probe_backend(&request).await?),
            error_code: String::new(),
            error_message: String::new(),
        }),
        ProviderCommand::Execute { request } => {
            let result = execute_backend(&request).await?;
            let receipt = OperationReceipt {
                operation_id: request.operation_id.clone(),
                input_digest: request.input_digest(),
                output_digest: digest_bytes(result.as_bytes()),
                snapshot_digest: digest_json(&request.snapshot)?,
                provider_revision: request.snapshot.provider_revision.clone(),
                result,
                completed_at_us: chrono::Utc::now().timestamp_micros(),
            };
            Ok(ProviderResponse {
                ok: true,
                receipt: Some(receipt),
                probe: None,
                error_code: String::new(),
                error_message: String::new(),
            })
        }
    }
}

async fn execute_backend(request: &OperationRequest) -> Result<String, KernelError> {
    match request.backend.as_str() {
        "reference" => Ok(digest_bytes(request.input.as_bytes())),
        "reference-crash" => std::process::exit(70),
        "reference-delay" => {
            let delay_ms = request.model.parse::<u64>().unwrap_or(1_000).min(30_000);
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            Ok(digest_bytes(request.input.as_bytes()))
        }
        "reference-math" => reference_math(&request.input),
        "openai-compatible" | "edge-openai-compatible" => openai_compatible(request).await,
        "ollama" => ollama(request).await,
        other => Err(KernelError::Provider(format!(
            "unsupported backend: {other}"
        ))),
    }
}

fn reference_math(input: &str) -> Result<String, KernelError> {
    let request: Value = serde_json::from_str(input)
        .map_err(|error| KernelError::Provider(format!("invalid mathematical input: {error}")))?;
    let operation = request
        .get("operation")
        .and_then(Value::as_str)
        .ok_or_else(|| KernelError::Provider("mathematical operation is required".into()))?;
    let result = match operation {
        "linear" => {
            let coefficients = number_array(&request, "coefficients")?;
            let values = number_array(&request, "input")?;
            if coefficients.len() != values.len() {
                return Err(KernelError::Provider(
                    "coefficients and input must have equal length".into(),
                ));
            }
            let bias = request.get("bias").and_then(Value::as_f64).unwrap_or(0.0);
            coefficients
                .iter()
                .zip(values.iter())
                .fold(bias, |sum, (coefficient, value)| sum + coefficient * value)
        }
        "polynomial" => {
            let coefficients = number_array(&request, "coefficients")?;
            let value = request
                .get("input")
                .and_then(Value::as_f64)
                .ok_or_else(|| KernelError::Provider("numeric input is required".into()))?;
            coefficients
                .iter()
                .rev()
                .fold(0.0, |result, coefficient| result * value + coefficient)
        }
        other => {
            return Err(KernelError::Provider(format!(
                "unsupported mathematical operation: {other}"
            )))
        }
    };
    serde_json::to_string(&json!({"operation": operation, "result": result}))
        .map_err(|error| KernelError::Serialization(error.to_string()))
}

fn number_array(value: &Value, field: &str) -> Result<Vec<f64>, KernelError> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| KernelError::Provider(format!("numeric array '{field}' is required")))?
        .iter()
        .map(|item| {
            item.as_f64()
                .ok_or_else(|| KernelError::Provider(format!("'{field}' must be numeric")))
        })
        .collect()
}

async fn openai_compatible(request: &OperationRequest) -> Result<String, KernelError> {
    let allow_loopback_http = request.backend == "edge-openai-compatible";
    let invocation = decode_model_input(&request.input)?;
    let capability = invocation
        .as_ref()
        .map(|value| value.capability.as_str())
        .unwrap_or("chat");
    let path = if capability == "embedding" {
        "embeddings"
    } else {
        "chat/completions"
    };
    let url = checked_endpoint(
        &format!("{}/{path}", request.endpoint.trim_end_matches('/')),
        allow_loopback_http,
    )?;
    let response = reqwest::Client::builder()
        .timeout(Duration::from_millis(request.timeout_ms.max(1)))
        .build()
        .map_err(|error| KernelError::Provider(error.to_string()))?
        .post(url);
    let call = if request.credential_env.is_empty() && allow_loopback_http {
        response
    } else {
        response.bearer_auth(credential(request)?)
    };
    let body = openai_request_body(request, invocation.as_ref());
    let response = call
        .json(&body)
        .send()
        .await
        .map_err(|error| KernelError::Provider(error.to_string()))?;
    ensure_provider_http_success(response.status())?;
    let body: Value = response
        .json()
        .await
        .map_err(|error| KernelError::Provider(error.to_string()))?;
    if let Some(invocation) = invocation {
        return encode_openai_output(&body, &invocation);
    }
    body.pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| KernelError::Provider("response contains no message content".into()))
}

fn ensure_provider_http_success(status: reqwest::StatusCode) -> Result<(), KernelError> {
    if status.is_success() {
        return Ok(());
    }
    let code = if status == reqwest::StatusCode::PAYMENT_REQUIRED {
        "NOUS_PROVIDER_BALANCE_EXHAUSTED"
    } else {
        "NOUS_PROVIDER_HTTP_ERROR"
    };
    Err(KernelError::Provider(format!("{code}: HTTP {status}")))
}

async fn ollama(request: &OperationRequest) -> Result<String, KernelError> {
    let invocation = decode_model_input(&request.input)?;
    let path = if invocation.is_some() {
        "api/chat"
    } else {
        "api/generate"
    };
    let url = checked_endpoint(
        &format!("{}/{path}", request.endpoint.trim_end_matches('/')),
        true,
    )?;
    let body = invocation.as_ref().map_or_else(
        || json!({"model": request.model, "prompt": request.input, "stream": false}),
        |value| {
            let mut body = json!({
                "model": request.model,
                "messages": model_messages(value),
                "stream": false,
            });
            if let Some(temperature) = value.temperature {
                body["options"] = json!({"temperature": temperature});
            }
            body
        },
    );
    let response = reqwest::Client::builder()
        .timeout(Duration::from_millis(request.timeout_ms.max(1)))
        .build()
        .map_err(|error| KernelError::Provider(error.to_string()))?
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|error| KernelError::Provider(error.to_string()))?
        .error_for_status()
        .map_err(|error| KernelError::Provider(error.to_string()))?;
    let body: Value = response
        .json()
        .await
        .map_err(|error| KernelError::Provider(error.to_string()))?;
    if invocation.is_some() {
        let output = ModelInvocationOutput {
            schema_version: 1,
            content: body.pointer("/message/content").cloned().ok_or_else(|| {
                KernelError::Provider("response contains no message content".into())
            })?,
            usage: json!({
                "prompt_tokens": body.get("prompt_eval_count").cloned().unwrap_or(Value::Null),
                "completion_tokens": body.get("eval_count").cloned().unwrap_or(Value::Null),
            }),
            finish_reason: body
                .get("done_reason")
                .and_then(Value::as_str)
                .unwrap_or("completed")
                .into(),
            metadata: json!({"backend": "ollama"}),
        };
        return serde_json::to_string(&output)
            .map_err(|error| KernelError::Serialization(error.to_string()));
    }
    body.get("response")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| KernelError::Provider("response contains no generated text".into()))
}

fn decode_model_input(input: &str) -> Result<Option<ModelInvocationInput>, KernelError> {
    let Ok(value) = serde_json::from_str::<Value>(input) else {
        return Ok(None);
    };
    if value.get("schema_version").is_none()
        || (value.get("messages").is_none() && value.get("input").is_none())
    {
        return Ok(None);
    }
    let invocation: ModelInvocationInput = serde_json::from_value(value)
        .map_err(|error| KernelError::Provider(format!("invalid model invocation: {error}")))?;
    invocation
        .validate()
        .map_err(|error| KernelError::Provider(format!("invalid model invocation: {error}")))?;
    Ok(Some(invocation))
}

fn model_messages(invocation: &ModelInvocationInput) -> Vec<Value> {
    if !invocation.messages.is_empty() {
        return invocation.messages.clone();
    }
    vec![json!({"role": "user", "content": invocation.input})]
}

fn openai_request_body(
    request: &OperationRequest,
    invocation: Option<&ModelInvocationInput>,
) -> Value {
    let Some(invocation) = invocation else {
        return json!({
            "model": request.model,
            "messages": [{"role": "user", "content": request.input}],
        });
    };
    if invocation.capability == "embedding" {
        return json!({"model": request.model, "input": invocation.input});
    }
    let mut body = json!({
        "model": request.model,
        "messages": model_messages(invocation),
    });
    if !invocation.tools.is_empty() {
        body["tools"] = Value::Array(invocation.tools.clone());
    }
    if let Some(response_format) = &invocation.response_format {
        body["response_format"] = response_format.clone();
    }
    if let Some(max_tokens) = invocation.max_output_tokens {
        body["max_tokens"] = json!(max_tokens);
    }
    if let Some(temperature) = invocation.temperature {
        body["temperature"] = json!(temperature);
    }
    body
}

fn encode_openai_output(
    body: &Value,
    invocation: &ModelInvocationInput,
) -> Result<String, KernelError> {
    let (content, finish_reason, metadata) = if invocation.capability == "embedding" {
        (
            body.pointer("/data/0/embedding")
                .cloned()
                .ok_or_else(|| KernelError::Provider("response contains no embedding".into()))?,
            "completed".to_string(),
            json!({"backend": "openai-compatible"}),
        )
    } else {
        let message = body
            .pointer("/choices/0/message")
            .ok_or_else(|| KernelError::Provider("response contains no message".into()))?;
        (
            message.get("content").cloned().unwrap_or(Value::Null),
            body.pointer("/choices/0/finish_reason")
                .and_then(Value::as_str)
                .unwrap_or("completed")
                .to_string(),
            json!({
                "backend": "openai-compatible",
                "tool_calls": message.get("tool_calls").cloned().unwrap_or(Value::Null),
            }),
        )
    };
    serde_json::to_string(&ModelInvocationOutput {
        schema_version: 1,
        content,
        usage: body.get("usage").cloned().unwrap_or(Value::Null),
        finish_reason,
        metadata,
    })
    .map_err(|error| KernelError::Serialization(error.to_string()))
}

async fn probe_backend(request: &ProviderProbeRequest) -> Result<ProviderProbeReport, KernelError> {
    let started = Instant::now();
    let (runtime_class, available_models, mut warnings) = match request.backend.as_str() {
        "reference" | "reference-delay" | "reference-crash" | "reference-math" => (
            ProviderRuntimeClass::Reference,
            vec!["reference".into()],
            vec!["deterministic conformance backend; not a real model".into()],
        ),
        "openai-compatible" => {
            let credential = probe_credential(request)?;
            let url = checked_endpoint(
                &format!("{}/models", request.endpoint.trim_end_matches('/')),
                false,
            )?;
            let response = probe_client(request.timeout_ms)?
                .get(url)
                .bearer_auth(credential)
                .send()
                .await
                .map_err(|error| KernelError::Provider(error.to_string()))?
                .error_for_status()
                .map_err(|error| KernelError::Provider(error.to_string()))?;
            let body: Value = response
                .json()
                .await
                .map_err(|error| KernelError::Provider(error.to_string()))?;
            (
                ProviderRuntimeClass::Remote,
                body.get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect(),
                vec![],
            )
        }
        "ollama" => {
            let url = checked_endpoint(
                &format!("{}/api/tags", request.endpoint.trim_end_matches('/')),
                true,
            )?;
            let response = probe_client(request.timeout_ms)?
                .get(url)
                .send()
                .await
                .map_err(|error| KernelError::Provider(error.to_string()))?
                .error_for_status()
                .map_err(|error| KernelError::Provider(error.to_string()))?;
            let body: Value = response
                .json()
                .await
                .map_err(|error| KernelError::Provider(error.to_string()))?;
            (
                ProviderRuntimeClass::Local,
                body.get("models")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|model| {
                        model.get("name").and_then(Value::as_str).map(str::to_owned)
                    })
                    .collect(),
                vec![],
            )
        }
        "edge-openai-compatible" => {
            let url = checked_endpoint(
                &format!("{}/models", request.endpoint.trim_end_matches('/')),
                true,
            )?;
            let mut call = probe_client(request.timeout_ms)?.get(url);
            if !request.credential_env.is_empty() {
                call = call.bearer_auth(probe_credential(request)?);
            }
            let response = call
                .send()
                .await
                .map_err(|error| KernelError::Provider(error.to_string()))?
                .error_for_status()
                .map_err(|error| KernelError::Provider(error.to_string()))?;
            let body: Value = response
                .json()
                .await
                .map_err(|error| KernelError::Provider(error.to_string()))?;
            (
                ProviderRuntimeClass::Edge,
                body.get("data")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|model| model.get("id").and_then(Value::as_str).map(str::to_owned))
                    .collect(),
                vec!["capabilities not returned by the endpoint remain UNKNOWN".into()],
            )
        }
        other => {
            return Err(KernelError::Provider(format!(
                "unsupported backend: {other}"
            )))
        }
    };
    if !request.model.is_empty()
        && !available_models.is_empty()
        && !available_models.iter().any(|model| model == &request.model)
    {
        warnings.push(format!(
            "configured model '{}' was not reported by the provider",
            request.model
        ));
    }
    if request.execution_domain != ProviderRuntimeClass::Unknown
        && request.execution_domain != runtime_class
    {
        warnings.push(format!(
            "configured execution domain '{:?}' differs from probed domain '{runtime_class:?}'",
            request.execution_domain
        ));
    }
    let is_reference = runtime_class == ProviderRuntimeClass::Reference;
    let is_local = matches!(
        runtime_class,
        ProviderRuntimeClass::Local | ProviderRuntimeClass::Edge | ProviderRuntimeClass::Reference
    );
    Ok(ProviderProbeReport {
        reachable: true,
        latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        available_models,
        capabilities: ModelCapabilityManifest {
            schema_version: 1,
            provider: request.provider.clone(),
            backend: request.backend.clone(),
            model: request.model.clone(),
            runtime_class,
            chat: if is_reference {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unknown
            },
            streaming: if is_reference {
                CapabilitySupport::Unsupported
            } else {
                CapabilitySupport::Unknown
            },
            tools: CapabilitySupport::Unknown,
            structured_output: CapabilitySupport::Unknown,
            vision: CapabilitySupport::Unknown,
            embedding: CapabilitySupport::Unknown,
            reasoning: CapabilitySupport::Unknown,
            local_execution: if is_local {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            },
            remote_execution: if runtime_class == ProviderRuntimeClass::Remote {
                CapabilitySupport::Supported
            } else {
                CapabilitySupport::Unsupported
            },
            context_length: None,
            privacy_local: is_local,
            privacy: if is_local {
                ModelPrivacy::LocalOnly
            } else {
                ModelPrivacy::ProviderManaged
            },
            estimated_latency_ms: Some(
                u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            ),
            estimated_cost_microcents: None,
            probed_at_us: chrono::Utc::now().timestamp_micros(),
        },
        warnings,
    })
}

fn probe_client(timeout_ms: u64) -> Result<reqwest::Client, KernelError> {
    reqwest::Client::builder()
        .timeout(Duration::from_millis(timeout_ms.max(1)))
        .build()
        .map_err(|error| KernelError::Provider(error.to_string()))
}

fn probe_credential(request: &ProviderProbeRequest) -> Result<String, KernelError> {
    if request.credential_env.is_empty() {
        return Err(KernelError::Provider(
            "credential reference is required".into(),
        ));
    }
    std::env::var(&request.credential_env)
        .map_err(|_| KernelError::Provider("credential reference is unavailable".into()))
}

fn checked_endpoint(value: &str, allow_loopback_http: bool) -> Result<reqwest::Url, KernelError> {
    let url = reqwest::Url::parse(value)
        .map_err(|_| KernelError::Provider("provider endpoint is invalid".into()))?;
    if !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(KernelError::Provider(
            "provider endpoint must not contain credentials, query parameters, or fragments".into(),
        ));
    }
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "::1"));
    if url.scheme() != "https" && !(allow_loopback_http && loopback) {
        return Err(KernelError::Provider(
            "provider endpoint must use HTTPS (HTTP is allowed only on loopback)".into(),
        ));
    }
    Ok(url)
}

fn credential(request: &OperationRequest) -> Result<String, KernelError> {
    if request.credential_env.is_empty() {
        return Err(KernelError::Provider(
            "credential reference is required".into(),
        ));
    }
    std::env::var(&request.credential_env)
        .map_err(|_| KernelError::Provider("credential reference is unavailable".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model_request(input: String) -> OperationRequest {
        OperationRequest {
            operation_id: "operation".into(),
            workload_id: "workload".into(),
            step_id: "step".into(),
            backend: "openai-compatible".into(),
            execution_domain: ProviderRuntimeClass::Remote,
            model: "model".into(),
            endpoint: "https://example.invalid/v1".into(),
            credential_env: "TEST_API_KEY".into(),
            provider_entrypoint: String::new(),
            input,
            delivery: nous_kernel_core::DeliverySemantics::Idempotent,
            snapshot: nous_kernel_core::SemanticExecutionSnapshot {
                model_revision: "model".into(),
                provider_revision: "provider".into(),
                prompt_revision: "prompt".into(),
                tool_revision: "tool".into(),
                knowledge_revision: "knowledge".into(),
                policy_revision: "policy".into(),
                capability_revision: "capability".into(),
                context_revision: "context".into(),
            },
            timeout_ms: 1_000,
        }
    }

    #[test]
    fn mathematical_provider_is_deterministic() {
        let linear = reference_math(
            r#"{"operation":"linear","coefficients":[2.0,3.0],"input":[4.0,5.0],"bias":1.0}"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&linear).unwrap()["result"],
            24.0
        );

        let polynomial = reference_math(
            r#"{"operation":"polynomial","coefficients":[1.0,2.0,3.0],"input":2.0}"#,
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&polynomial).unwrap()["result"],
            17.0
        );
    }

    #[test]
    fn mathematical_provider_rejects_dimension_mismatch() {
        let error =
            reference_math(r#"{"operation":"linear","coefficients":[1.0],"input":[1.0,2.0]}"#)
                .unwrap_err();
        assert!(error.to_string().contains("equal length"));
    }

    #[test]
    fn openai_model_envelope_preserves_messages_tools_and_limits() {
        let input = serde_json::to_string(&ModelInvocationInput {
            schema_version: 1,
            capability: "chat".into(),
            messages: vec![json!({"role": "system", "content": "be exact"})],
            input: Value::Null,
            tools: vec![json!({"type": "function", "function": {"name": "probe"}})],
            response_format: Some(json!({"type": "json_object"})),
            max_output_tokens: Some(128),
            temperature: Some(0.25),
        })
        .unwrap();
        let request = model_request(input);
        let invocation = decode_model_input(&request.input).unwrap().unwrap();
        let body = openai_request_body(&request, Some(&invocation));
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["tools"][0]["function"]["name"], "probe");
        assert_eq!(body["max_tokens"], 128);
        assert_eq!(body["temperature"], 0.25);
    }

    #[test]
    fn openai_response_is_provider_neutral() {
        let invocation = ModelInvocationInput {
            schema_version: 1,
            capability: "chat".into(),
            messages: vec![json!({"role": "user", "content": "hello"})],
            input: Value::Null,
            tools: Vec::new(),
            response_format: None,
            max_output_tokens: None,
            temperature: None,
        };
        let encoded = encode_openai_output(
            &json!({
                "choices": [{"message": {"content": "world"}, "finish_reason": "stop"}],
                "usage": {"total_tokens": 2}
            }),
            &invocation,
        )
        .unwrap();
        let output: ModelInvocationOutput = serde_json::from_str(&encoded).unwrap();
        assert_eq!(output.content, "world");
        assert_eq!(output.usage["total_tokens"], 2);
    }

    #[test]
    fn payment_required_is_a_stable_balance_error() {
        let error =
            ensure_provider_http_success(reqwest::StatusCode::PAYMENT_REQUIRED).unwrap_err();
        assert_eq!(
            error.to_string(),
            "provider process error: NOUS_PROVIDER_BALANCE_EXHAUSTED: HTTP 402 Payment Required"
        );
    }
}
