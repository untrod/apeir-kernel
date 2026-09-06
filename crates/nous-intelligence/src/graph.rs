use crate::{invalid, require, schema, valid_identifier, BackendSpec, IntelligenceError, Validate};
use nous_types::{ContractValidation, ResourceVector, WorkflowContract};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeKind {
    Model,
    Algorithm,
    Preprocessor,
    Postprocessor,
    Router,
    Adapter,
    Verifier,
    Solver,
    Tool,
    Device,
    Human,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SideEffectDeclaration {
    None,
    FilesystemRead,
    FilesystemWrite,
    Network,
    DeviceRead,
    DeviceWrite,
    HumanApproval,
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphNode {
    pub id: String,
    pub kind: NodeKind,
    #[serde(default)]
    pub reference: String,
    pub capability: String,
    #[serde(default)]
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Value,
    #[serde(default)]
    pub resources: ResourceVector,
    #[serde(default)]
    pub parameters: BTreeMap<String, Value>,
    #[serde(default)]
    pub side_effects: BTreeSet<SideEffectDeclaration>,
    pub backend: BackendSpec,
}

impl Validate for GraphNode {
    fn validate(&self) -> Result<(), IntelligenceError> {
        if !valid_identifier(&self.id) {
            return Err(invalid(
                "GraphNode",
                "id",
                "must use letters, digits, '.', '-' or '_'",
            ));
        }
        if !self.capability.contains('.') {
            return Err(invalid(
                "GraphNode",
                "capability",
                "must be a namespaced capability identifier",
            ));
        }
        if matches!(
            self.kind,
            NodeKind::Model | NodeKind::Tool | NodeKind::Device
        ) {
            require("GraphNode", "reference", &self.reference)?;
        }
        self.backend.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphEdge {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IntelligenceGraph {
    pub schema_version: u32,
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub nodes: Vec<GraphNode>,
    #[serde(default)]
    pub edges: Vec<GraphEdge>,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

impl Validate for IntelligenceGraph {
    fn validate(&self) -> Result<(), IntelligenceError> {
        schema("IntelligenceGraph", self.schema_version)?;
        require("IntelligenceGraph", "id", &self.id)?;
        require("IntelligenceGraph", "version", &self.version)?;
        if self.nodes.is_empty() {
            return Err(invalid(
                "IntelligenceGraph",
                "nodes",
                "must contain at least one node",
            ));
        }

        let mut nodes = BTreeMap::new();
        for node in &self.nodes {
            node.validate()?;
            if nodes.insert(node.id.as_str(), node).is_some() {
                return Err(invalid(
                    "IntelligenceGraph",
                    "nodes",
                    format!("duplicate node '{}'", node.id),
                ));
            }
        }

        let mut adjacency: BTreeMap<&str, Vec<&str>> =
            nodes.keys().map(|id| (*id, Vec::new())).collect();
        let mut indegree: BTreeMap<&str, usize> = nodes.keys().map(|id| (*id, 0)).collect();
        let mut unique_edges = BTreeSet::new();
        for edge in &self.edges {
            if edge.from == edge.to {
                return Err(invalid(
                    "IntelligenceGraph",
                    "edges",
                    format!("self-cycle at '{}'", edge.from),
                ));
            }
            let from = nodes.get(edge.from.as_str()).ok_or_else(|| {
                invalid(
                    "IntelligenceGraph",
                    "edges",
                    format!("unknown source node '{}'", edge.from),
                )
            })?;
            let to = nodes.get(edge.to.as_str()).ok_or_else(|| {
                invalid(
                    "IntelligenceGraph",
                    "edges",
                    format!("unknown target node '{}'", edge.to),
                )
            })?;
            if !unique_edges.insert((&edge.from, &edge.to)) {
                return Err(invalid(
                    "IntelligenceGraph",
                    "edges",
                    format!("duplicate edge '{} -> {}'", edge.from, edge.to),
                ));
            }
            validate_schema_connection(&from.output_schema, &to.input_schema).map_err(
                |message| {
                    invalid(
                        "IntelligenceGraph",
                        "edges",
                        format!("{} -> {}: {message}", edge.from, edge.to),
                    )
                },
            )?;
            adjacency
                .get_mut(edge.from.as_str())
                .unwrap()
                .push(&edge.to);
            *indegree.get_mut(edge.to.as_str()).unwrap() += 1;
        }

        let mut queue: VecDeque<&str> = indegree
            .iter()
            .filter_map(|(id, count)| (*count == 0).then_some(*id))
            .collect();
        let mut visited = 0;
        while let Some(node) = queue.pop_front() {
            visited += 1;
            for target in &adjacency[node] {
                let count = indegree.get_mut(target).unwrap();
                *count -= 1;
                if *count == 0 {
                    queue.push_back(target);
                }
            }
        }
        if visited != nodes.len() {
            return Err(invalid(
                "IntelligenceGraph",
                "edges",
                "graph contains a cycle",
            ));
        }
        Ok(())
    }
}

fn validate_schema_connection(output: &Value, input: &Value) -> Result<(), String> {
    let output_type = output.get("type").and_then(Value::as_str);
    let input_type = input.get("type").and_then(Value::as_str);
    if let (Some(output_type), Some(input_type)) = (output_type, input_type) {
        if output_type != input_type {
            return Err(format!(
                "output type '{output_type}' does not match input type '{input_type}'"
            ));
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDiff {
    pub added_nodes: Vec<String>,
    pub removed_nodes: Vec<String>,
    pub changed_nodes: Vec<String>,
    pub added_edges: Vec<GraphEdge>,
    pub removed_edges: Vec<GraphEdge>,
}

impl IntelligenceGraph {
    /// Lower the portable graph topology into the existing Runtime Foundation
    /// workflow contract. Execution still goes through the runtime compiler,
    /// scheduler, and NKI; this method creates no alternate execution path.
    pub fn to_workflow_contract(&self) -> Result<WorkflowContract, IntelligenceError> {
        self.validate()?;
        let workflow = WorkflowContract {
            schema_version: self.schema_version,
            id: self.id.clone(),
            version: self.version.clone(),
            steps: self.nodes.iter().map(|node| node.id.clone()).collect(),
            edges: self
                .edges
                .iter()
                .map(|edge| [edge.from.clone(), edge.to.clone()])
                .collect(),
            metadata: BTreeMap::from([
                ("source".into(), "IntelligenceGraph".into()),
                ("source_schema".into(), self.schema_version.to_string()),
            ]),
        };
        workflow
            .validate()
            .map_err(|error| invalid("IntelligenceGraph", "workflow", error.to_string()))?;
        Ok(workflow)
    }

    pub fn diff(&self, other: &Self) -> GraphDiff {
        let left: BTreeMap<&str, &GraphNode> = self
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let right: BTreeMap<&str, &GraphNode> = other
            .nodes
            .iter()
            .map(|node| (node.id.as_str(), node))
            .collect();
        let mut added_nodes = right
            .keys()
            .filter(|id| !left.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>();
        let mut removed_nodes = left
            .keys()
            .filter(|id| !right.contains_key(**id))
            .map(|id| (*id).to_owned())
            .collect::<Vec<_>>();
        let mut changed_nodes = left
            .iter()
            .filter_map(|(id, node)| right.get(id).filter(|other| **other != *node).map(|_| *id))
            .map(str::to_owned)
            .collect::<Vec<_>>();
        added_nodes.sort();
        removed_nodes.sort();
        changed_nodes.sort();
        let left_edges: BTreeSet<_> = self.edges.iter().cloned().collect();
        let right_edges: BTreeSet<_> = other.edges.iter().cloned().collect();
        GraphDiff {
            added_nodes,
            removed_nodes,
            changed_nodes,
            added_edges: right_edges.difference(&left_edges).cloned().collect(),
            removed_edges: left_edges.difference(&right_edges).cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, input: &str, output: &str) -> GraphNode {
        GraphNode {
            id: id.into(),
            kind: NodeKind::Algorithm,
            reference: String::new(),
            capability: "math.evaluate".into(),
            input_schema: serde_json::json!({"type": input}),
            output_schema: serde_json::json!({"type": output}),
            resources: ResourceVector::default(),
            parameters: BTreeMap::new(),
            side_effects: BTreeSet::new(),
            backend: BackendSpec {
                kind: crate::BackendKind::Native,
                provider: String::new(),
                entrypoint: "reference-math".into(),
                configuration: BTreeMap::new(),
            },
        }
    }

    #[test]
    fn graph_rejects_cycles_and_schema_mismatches() {
        let mut graph = IntelligenceGraph {
            schema_version: 1,
            id: "math-pipeline".into(),
            version: "1.0.0".into(),
            nodes: vec![node("a", "object", "object"), node("b", "object", "object")],
            edges: vec![GraphEdge {
                from: "a".into(),
                to: "b".into(),
            }],
            metadata: BTreeMap::new(),
        };
        graph.validate().unwrap();
        graph.edges.push(GraphEdge {
            from: "b".into(),
            to: "a".into(),
        });
        assert!(graph.validate().unwrap_err().to_string().contains("cycle"));
        graph.edges.pop();
        graph.nodes[1].input_schema = serde_json::json!({"type": "array"});
        assert!(graph
            .validate()
            .unwrap_err()
            .to_string()
            .contains("does not match"));
    }

    #[test]
    fn graph_diff_is_stable() {
        let left = IntelligenceGraph {
            schema_version: 1,
            id: "g".into(),
            version: "1".into(),
            nodes: vec![node("a", "object", "object")],
            edges: vec![],
            metadata: BTreeMap::new(),
        };
        let mut right = left.clone();
        right.nodes.push(node("b", "object", "object"));
        assert_eq!(left.diff(&right).added_nodes, vec!["b"]);
    }

    #[test]
    fn graph_lowers_to_the_foundation_workflow_contract() {
        let graph = IntelligenceGraph {
            schema_version: 1,
            id: "g".into(),
            version: "1".into(),
            nodes: vec![node("a", "object", "object"), node("b", "object", "object")],
            edges: vec![GraphEdge {
                from: "a".into(),
                to: "b".into(),
            }],
            metadata: BTreeMap::new(),
        };
        let workflow = graph.to_workflow_contract().unwrap();
        assert_eq!(workflow.steps, vec!["a", "b"]);
        assert_eq!(workflow.edges, vec![[String::from("a"), String::from("b")]]);
    }
}
