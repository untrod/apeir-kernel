//! Scheduler integration tests.
//!
//! Verify that the scheduler correctly:
//! 1. Filters candidates by resource constraints
//! 2. Rejects workloads that cannot fit
//! 3. Respects deadline requirements
//! 4. Produces deterministic results for identical inputs
//! 5. Handles empty candidate sets
//! 6. All 10 policies produce valid results

#[cfg(test)]
mod scheduler_integration_tests {
    use crate::placement::*;
    use crate::policy::*;
    use nous_types::resource::ResourceVector;
    use nous_types::workload::{
        CapabilityRequirements, ModelRequirements, RoutingPreference, WorkloadSpec,
    };

    fn make_test_node(
        id: &str,
        device_type: &str,
        free_vram: u64,
        free_ram: u64,
        engine_tps: f64,
        kv_hit: f64,
        loaded_models: Vec<&str>,
    ) -> NodeInfo {
        NodeInfo {
            node_id: id.to_string(),
            hostname: format!("{}.local", id),
            arch: "x86_64".into(),
            os: "linux".into(),
            capacity: ResourceVector {
                device_memory_bytes: 16 * 1024 * 1024 * 1024,
                ram_bytes: 32 * 1024 * 1024 * 1024,
                kv_cache_bytes: 4 * 1024 * 1024 * 1024,
                cpu_cores_millis: 8000,
                ..Default::default()
            },
            allocatable: ResourceVector {
                device_memory_bytes: free_vram,
                ram_bytes: free_ram,
                kv_cache_bytes: 2 * 1024 * 1024 * 1024,
                cpu_cores_millis: 4000,
                ..Default::default()
            },
            devices: vec![DevicePlacementInfo {
                device_id: format!("{}-dev0", id),
                device_type: device_type.into(),
                vendor: "TestVendor".into(),
                total_vram: 16 * 1024 * 1024 * 1024,
                free_vram,
                utilization: 0.3,
                temperature_celsius: 55.0,
                has_kv_cache_for_model: kv_hit > 0.5,
            }],
            engines: vec![EnginePlacementInfo {
                engine_id: format!("{}-eng0", id),
                engine_type: "testEngine".into(),
                loaded_models: loaded_models.iter().map(|s| s.to_string()).collect(),
                active_requests: 0,
                max_batch_size: 32,
                supports_streaming: true,
                capabilities: vec!["chat".into(), "tools".into()],
                healthy: true,
                queue_depth: 0,
                model_load_ms: 500,
                state_transfer_ms: 10,
                estimated_execution_ms: 100,
                estimated_tps: engine_tps,
                kv_cache_hit_rate: kv_hit,
            }],
            network_latency_us: if id == "local" { 0 } else { 5000 },
            is_local: id == "local",
            connection_state: crate::NodeConnectionState::Connected,
        }
    }

    // -- Test 1: Resource-sufficient workload is placed --

    #[test]
    fn test_workload_with_sufficient_resources_is_placed() {
        let policy = WeightedSumPolicy::default();
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let nodes = vec![make_test_node(
            "local",
            "cuda",
            12 * 1024 * 1024 * 1024,
            16 * 1024 * 1024 * 1024,
            100.0,
            0.8,
            vec!["test-model"],
        )];

        let workload = WorkloadSpec {
            resource_requirements: nous_types::workload::ResourceRequirements {
                minimum: ResourceVector {
                    device_memory_bytes: 4 * 1024 * 1024 * 1024,
                    ram_bytes: 8 * 1024 * 1024 * 1024,
                    ..Default::default()
                },
                ..Default::default()
            },
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["test-model".into()],
                ..Default::default()
            },
            ..Default::default()
        };

        let decision = scheduler.place(&workload, &nodes);
        assert!(
            decision.is_some(),
            "Workload with sufficient resources must be placed"
        );
        let d = decision.unwrap();
        assert_eq!(d.node_id, "local");
        assert!(!d.reasoning.is_empty());
    }

    // -- Test 2: Resource-insufficient workload is rejected --

    #[test]
    fn test_workload_with_insufficient_vram_is_rejected() {
        let policy = WeightedSumPolicy::default();
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let nodes = vec![make_test_node(
            "small",
            "cuda",
            512 * 1024 * 1024,
            8 * 1024 * 1024 * 1024,
            50.0,
            0.5,
            vec![],
        )];

        let workload = WorkloadSpec {
            resource_requirements: nous_types::workload::ResourceRequirements {
                minimum: ResourceVector {
                    device_memory_bytes: 8 * 1024 * 1024 * 1024, // 8GB needed, only 512MB available
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let decision = scheduler.place(&workload, &nodes);
        assert!(
            decision.is_none(),
            "Workload with insufficient VRAM must be rejected"
        );
    }

    // -- Test 3: All 10 policies produce valid decisions --

    #[test]
    fn test_all_policies_produce_decisions() {
        let nodes = vec![
            make_test_node(
                "local",
                "cuda",
                8 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                80.0,
                0.9,
                vec!["model-a"],
            ),
            make_test_node(
                "remote",
                "cuda",
                12 * 1024 * 1024 * 1024,
                32 * 1024 * 1024 * 1024,
                120.0,
                0.3,
                vec!["model-a", "model-b"],
            ),
        ];

        let workload = WorkloadSpec {
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["model-a".into()],
                ..Default::default()
            },
            ..Default::default()
        };

        let policies: Vec<Box<dyn SchedulerPolicy>> = vec![
            Box::new(FifoPolicy),
            Box::new(PriorityPolicy),
            Box::new(WeightedSumPolicy::default()),
            Box::new(MultiplicativePolicy),
            Box::new(ParetoPolicy::default()),
            Box::new(CacheAwarePolicy::default()),
            Box::new(DeadlineAwarePolicy),
            Box::new(CriticalPathPolicy),
            Box::new(ContextualBanditPolicy::default()),
        ];

        for policy in policies {
            let scheduler = PlacementScheduler::new(policy);
            let decision = scheduler.place(&workload, &nodes);
            assert!(
                decision.is_some(),
                "Policy must produce a placement for valid candidates"
            );
        }
    }

    // -- Test 4: Empty candidate set returns None --

    #[test]
    fn test_empty_nodes_returns_none() {
        let policy = WeightedSumPolicy::default();
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let decision = scheduler.place(&WorkloadSpec::default(), &[]);
        assert!(
            decision.is_none(),
            "Empty node list must return no placement"
        );
    }

    // -- Test 5: Deadline-aware policy prefers faster engines --

    #[test]
    fn test_deadline_aware_policy_prefers_faster_engine() {
        let policy = DeadlineAwarePolicy;
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let nodes = vec![
            make_test_node(
                "slow",
                "cuda",
                8 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                20.0,
                0.5,
                vec!["model-x"],
            ),
            make_test_node(
                "fast",
                "cuda",
                8 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                200.0,
                0.5,
                vec!["model-x"],
            ),
        ];

        let workload = WorkloadSpec {
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["model-x".into()],
                ..Default::default()
            },
            deadline_us: 100_000, // Short deadline - need fast engine
            ..Default::default()
        };

        let decision = scheduler.place(&workload, &nodes);
        assert!(decision.is_some());
        // DeadlineAwarePolicy should prefer the faster engine
        let d = decision.unwrap();
        assert_eq!(
            d.engine_id, "fast-eng0",
            "Deadline-aware policy must prefer faster engine under deadline"
        );
    }

    // -- Test 6: Cache-aware policy prefers nodes with KV cache hits --

    #[test]
    fn test_cache_aware_policy_prefers_kv_hit() {
        let policy = CacheAwarePolicy::default();
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let nodes = vec![
            make_test_node(
                "no-cache",
                "cuda",
                8 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                100.0,
                0.0,
                vec!["model-y"],
            ),
            make_test_node(
                "has-cache",
                "cuda",
                8 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                100.0,
                0.95,
                vec!["model-y"],
            ),
        ];

        let workload = WorkloadSpec {
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["model-y".into()],
                ..Default::default()
            },
            ..Default::default()
        };

        let decision = scheduler.place(&workload, &nodes);
        assert!(decision.is_some());
        let d = decision.unwrap();
        assert_eq!(
            d.node_id, "has-cache",
            "Cache-aware policy must prefer node with KV cache hits"
        );
    }

    // -- Test 7: Placement decisions are deterministic for same input --

    #[test]
    fn test_placement_is_deterministic() {
        let policy = WeightedSumPolicy::default();
        let nodes = vec![make_test_node(
            "n1",
            "cuda",
            8 * 1024 * 1024 * 1024,
            16 * 1024 * 1024 * 1024,
            100.0,
            0.5,
            vec!["m1"],
        )];
        let workload = WorkloadSpec::default();

        let scheduler = PlacementScheduler::new(Box::new(policy));
        let d1 = scheduler.place(&workload, &nodes);
        let d2 = scheduler.place(&workload, &nodes);

        match (d1, d2) {
            (Some(a), Some(b)) => {
                assert_eq!(a.node_id, b.node_id);
                assert_eq!(a.engine_id, b.engine_id);
                assert_eq!(a.device_id, b.device_id);
            }
            _ => panic!("Deterministic: both calls must produce same result"),
        }
    }

    // -- Test 8: Filter rejects incompatible device types --

    #[test]
    fn test_filter_rejects_incompatible_device_type() {
        let policy = WeightedSumPolicy::default();
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let nodes = vec![make_test_node(
            "cpu-only",
            "cpu",
            0,
            16 * 1024 * 1024 * 1024,
            10.0,
            0.0,
            vec![],
        )];

        let workload = WorkloadSpec {
            device_requirements: nous_types::workload::DeviceRequirements {
                allowed_device_types: vec![nous_types::workload::DeviceType::Cuda],
                ..Default::default()
            },
            ..Default::default()
        };

        let decision = scheduler.place(&workload, &nodes);
        assert!(
            decision.is_none(),
            "CPU-only node must be rejected when CUDA is required"
        );
    }

    // -- Test 9: Multiple nodes - best one selected --

    #[test]
    fn test_best_node_selected_from_multiple() {
        let policy = WeightedSumPolicy::default();
        let scheduler = PlacementScheduler::new(Box::new(policy));
        let nodes = vec![
            make_test_node(
                "n-low-tps",
                "cuda",
                12 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                30.0,
                0.3,
                vec!["m"],
            ),
            make_test_node(
                "n-high-tps",
                "cuda",
                12 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                150.0,
                0.7,
                vec!["m"],
            ),
            make_test_node(
                "n-busy",
                "cuda",
                12 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
                100.0,
                0.5,
                vec!["m"],
            ),
        ];

        // Make n-busy actually busy
        let mut busy_nodes = nodes.clone();
        busy_nodes[2].engines[0].active_requests = 30; // High load
        busy_nodes[2].devices[0].utilization = 0.95;

        let workload = WorkloadSpec {
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["m".into()],
                ..Default::default()
            },
            ..Default::default()
        };

        let decision = scheduler.place(&workload, &busy_nodes);
        assert!(decision.is_some());
        let d = decision.unwrap();
        // n-high-tps has highest TPS and good KV hit rate, n-busy is overloaded
        assert_eq!(
            d.node_id, "n-high-tps",
            "Scheduler should pick best node (high TPS + low load), got {}",
            d.node_id
        );
    }

    #[test]
    fn local_only_is_a_hard_feasibility_constraint() {
        let scheduler = PlacementScheduler::new(Box::new(WeightedSumPolicy::default()));
        let remote = make_test_node(
            "remote",
            "cuda",
            12 * 1024 * 1024 * 1024,
            16 * 1024 * 1024 * 1024,
            100.0,
            0.8,
            vec!["model-a"],
        );
        let workload = WorkloadSpec {
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["model-a".into()],
                routing: RoutingPreference::LocalOnly,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(scheduler.place(&workload, &[remote]).is_none());
    }

    #[test]
    fn missing_model_capability_is_not_soft_scored() {
        let scheduler = PlacementScheduler::new(Box::new(WeightedSumPolicy::default()));
        let mut local = make_test_node(
            "local",
            "cuda",
            12 * 1024 * 1024 * 1024,
            16 * 1024 * 1024 * 1024,
            100.0,
            0.8,
            vec!["model-a"],
        );
        local.engines[0].capabilities = vec!["chat".into()];
        let workload = WorkloadSpec {
            model_requirements: ModelRequirements {
                allowed_model_families: vec!["model-a".into()],
                ..Default::default()
            },
            capability_requirements: CapabilityRequirements {
                required_capabilities: vec!["vision".into()],
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(scheduler.place(&workload, &[local]).is_none());
    }
}
