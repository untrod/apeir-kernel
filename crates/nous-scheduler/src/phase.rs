//! Phase Scheduler - Encoder/Prefill/Decode co-location vs disaggregation.
//!
//! Decides whether phases should run:
//! - Same machine (co-located)
//! - Same process (shared memory)
//! - Cross-process (IPC)
//! - Cross-GPU (NVLink/NVSwitch)
//! - Cross-node (network)
//! - Disaggregated (separate prefill and decode servers)

/// Strategy for phase placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhasePlacement {
    /// All phases on the same device in the same engine.
    Colocated,
    /// Prefill and decode on different devices (same node).
    PrefillDecodeDisaggregated,
    /// Encoder, prefill, and decode each on different nodes.
    FullyDisaggregated,
    /// Encoder and prefill together, decode separate.
    EncoderPrefillColocated,
    /// Dynamically migrate phases based on load.
    Adaptive,
}

/// Configuration for phase scheduling.
#[derive(Debug, Clone)]
pub struct PhaseSchedulerConfig {
    /// Maximum prompt length for colocated execution.
    pub max_prompt_len_colocated: u64,
    /// Minimum output length to justify disaggregation.
    pub min_output_len_disaggregated: u64,
    /// KV cache size threshold for migration.
    pub kv_size_threshold_bytes: u64,
    /// Network bandwidth required for disaggregation (bps).
    pub min_network_bw_disaggregated: u64,
    /// Whether adaptive scheduling is enabled.
    pub adaptive_enabled: bool,
}

impl Default for PhaseSchedulerConfig {
    fn default() -> Self {
        Self {
            max_prompt_len_colocated: 4096,
            min_output_len_disaggregated: 512,
            kv_size_threshold_bytes: 1024 * 1024 * 1024, // 1 GB
            min_network_bw_disaggregated: 10_000_000_000, // 10 Gbps
            adaptive_enabled: false,
        }
    }
}

/// The Phase Scheduler decides whether to colocate or disaggregate inference phases.
pub struct PhaseScheduler {
    config: PhaseSchedulerConfig,
}

impl PhaseScheduler {
    pub fn new(config: PhaseSchedulerConfig) -> Self {
        Self { config }
    }

    /// Decide the optimal placement for a given workload's phases.
    pub fn decide(
        &self,
        prompt_length: u64,
        estimated_output_length: u64,
        kv_cache_size_bytes: u64,
        available_network_bw_bps: u64,
        gpu_load: f64,
    ) -> PhasePlacement {
        // Rule 1: Short prompts and short outputs -> colocate
        if prompt_length <= self.config.max_prompt_len_colocated
            && estimated_output_length <= self.config.min_output_len_disaggregated
        {
            return PhasePlacement::Colocated;
        }

        // Rule 2: Large KV cache and sufficient network -> disaggregate
        if kv_cache_size_bytes >= self.config.kv_size_threshold_bytes
            && available_network_bw_bps >= self.config.min_network_bw_disaggregated
        {
            return PhasePlacement::PrefillDecodeDisaggregated;
        }

        // Rule 3: GPU under heavy load -> adaptive migration
        if self.config.adaptive_enabled && gpu_load > 0.8 {
            return PhasePlacement::Adaptive;
        }

        // Rule 4: Default -> colocate
        PhasePlacement::Colocated
    }

    /// Estimate the latency impact of a given placement choice.
    pub fn estimate_latency(
        &self,
        placement: PhasePlacement,
        prompt_length: u64,
        output_length: u64,
        network_latency_us: u64,
    ) -> PhaseLatencyEstimate {
        match placement {
            PhasePlacement::Colocated => PhaseLatencyEstimate {
                ttft_ms: prompt_length as f64 * 0.002, // ~2uss per token in batch
                tpot_ms: 20.0,                         // ~20ms per token for decode
                transfer_overhead_ms: 0.0,
                total_ms: prompt_length as f64 * 0.002 + output_length as f64 * 20.0,
            },
            PhasePlacement::PrefillDecodeDisaggregated => PhaseLatencyEstimate {
                ttft_ms: prompt_length as f64 * 0.0015, // Faster prefill on dedicated node
                tpot_ms: 25.0,                          // Slower decode due to KV transfer
                transfer_overhead_ms: network_latency_us as f64 / 1000.0,
                total_ms: prompt_length as f64 * 0.0015
                    + output_length as f64 * 25.0
                    + network_latency_us as f64 / 1000.0,
            },
            PhasePlacement::FullyDisaggregated => PhaseLatencyEstimate {
                ttft_ms: prompt_length as f64 * 0.001,
                tpot_ms: 30.0,
                transfer_overhead_ms: network_latency_us as f64 / 500.0,
                total_ms: prompt_length as f64 * 0.001
                    + output_length as f64 * 30.0
                    + network_latency_us as f64 / 500.0,
            },
            PhasePlacement::EncoderPrefillColocated => PhaseLatencyEstimate {
                ttft_ms: prompt_length as f64 * 0.0018,
                tpot_ms: 22.0,
                transfer_overhead_ms: network_latency_us as f64 / 2000.0,
                total_ms: prompt_length as f64 * 0.0018
                    + output_length as f64 * 22.0
                    + network_latency_us as f64 / 2000.0,
            },
            PhasePlacement::Adaptive => PhaseLatencyEstimate {
                ttft_ms: prompt_length as f64 * 0.002,
                tpot_ms: 20.0,
                transfer_overhead_ms: 0.0, // Adaptive chooses the best at runtime
                total_ms: prompt_length as f64 * 0.002 + output_length as f64 * 20.0,
            },
        }
    }
}

/// Estimated latency for a phase placement decision.
#[derive(Debug, Clone)]
pub struct PhaseLatencyEstimate {
    pub ttft_ms: f64,
    pub tpot_ms: f64,
    pub transfer_overhead_ms: f64,
    pub total_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_short_prompt_colocated() {
        let scheduler = PhaseScheduler::new(PhaseSchedulerConfig::default());
        let decision = scheduler.decide(1000, 100, 100_000_000, 100_000_000_000, 0.5);
        assert_eq!(decision, PhasePlacement::Colocated);
    }

    #[test]
    fn test_large_kv_disaggregated() {
        let scheduler = PhaseScheduler::new(PhaseSchedulerConfig::default());
        let decision = scheduler.decide(32000, 2000, 2_000_000_000, 40_000_000_000, 0.5);
        assert_eq!(decision, PhasePlacement::PrefillDecodeDisaggregated);
    }

    #[test]
    fn test_colocated_latency_no_transfer() {
        let scheduler = PhaseScheduler::new(PhaseSchedulerConfig::default());
        let est = scheduler.estimate_latency(PhasePlacement::Colocated, 2000, 100, 1000);
        assert_eq!(est.transfer_overhead_ms, 0.0);
        assert!(est.ttft_ms < est.tpot_ms * 100.0); // TTFT should be much less than total decode time for long outputs
    }
}
