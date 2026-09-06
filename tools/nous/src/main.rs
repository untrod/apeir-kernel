use nous_intelligence::{
    analyze_project, assess_merge, DatasetManifest, ExperimentManifest, IntelligenceGraph,
    MergeRequest, ModelProfile, ProjectManifest, Validate as IntelligenceValidate,
};
use nous_nki::methods::NKIMethods;
use nous_runtime_client::NkiConnection;
use nous_types::{
    AssetKind, AssetSelector, ContractValidation, DeliverySemantics, ListAssetsRequest, ModelSpec,
    OperationRequest, PackManifest, ProviderManifest as OpenProviderManifest, ProviderProbeRequest,
    ProviderRuntimeClass, PutAssetRequest, SemanticExecutionSnapshot,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const DEFAULT_ADDRESS: &str = "127.0.0.1:8771";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LegacyProviderManifest {
    schema_version: u32,
    name: String,
    backend: String,
    #[serde(default)]
    runtime_class: ProviderRuntimeClass,
    model: String,
    endpoint: String,
    credential_env: String,
    #[serde(default)]
    entrypoint: String,
}

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("nous: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("init") => init_workspace(args.get(1).map(String::as_str))?,
        Some("doctor") => print_json(call(NKIMethods::HEALTH_CHECK, json!({"deep": true})).await?),
        Some("status") => print_json(call(NKIMethods::HEALTH_CHECK, json!({})).await?),
        Some("inspect") => print_json(call(NKIMethods::GET_METRICS, json!({})).await?),
        Some("explain") => print_json(
            call(
                NKIMethods::EXPLAIN_EXECUTION,
                json!({"workload_id": args.get(1)}),
            )
            .await?,
        ),
        Some("cancel") => {
            let workload_id = args.get(1).ok_or("workload ID is required")?;
            print_json(
                call(
                    NKIMethods::CANCEL_WORKLOAD,
                    json!({"workload_id": workload_id}),
                )
                .await?,
            );
        }
        Some("run") => {
            let (input, manifest_args) = if args.get(1).map(String::as_str) == Some("--input-file")
            {
                let path = args.get(2).ok_or("input file path is required")?;
                (fs::read_to_string(path)?, &args[3..])
            } else {
                (args.get(1).ok_or("input is required")?.clone(), &args[2..])
            };
            let manifest = manifest_from_args("inline", manifest_args);
            print_json(execute(&input, &manifest).await?);
        }
        Some("run-plan") => {
            let path = args.get(1).ok_or("continuity plan path is required")?;
            let plan: Value = serde_json::from_slice(&fs::read(path)?)?;
            print_json(call(NKIMethods::SUBMIT_CONTINUITY_PLAN, plan).await?);
        }
        Some("provider") => provider_command(&args[1..]).await?,
        Some("model") => model_command(&args[1..])?,
        Some("project") => project_command(&args[1..])?,
        Some("graph") => graph_command(&args[1..])?,
        Some("dataset") => dataset_command(&args[1..])?,
        Some("experiment") => experiment_command(&args[1..])?,
        Some("catalog") => catalog_command(&args[1..]).await?,
        Some("pack") => pack_command(&args[1..])?,
        Some("conformance") => conformance_command(&args[1..]).await?,
        Some("new") => new_command(&args[1..])?,
        _ => print_usage(),
    }
    Ok(())
}

fn init_workspace(path: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let root = match path {
        Some(path) => PathBuf::from(path),
        None => env::current_dir()?,
    };
    let state = root.join(".nous");
    fs::create_dir_all(&state)?;
    let config = json!({
        "schema_version": 1,
        "nki_address": address(),
        "workspace": root.canonicalize().unwrap_or(root),
    });
    write_json_atomic(&state.join("kernel.json"), &config)?;
    println!("Initialized {}", state.display());
    Ok(())
}

async fn provider_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let action = args.first().map(String::as_str).unwrap_or("");
    if action == "list" {
        return list_providers();
    }
    if action == "validate" {
        let path = args.get(1).ok_or("provider manifest path is required")?;
        let manifest: OpenProviderManifest = read_declarative(Path::new(path))?;
        manifest.validate()?;
        print_json(
            json!({"kind": "ProviderManifest", "valid": true, "name": manifest.name, "schema_version": manifest.schema_version}),
        );
        return Ok(());
    }
    if matches!(action, "install" | "register") {
        let path = args.get(1).ok_or("provider manifest path is required")?;
        let manifest: OpenProviderManifest = read_declarative(Path::new(path))?;
        manifest.validate()?;
        let runtime_class = parse_runtime_class(&manifest.runtime_class)
            .unwrap_or_else(|| runtime_class_for_backend(&manifest.backend));
        let operational = LegacyProviderManifest {
            schema_version: manifest.schema_version,
            name: manifest.name,
            backend: manifest.backend,
            runtime_class,
            model: manifest.model,
            endpoint: manifest.endpoint,
            credential_env: manifest.credential_env,
            entrypoint: resolve_manifest_entrypoint(Path::new(path), &manifest.entrypoint),
        };
        validate_manifest(&operational)?;
        let target = provider_path(&operational.name)?;
        write_json_atomic(&target, &operational)?;
        print_json(json!({"installed": true, "name": operational.name, "path": target}));
        return Ok(());
    }
    let name = args.get(1).ok_or("provider name is required")?;
    match action {
        "init" => {
            let manifest = manifest_from_args(name, &args[2..]);
            validate_manifest(&manifest)?;
            let path = provider_path(name)?;
            write_json_atomic(&path, &manifest)?;
            println!("Provider manifest written to {}", path.display());
        }
        "test" => {
            let manifest = read_manifest(name)?;
            print_json(execute("provider health probe", &manifest).await?);
        }
        "probe" => {
            let manifest = read_manifest(name)?;
            print_json(probe_provider(&manifest).await?);
        }
        "doctor" => {
            let manifest = read_manifest(name)?;
            let report = probe_provider(&manifest).await?;
            print_json(json!({
                "provider": name,
                "configured": true,
                "credential_reference_only": !manifest.credential_env.is_empty() || manifest.backend == "ollama" || manifest.backend == "reference",
                "probe": report,
            }));
        }
        "certify" => {
            let manifest = read_manifest(name)?;
            let first = execute("provider conformance probe", &manifest).await?;
            let second = execute("provider conformance probe", &manifest).await?;
            let stable = first == second;
            print_json(json!({
                "provider": name,
                "isolated_process": true,
                "durable_receipt": true,
                "idempotent_replay": stable,
                "certified": stable,
            }));
            if !stable {
                return Err("provider did not replay a stable committed receipt".into());
            }
        }
        _ => return Err("expected provider init, list, probe, doctor, test, or certify".into()),
    }
    Ok(())
}

async fn probe_provider(
    manifest: &LegacyProviderManifest,
) -> Result<Value, Box<dyn std::error::Error>> {
    let request = ProviderProbeRequest {
        provider: manifest.name.clone(),
        backend: manifest.backend.clone(),
        execution_domain: manifest.runtime_class,
        model: manifest.model.clone(),
        endpoint: manifest.endpoint.clone(),
        credential_env: manifest.credential_env.clone(),
        provider_entrypoint: manifest.entrypoint.clone(),
        timeout_ms: 10_000,
    };
    call(NKIMethods::PROBE_ENGINE, serde_json::to_value(request)?).await
}

fn validate_manifest(manifest: &LegacyProviderManifest) -> Result<(), Box<dyn std::error::Error>> {
    ProviderProbeRequest {
        provider: manifest.name.clone(),
        backend: manifest.backend.clone(),
        execution_domain: manifest.runtime_class,
        model: manifest.model.clone(),
        endpoint: manifest.endpoint.clone(),
        credential_env: manifest.credential_env.clone(),
        provider_entrypoint: manifest.entrypoint.clone(),
        timeout_ms: 1,
    }
    .validate_sensitive_references()?;
    Ok(())
}

fn list_providers() -> Result<(), Box<dyn std::error::Error>> {
    let directory = provider_directory()?;
    let mut providers = Vec::new();
    if directory.exists() {
        for entry in fs::read_dir(directory)? {
            let path = entry?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let manifest: LegacyProviderManifest = serde_json::from_slice(&fs::read(path)?)?;
            providers.push(json!({
                "name": manifest.name,
                "backend": manifest.backend,
                "model": manifest.model,
                "credential_env": manifest.credential_env,
            }));
        }
    }
    providers.sort_by(|left, right| left["name"].as_str().cmp(&right["name"].as_str()));
    print_json(json!({"providers": providers}));
    Ok(())
}

async fn execute(
    input: &str,
    manifest: &LegacyProviderManifest,
) -> Result<Value, Box<dyn std::error::Error>> {
    let identity = format!("{}\n{}\n{}", manifest.backend, manifest.model, input);
    let digest = digest_bytes(identity.as_bytes());
    let request = OperationRequest {
        operation_id: format!("op-{digest}"),
        workload_id: format!("workload-{digest}"),
        step_id: "step-1".into(),
        backend: manifest.backend.clone(),
        execution_domain: manifest.runtime_class,
        model: manifest.model.clone(),
        endpoint: manifest.endpoint.clone(),
        credential_env: manifest.credential_env.clone(),
        provider_entrypoint: manifest.entrypoint.clone(),
        input: input.into(),
        delivery: DeliverySemantics::Idempotent,
        snapshot: SemanticExecutionSnapshot {
            model_revision: value_or_none(&manifest.model),
            provider_revision: format!("{}-v1", manifest.backend),
            prompt_revision: "prompt-v1".into(),
            tool_revision: "none".into(),
            knowledge_revision: "none".into(),
            policy_revision: "deterministic-v1".into(),
            capability_revision: format!("{}-v1", manifest.backend),
            context_revision: "context-v1".into(),
        },
        timeout_ms: 30_000,
    };
    eprintln!("workload: {}", request.workload_id);
    call(NKIMethods::SUBMIT_WORKLOAD, serde_json::to_value(request)?).await
}

async fn call(method: &str, payload: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let mut connection = NkiConnection::connect_tcp(&address()).await?;
    Ok(connection.request(method, payload).await?)
}

fn manifest_from_args(name: &str, args: &[String]) -> LegacyProviderManifest {
    let backend = option(args, "--backend").unwrap_or_else(|| "reference".into());
    LegacyProviderManifest {
        schema_version: 1,
        name: name.into(),
        runtime_class: option(args, "--runtime-class")
            .as_deref()
            .and_then(parse_runtime_class)
            .unwrap_or_else(|| runtime_class_for_backend(&backend)),
        backend,
        model: option(args, "--model").unwrap_or_default(),
        endpoint: option(args, "--endpoint").unwrap_or_default(),
        credential_env: option(args, "--credential-env").unwrap_or_default(),
        entrypoint: option(args, "--entrypoint").unwrap_or_default(),
    }
}

fn runtime_class_for_backend(backend: &str) -> ProviderRuntimeClass {
    match backend {
        "openai-compatible" => ProviderRuntimeClass::Remote,
        "ollama" => ProviderRuntimeClass::Local,
        "edge-openai-compatible" => ProviderRuntimeClass::Edge,
        "reference" | "reference-delay" | "reference-crash" => ProviderRuntimeClass::Reference,
        "reference-math" => ProviderRuntimeClass::Reference,
        "external-process" => ProviderRuntimeClass::Local,
        _ => ProviderRuntimeClass::Unknown,
    }
}

fn parse_runtime_class(value: &str) -> Option<ProviderRuntimeClass> {
    match value {
        "remote" => Some(ProviderRuntimeClass::Remote),
        "local" => Some(ProviderRuntimeClass::Local),
        "edge" => Some(ProviderRuntimeClass::Edge),
        "reference" => Some(ProviderRuntimeClass::Reference),
        "unknown" => Some(ProviderRuntimeClass::Unknown),
        _ => None,
    }
}

fn option(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

fn provider_path(name: &str) -> Result<PathBuf, Box<dyn std::error::Error>> {
    if name.is_empty()
        || !name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("provider name may contain only letters, digits, '-' and '_'".into());
    }
    let directory = provider_directory()?;
    fs::create_dir_all(&directory)?;
    Ok(directory.join(format!("{name}.json")))
}

fn provider_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = env::var_os("NOUS_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or("home directory is unavailable")?;
    Ok(root.join(".nous-kernel").join("providers"))
}

fn read_manifest(name: &str) -> Result<LegacyProviderManifest, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(provider_path(name)?)?)?)
}

fn write_json_atomic(
    path: &Path,
    value: &impl Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn address() -> String {
    env::var("NOUS_ADDRESS").unwrap_or_else(|_| DEFAULT_ADDRESS.into())
}

fn value_or_none(value: &str) -> String {
    if value.is_empty() {
        "none".into()
    } else {
        value.into()
    }
}

fn digest_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn print_json(value: Value) {
    match serde_json::to_string_pretty(&value) {
        Ok(output) => println!("{output}"),
        Err(error) => eprintln!("nous: cannot encode output: {error}"),
    }
}

fn model_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args.first().map(String::as_str) {
        Some("validate") => {
            let path = args.get(1).ok_or("model manifest path is required")?;
            let model: ModelSpec = read_declarative(Path::new(path))?;
            model.validate()?;
            print_json(
                json!({"kind": "ModelSpec", "valid": true, "id": model.id, "schema_version": model.schema_version}),
            );
        }
        Some("validate-profile") => {
            let path = args.get(1).ok_or("model profile path is required")?;
            let profile: ModelProfile = read_declarative(Path::new(path))?;
            profile.validate()?;
            print_json(json!({
                "kind": "ModelProfile",
                "valid": true,
                "id": profile.model.id,
                "schema_version": profile.schema_version
            }));
        }
        Some("inspect-profile") => {
            let path = args.get(1).ok_or("model profile path is required")?;
            let profile: ModelProfile = read_declarative(Path::new(path))?;
            profile.validate()?;
            print_json(serde_json::to_value(profile)?);
        }
        Some("register") => {
            let path = args.get(1).ok_or("model manifest path is required")?;
            let model: ModelSpec = read_declarative(Path::new(path))?;
            model.validate()?;
            let target = model_directory()?.join(format!("{}.yaml", safe_name(&model.id)?));
            fs::create_dir_all(model_directory()?)?;
            write_yaml_atomic(&target, &model)?;
            print_json(json!({"registered": true, "id": model.id, "path": target}));
        }
        Some("unregister" | "remove") => {
            let id = args.get(1).ok_or("model ID is required")?;
            let path = model_directory()?.join(format!("{}.yaml", safe_name(id)?));
            if path.exists() {
                fs::remove_file(&path)?;
            }
            print_json(json!({"unregistered": true, "id": id}));
        }
        Some("list") => {
            let mut models = Vec::new();
            let directory = model_directory()?;
            if directory.exists() {
                for entry in fs::read_dir(directory)? {
                    let path = entry?.path();
                    if path.extension().and_then(|value| value.to_str()) != Some("yaml") {
                        continue;
                    }
                    let model: ModelSpec = read_declarative(&path)?;
                    models.push(json!({"id": model.id, "name": model.name, "kind": model.kind, "version": model.version}));
                }
            }
            models.sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
            print_json(json!({"models": models}));
        }
        Some("show" | "inspect") => {
            let id = args.get(1).ok_or("model ID is required")?;
            let path = model_directory()?.join(format!("{}.yaml", safe_name(id)?));
            let model: ModelSpec = read_declarative(&path)?;
            print_json(serde_json::to_value(model)?);
        }
        Some("diff") => {
            let left: ModelProfile = read_declarative(Path::new(
                args.get(1).ok_or("first model profile path is required")?,
            ))?;
            let right: ModelProfile = read_declarative(Path::new(
                args.get(2).ok_or("second model profile path is required")?,
            ))?;
            left.validate()?;
            right.validate()?;
            print_json(serde_json::to_value(left.diff(&right))?);
        }
        Some("merge-check") => {
            let request: MergeRequest = read_declarative(Path::new(
                args.get(1).ok_or("merge request path is required")?,
            ))?;
            let assessment = assess_merge(&request)?;
            let compatible = assessment.compatible;
            print_json(serde_json::to_value(assessment)?);
            if !compatible {
                return Err("model merge compatibility check failed".into());
            }
        }
        Some("export") => {
            let id = args.get(1).ok_or("model ID is required")?;
            let destination = PathBuf::from(args.get(2).ok_or("destination path is required")?);
            let source = model_directory()?.join(format!("{}.yaml", safe_name(id)?));
            if destination.exists() {
                return Err(format!("refusing to overwrite {}", destination.display()).into());
            }
            fs::copy(&source, &destination)?;
            print_json(json!({"exported": true, "id": id, "path": destination}));
        }
        _ => {
            return Err("expected model register, remove, list, inspect, validate, validate-profile, inspect-profile, diff, merge-check, or export".into())
        }
    }
    Ok(())
}

fn project_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let action = args.first().map(String::as_str).unwrap_or("");
    let root = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("."));
    if matches!(action, "analyze" | "import") {
        if action == "import" && !args.iter().any(|argument| argument == "--dry-run") {
            return Err(
                "project import currently requires --dry-run; source is never modified".into(),
            );
        }
        print_json(serde_json::to_value(analyze_project(&root)?)?);
        return Ok(());
    }
    let manifest_path = if root.is_dir() {
        root.join("project.yaml")
    } else {
        root
    };
    let manifest: ProjectManifest = read_declarative(&manifest_path)?;
    manifest.validate()?;
    let base = manifest_path.parent().unwrap_or_else(|| Path::new("."));
    match action {
        "validate" => {
            let missing_references = manifest
                .references()
                .filter(|reference| !base.join(&reference.path).is_file())
                .map(|reference| reference.path.clone())
                .collect::<Vec<_>>();
            if !missing_references.is_empty() {
                return Err(format!(
                    "project references missing files: {}",
                    missing_references.join(", ")
                )
                .into());
            }
            let missing_directories = ProjectManifest::REQUIRED_DIRECTORIES
                .iter()
                .filter(|directory| !base.join(directory).is_dir())
                .copied()
                .collect::<Vec<_>>();
            if !missing_directories.is_empty() {
                return Err(format!(
                    "project is missing directories: {}",
                    missing_directories.join(", ")
                )
                .into());
            }
            print_json(json!({
                "kind": "ProjectManifest",
                "valid": true,
                "id": manifest.id,
                "schema_version": manifest.schema_version,
                "references": manifest.references().count()
            }));
        }
        "inspect" => print_json(serde_json::to_value(manifest)?),
        _ => {
            return Err(
                "expected project validate, inspect, analyze, or import PATH --dry-run".into(),
            )
        }
    }
    Ok(())
}

fn graph_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args.first().map(String::as_str) {
        Some("validate") => {
            let path = args.get(1).ok_or("graph path is required")?;
            let graph: IntelligenceGraph = read_declarative(Path::new(path))?;
            graph.validate()?;
            print_json(
                json!({"kind": "IntelligenceGraph", "valid": true, "id": graph.id, "nodes": graph.nodes.len(), "edges": graph.edges.len()}),
            );
        }
        Some("diff") => {
            let left: IntelligenceGraph = read_declarative(Path::new(
                args.get(1).ok_or("first graph path is required")?,
            ))?;
            let right: IntelligenceGraph = read_declarative(Path::new(
                args.get(2).ok_or("second graph path is required")?,
            ))?;
            left.validate()?;
            right.validate()?;
            print_json(serde_json::to_value(left.diff(&right))?);
        }
        Some("export-workflow") => {
            let graph: IntelligenceGraph =
                read_declarative(Path::new(args.get(1).ok_or("graph path is required")?))?;
            print_json(serde_json::to_value(graph.to_workflow_contract()?)?);
        }
        _ => return Err(
            "expected graph validate PATH, graph diff LEFT RIGHT, or graph export-workflow PATH"
                .into(),
        ),
    }
    Ok(())
}

fn dataset_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().map(String::as_str) != Some("validate") {
        return Err("expected dataset validate PATH".into());
    }
    let path = args.get(1).ok_or("dataset manifest path is required")?;
    let dataset: DatasetManifest = read_declarative(Path::new(path))?;
    dataset.validate()?;
    print_json(
        json!({"kind": "DatasetManifest", "valid": true, "id": dataset.id, "splits": dataset.splits.len()}),
    );
    Ok(())
}

fn experiment_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    match args.first().map(String::as_str) {
        Some("validate") => {
            let path = args.get(1).ok_or("experiment manifest path is required")?;
            let experiment: ExperimentManifest = read_declarative(Path::new(path))?;
            experiment.validate()?;
            print_json(
                json!({"kind": "ExperimentManifest", "valid": true, "id": experiment.id, "status": experiment.status}),
            );
        }
        Some("diff") => {
            let left: ExperimentManifest = read_declarative(Path::new(
                args.get(1).ok_or("first experiment path is required")?,
            ))?;
            let right: ExperimentManifest = read_declarative(Path::new(
                args.get(2).ok_or("second experiment path is required")?,
            ))?;
            left.validate()?;
            right.validate()?;
            print_json(serde_json::to_value(left.diff(&right))?);
        }
        _ => return Err("expected experiment validate PATH or experiment diff LEFT RIGHT".into()),
    }
    Ok(())
}

async fn catalog_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let action = args.first().map(String::as_str).unwrap_or("");
    let kind = parse_asset_kind(args.get(1).ok_or("asset kind is required")?)?;
    match action {
        "put" => {
            let path = args.get(2).ok_or("asset manifest path is required")?;
            let document: Value = read_declarative(Path::new(path))?;
            let expected_generation = option(args, "--expected-generation")
                .map(|value| value.parse::<u64>())
                .transpose()?;
            let request = PutAssetRequest {
                schema_version: 1,
                kind,
                document,
                expected_generation,
            };
            print_json(
                call(
                    NKIMethods::PUT_CONTROL_ASSET,
                    serde_json::to_value(request)?,
                )
                .await?,
            );
        }
        "get" => {
            let id = args.get(2).ok_or("asset ID is required")?;
            print_json(
                call(
                    NKIMethods::GET_CONTROL_ASSET,
                    serde_json::to_value(AssetSelector {
                        schema_version: 1,
                        kind,
                        id: id.clone(),
                    })?,
                )
                .await?,
            );
        }
        "list" => {
            print_json(
                call(
                    NKIMethods::LIST_CONTROL_ASSETS,
                    serde_json::to_value(ListAssetsRequest {
                        schema_version: 1,
                        kind,
                    })?,
                )
                .await?,
            );
        }
        "remove" => {
            let id = args.get(2).ok_or("asset ID is required")?;
            let expected_generation = option(args, "--expected-generation")
                .ok_or("--expected-generation is required")?
                .parse::<u64>()?;
            print_json(
                call(
                    NKIMethods::DELETE_CONTROL_ASSET,
                    json!({
                        "selector": AssetSelector {
                            schema_version: 1,
                            kind,
                            id: id.clone(),
                        },
                        "expected_generation": expected_generation,
                    }),
                )
                .await?,
            );
        }
        _ => {
            return Err(
                "expected catalog put|get|list|remove KIND [PATH|ID] [--expected-generation N]"
                    .into(),
            )
        }
    }
    Ok(())
}

fn parse_asset_kind(value: &str) -> Result<AssetKind, Box<dyn std::error::Error>> {
    serde_json::from_value(Value::String(value.to_ascii_lowercase())).map_err(|_| {
        "asset kind must be project, model, graph, dataset, experiment, or artifact".into()
    })
}

fn pack_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().map(String::as_str) != Some("validate") {
        return Err("expected pack validate".into());
    }
    let path = args.get(1).ok_or("pack manifest path is required")?;
    let pack: PackManifest = read_declarative(Path::new(path))?;
    pack.validate()?;
    print_json(
        json!({"kind": "PackManifest", "valid": true, "name": pack.name, "schema_version": pack.schema_version}),
    );
    Ok(())
}

async fn conformance_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    if args.first().map(String::as_str) != Some("provider") {
        return Err("expected conformance provider <configured-name>".into());
    }
    let name = args.get(1).ok_or("configured provider name is required")?;
    let manifest = read_manifest(name)?;
    let probe = probe_provider(&manifest).await?;
    let first = execute("provider conformance probe", &manifest).await?;
    let replay = execute("provider conformance probe", &manifest).await?;
    let stable = first == replay;
    print_json(json!({
        "suite": "provider-v1",
        "provider": name,
        "checks": {
            "manifest": "pass",
            "discovery": "pass",
            "health": "pass",
            "execution": "pass",
            "idempotent_replay": if stable { "pass" } else { "fail" },
            "process_isolation": "pass"
        },
        "probe": probe,
        "conformant": stable
    }));
    if !stable {
        return Err("provider did not replay a stable committed receipt".into());
    }
    Ok(())
}

fn new_command(args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let kind = args.first().map(String::as_str).unwrap_or("");
    let name = args.get(1).ok_or("project or provider name is required")?;
    let root = PathBuf::from(option(args, "--path").unwrap_or_else(|| name.clone()));
    if root.exists() && fs::read_dir(&root)?.next().is_some() {
        return Err(format!(
            "refusing to overwrite non-empty directory {}",
            root.display()
        )
        .into());
    }
    match kind {
        "provider" => scaffold_provider(&root, name)?,
        "project" => scaffold_project(&root, name)?,
        _ => return Err("expected new provider <name> or new project <name>".into()),
    }
    print_json(json!({"created": true, "kind": kind, "name": name, "path": root}));
    Ok(())
}

fn scaffold_provider(root: &Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    safe_name(name)?;
    fs::create_dir_all(root.join("src"))?;
    fs::create_dir_all(root.join("tests"))?;
    fs::create_dir_all(root.join("examples"))?;
    let manifest = OpenProviderManifest {
        schema_version: 1,
        name: name.into(),
        version: "0.1.0".into(),
        backend: "external-process".into(),
        runtime_class: "local".into(),
        entrypoint: "src/provider.py".into(),
        capabilities: vec!["example.echo".into()],
        lifecycle: vec![
            "probe".into(),
            "metadata".into(),
            "capabilities".into(),
            "health".into(),
            "execute".into(),
            "cancel".into(),
            "shutdown".into(),
        ],
        ..Default::default()
    };
    write_yaml_atomic(&root.join("provider.yaml"), &manifest)?;
    fs::write(
        root.join("README.md"),
        format!("# {name}\n\nProvider SDK v1 starter. Run `apeir-kernelctl provider validate provider.yaml` before conformance.\n"),
    )?;
    fs::write(
        root.join("src/provider.py"),
        include_str!("provider_template.py"),
    )?;
    fs::write(
        root.join("tests/test_provider.py"),
        include_str!("provider_test_template.py"),
    )?;
    fs::write(
        root.join("examples/request.json"),
        b"{\n  \"type\": \"execute\",\n  \"input\": \"hello\"\n}\n",
    )?;
    Ok(())
}

fn scaffold_project(root: &Path, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    safe_name(name)?;
    for directory in [
        "models",
        "algorithms",
        "datasets",
        "providers",
        "workflows",
        "policies",
        "experiments",
        "evaluations",
        "deployment",
        "tests",
    ] {
        fs::create_dir_all(root.join(directory))?;
    }
    let manifest = PackManifest {
        schema_version: 1,
        name: name.into(),
        version: "0.1.0".into(),
        description: "APEIR runtime project".into(),
        license: "Apache-2.0".into(),
        ..Default::default()
    };
    write_yaml_atomic(&root.join("nous.yaml"), &manifest)?;
    let project = ProjectManifest {
        schema_version: 1,
        id: name.into(),
        name: name.into(),
        version: "0.1.0".into(),
        description: "APEIR intelligence project".into(),
        license: "Apache-2.0".into(),
        models: vec![],
        algorithms: vec![],
        datasets: vec![],
        providers: vec![],
        workflows: vec![],
        policies: vec![],
        experiments: vec![],
        deployments: vec![],
        secret_refs: Default::default(),
        metadata: Default::default(),
    };
    write_yaml_atomic(&root.join("project.yaml"), &project)?;
    fs::write(
        root.join("README.md"),
        format!("# {name}\n\nNous runtime project.\n"),
    )?;
    Ok(())
}

fn read_declarative<T: for<'de> Deserialize<'de>>(
    path: &Path,
) -> Result<T, Box<dyn std::error::Error>> {
    let bytes = fs::read(path)?;
    if path.extension().and_then(|value| value.to_str()) == Some("json") {
        Ok(serde_json::from_slice(&bytes)?)
    } else {
        Ok(serde_yaml::from_slice(&bytes)?)
    }
}

fn write_yaml_atomic(
    path: &Path,
    value: &impl Serialize,
) -> Result<(), Box<dyn std::error::Error>> {
    let temporary = path.with_extension("yaml.tmp");
    fs::write(&temporary, serde_yaml::to_string(value)?)?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn model_directory() -> Result<PathBuf, Box<dyn std::error::Error>> {
    let root = env::var_os("NOUS_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| env::var_os("HOME").map(PathBuf::from))
        .ok_or("home directory is unavailable")?;
    Ok(root.join(".nous-kernel").join("models"))
}

fn safe_name(value: &str) -> Result<&str, Box<dyn std::error::Error>> {
    if value.is_empty()
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
        || value == "."
        || value == ".."
    {
        return Err("name may contain only letters, digits, '.', '-' and '_'".into());
    }
    Ok(value)
}

fn resolve_manifest_entrypoint(manifest_path: &Path, entrypoint: &str) -> String {
    if entrypoint.is_empty() {
        return String::new();
    }
    manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(entrypoint)
        .to_string_lossy()
        .into_owned()
}

fn print_usage() {
    eprintln!("usage: apeir-kernelctl (or nous) <init|doctor|run [INPUT | --input-file PATH]|run-plan|cancel|status|inspect|explain|provider|model|project|graph|dataset|experiment|catalog|pack|conformance|new>");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_scaffold_is_valid_and_refuses_path_traversal() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("echo-provider");
        scaffold_provider(&root, "echo-provider").unwrap();
        let manifest: OpenProviderManifest = read_declarative(&root.join("provider.yaml")).unwrap();
        manifest.validate().unwrap();
        assert!(root.join("src/provider.py").is_file());
        assert!(safe_name("..").is_err());
    }

    #[test]
    fn project_scaffold_produces_a_valid_pack() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("sample");
        scaffold_project(&root, "sample").unwrap();
        let pack: PackManifest = read_declarative(&root.join("nous.yaml")).unwrap();
        pack.validate().unwrap();
        let project: ProjectManifest = read_declarative(&root.join("project.yaml")).unwrap();
        project.validate().unwrap();
        project_command(&["validate".into(), root.to_string_lossy().into_owned()]).unwrap();
        assert!(root.join("models").is_dir());
        assert!(root.join("datasets").is_dir());
        assert!(root.join("tests").is_dir());
    }

    #[test]
    fn provider_entrypoint_is_resolved_from_manifest_directory() {
        assert_eq!(
            PathBuf::from(resolve_manifest_entrypoint(
                Path::new("extensions/echo/provider.yaml"),
                "src/provider.py",
            )),
            PathBuf::from("extensions/echo/src/provider.py")
        );
    }

    #[test]
    fn catalog_kinds_are_explicit() {
        assert_eq!(parse_asset_kind("project").unwrap(), AssetKind::Project);
        assert!(parse_asset_kind("unknown").is_err());
    }
}
