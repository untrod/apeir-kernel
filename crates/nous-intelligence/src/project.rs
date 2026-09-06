use crate::{invalid, require, schema, valid_identifier, IntelligenceError, Validate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Component, Path};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectReference {
    pub id: String,
    pub path: String,
}

impl Validate for ProjectReference {
    fn validate(&self) -> Result<(), IntelligenceError> {
        require("ProjectReference", "id", &self.id)?;
        require("ProjectReference", "path", &self.path)?;
        let path = Path::new(&self.path);
        if path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
        {
            return Err(invalid(
                "ProjectReference",
                "path",
                "must be a portable project-relative path without parent traversal",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,
    #[serde(default)]
    pub models: Vec<ProjectReference>,
    #[serde(default)]
    pub algorithms: Vec<ProjectReference>,
    #[serde(default)]
    pub datasets: Vec<ProjectReference>,
    #[serde(default)]
    pub providers: Vec<ProjectReference>,
    #[serde(default)]
    pub workflows: Vec<ProjectReference>,
    #[serde(default)]
    pub policies: Vec<ProjectReference>,
    #[serde(default)]
    pub experiments: Vec<ProjectReference>,
    #[serde(default)]
    pub deployments: Vec<ProjectReference>,
    #[serde(default)]
    pub secret_refs: BTreeMap<String, String>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl ProjectManifest {
    pub const REQUIRED_DIRECTORIES: [&'static str; 10] = [
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
    ];

    pub fn references(&self) -> impl Iterator<Item = &ProjectReference> {
        self.models
            .iter()
            .chain(&self.algorithms)
            .chain(&self.datasets)
            .chain(&self.providers)
            .chain(&self.workflows)
            .chain(&self.policies)
            .chain(&self.experiments)
            .chain(&self.deployments)
    }
}

impl Validate for ProjectManifest {
    fn validate(&self) -> Result<(), IntelligenceError> {
        schema("ProjectManifest", self.schema_version)?;
        if !valid_identifier(&self.id) {
            return Err(invalid(
                "ProjectManifest",
                "id",
                "must use letters, digits, '.', '-' or '_'",
            ));
        }
        require("ProjectManifest", "name", &self.name)?;
        require("ProjectManifest", "version", &self.version)?;
        require("ProjectManifest", "license", &self.license)?;
        for reference in self.references() {
            reference.validate()?;
        }
        for (name, reference) in &self.secret_refs {
            if !valid_identifier(name)
                || reference.is_empty()
                || !reference.chars().all(|character| {
                    character == '_' || character.is_ascii_uppercase() || character.is_ascii_digit()
                })
                || !reference
                    .chars()
                    .next()
                    .is_some_and(|character| character == '_' || character.is_ascii_uppercase())
            {
                return Err(invalid(
                    "ProjectManifest",
                    "secret_refs",
                    "values must be environment-variable names; secret values are forbidden",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationProposal {
    pub adapter_first: bool,
    pub provider_manifest: bool,
    pub model_spec: bool,
    pub pack_manifest: bool,
    pub workflow_template: bool,
    pub user_review_required: bool,
    pub recommendations: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectAnalysis {
    pub root: String,
    pub files_scanned: usize,
    pub languages: BTreeSet<String>,
    pub dependency_manifests: Vec<String>,
    pub entrypoints: Vec<String>,
    pub model_assets: Vec<String>,
    pub container_files: Vec<String>,
    pub test_files: Vec<String>,
    pub license_files: Vec<String>,
    pub frameworks: BTreeSet<String>,
    pub proposal: MigrationProposal,
    pub warnings: Vec<String>,
}

/// Analyze a local project without following symlinks or modifying source.
///
/// The scan is intentionally bounded. It records metadata only and never opens
/// model weight files. Network repository cloning belongs to a separate,
/// explicitly authorized acquisition step.
pub fn analyze_project(root: &Path) -> io::Result<ProjectAnalysis> {
    const MAX_FILES: usize = 20_000;
    const MAX_DEPTH: usize = 24;
    const MAX_MANIFEST_BYTES: u64 = 1_048_576;
    const SKIPPED: [&str; 9] = [
        ".git",
        ".nous",
        ".venv",
        "node_modules",
        "target",
        "dist",
        "build",
        "__pycache__",
        ".pytest_cache",
    ];

    let root = root.canonicalize()?;
    if !root.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "project root must be a directory",
        ));
    }
    let mut analysis = ProjectAnalysis {
        root: root.to_string_lossy().into_owned(),
        files_scanned: 0,
        languages: BTreeSet::new(),
        dependency_manifests: vec![],
        entrypoints: vec![],
        model_assets: vec![],
        container_files: vec![],
        test_files: vec![],
        license_files: vec![],
        frameworks: BTreeSet::new(),
        proposal: MigrationProposal {
            adapter_first: true,
            provider_manifest: false,
            model_spec: false,
            pack_manifest: true,
            workflow_template: false,
            user_review_required: true,
            recommendations: vec![],
        },
        warnings: vec![],
    };
    let mut pending = vec![(root.clone(), 0usize)];
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_DEPTH {
            analysis.warnings.push(format!(
                "depth limit reached at {}",
                display_relative(&root, &directory)
            ));
            continue;
        }
        for entry in fs::read_dir(&directory)? {
            if analysis.files_scanned >= MAX_FILES {
                analysis
                    .warnings
                    .push(format!("file limit reached ({MAX_FILES})"));
                pending.clear();
                break;
            }
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if metadata.file_type().is_symlink() {
                analysis.warnings.push(format!(
                    "symlink not followed: {}",
                    display_relative(&root, &path)
                ));
                continue;
            }
            if metadata.is_dir() {
                let name = entry.file_name();
                if !SKIPPED.iter().any(|item| name == *item) {
                    pending.push((path, depth + 1));
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            analysis.files_scanned += 1;
            classify_file(
                &root,
                &path,
                metadata.len(),
                MAX_MANIFEST_BYTES,
                &mut analysis,
            )?;
        }
    }
    for collection in [
        &mut analysis.dependency_manifests,
        &mut analysis.entrypoints,
        &mut analysis.model_assets,
        &mut analysis.container_files,
        &mut analysis.test_files,
        &mut analysis.license_files,
        &mut analysis.warnings,
    ] {
        collection.sort();
        collection.dedup();
    }
    analysis.proposal.provider_manifest = !analysis.entrypoints.is_empty();
    analysis.proposal.model_spec = !analysis.model_assets.is_empty();
    analysis.proposal.workflow_template = analysis.entrypoints.len() > 1
        || (!analysis.entrypoints.is_empty() && !analysis.model_assets.is_empty());
    if analysis.proposal.provider_manifest {
        analysis
            .proposal
            .recommendations
            .push("wrap an existing entrypoint with a ProviderManifest adapter".into());
    }
    if analysis.proposal.model_spec {
        analysis.proposal.recommendations.push(
            "inspect model licenses and generate ModelSpec references without copying weights"
                .into(),
        );
    }
    if analysis.license_files.is_empty() {
        analysis.warnings.push(
            "no top-level license file was detected; manual license review is required".into(),
        );
    }
    analysis.proposal.recommendations.sort();
    analysis.warnings.sort();
    Ok(analysis)
}

fn classify_file(
    root: &Path,
    path: &Path,
    size: u64,
    max_manifest_bytes: u64,
    analysis: &mut ProjectAnalysis,
) -> io::Result<()> {
    let relative = display_relative(root, path);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    if let Some(language) = match extension.as_str() {
        "py" => Some("Python"),
        "rs" => Some("Rust"),
        "c" | "h" => Some("C"),
        "cc" | "cpp" | "cxx" | "hpp" => Some("C++"),
        "js" | "jsx" => Some("JavaScript"),
        "ts" | "tsx" => Some("TypeScript"),
        "go" => Some("Go"),
        "java" => Some("Java"),
        _ => None,
    } {
        analysis.languages.insert(language.into());
    }
    let dependency_manifest = matches!(
        name.as_str(),
        "pyproject.toml"
            | "requirements.txt"
            | "setup.py"
            | "cargo.toml"
            | "package.json"
            | "package-lock.json"
            | "poetry.lock"
            | "uv.lock"
            | "cmakelists.txt"
            | "go.mod"
    );
    if dependency_manifest {
        analysis.dependency_manifests.push(relative.clone());
        if size <= max_manifest_bytes {
            let content = fs::read_to_string(path)
                .unwrap_or_default()
                .to_ascii_lowercase();
            for (needle, framework) in [
                ("torch", "PyTorch"),
                ("transformers", "Transformers"),
                ("tensorflow", "TensorFlow"),
                ("onnxruntime", "ONNX Runtime"),
                ("numpy", "NumPy"),
                ("scipy", "SciPy"),
                ("fastapi", "FastAPI"),
                ("flask", "Flask"),
                ("tokio", "Tokio"),
                ("serde", "Serde"),
            ] {
                if content.contains(needle) {
                    analysis.frameworks.insert(framework.into());
                }
            }
        }
    }
    if matches!(
        name.as_str(),
        "main.py" | "app.py" | "cli.py" | "main.rs" | "lib.rs" | "index.js" | "index.ts"
    ) {
        analysis.entrypoints.push(relative.clone());
    }
    if matches!(
        extension.as_str(),
        "onnx" | "safetensors" | "gguf" | "pt" | "pth" | "ckpt"
    ) {
        analysis.model_assets.push(relative.clone());
    }
    if name == "dockerfile" || name.starts_with("docker-compose") {
        analysis.container_files.push(relative.clone());
    }
    if name.starts_with("test_")
        || name.ends_with("_test.rs")
        || path.components().any(|part| part.as_os_str() == "tests")
    {
        analysis.test_files.push(relative.clone());
    }
    if name == "license" || name.starts_with("license.") || name.starts_with("copying") {
        analysis.license_files.push(relative);
    }
    Ok(())
}

fn display_relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> ProjectManifest {
        ProjectManifest {
            schema_version: 1,
            id: "sample".into(),
            name: "Sample".into(),
            version: "0.1.0".into(),
            description: String::new(),
            license: "Apache-2.0".into(),
            models: vec![ProjectReference {
                id: "linear".into(),
                path: "models/linear.yaml".into(),
            }],
            algorithms: vec![],
            datasets: vec![],
            providers: vec![],
            workflows: vec![],
            policies: vec![],
            experiments: vec![],
            deployments: vec![],
            secret_refs: BTreeMap::from([("remote".into(), "MODEL_API_KEY".into())]),
            metadata: BTreeMap::new(),
        }
    }

    #[test]
    fn project_references_are_portable_and_secrets_are_references() {
        let mut project = manifest();
        project.validate().unwrap();
        project.models[0].path = "../model.yaml".into();
        assert!(project.validate().is_err());
        project.models[0].path = "models/model.yaml".into();
        project.secret_refs.insert("bad".into(), "sk-value".into());
        assert!(project.validate().is_err());
    }

    #[test]
    fn importer_analyzes_without_following_generated_directories() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(
            temporary.path().join("pyproject.toml"),
            "dependencies = ['torch', 'numpy']",
        )
        .unwrap();
        fs::write(temporary.path().join("main.py"), "print('reference')").unwrap();
        fs::write(temporary.path().join("LICENSE"), "Apache-2.0").unwrap();
        fs::create_dir(temporary.path().join("models")).unwrap();
        fs::write(temporary.path().join("models/model.onnx"), b"metadata").unwrap();
        fs::create_dir(temporary.path().join("target")).unwrap();
        fs::write(temporary.path().join("target/ignored.rs"), "").unwrap();

        let analysis = analyze_project(temporary.path()).unwrap();
        assert!(analysis.languages.contains("Python"));
        assert!(analysis.frameworks.contains("PyTorch"));
        assert_eq!(analysis.model_assets, vec!["models/model.onnx"]);
        assert!(analysis.proposal.provider_manifest);
        assert!(analysis.proposal.model_spec);
        assert_eq!(analysis.files_scanned, 4);
    }
}
