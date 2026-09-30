use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::RwLock;

use driftfs_core::{DriftFsError, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaultType {
    RateLimit { retry_after_secs: Option<u64> },
    AuthExpired,
    ServerError { message: String },
    NetworkDrop,
    MidStreamReadCutoff { max_bytes: usize },
    UploadFailure,
}

#[derive(Debug, Clone)]
pub struct FaultRule {
    pub fault: FaultType,
    pub after_calls: usize,
    pub count: usize,
}

impl FaultRule {
    pub fn new(fault: FaultType, after_calls: usize, count: usize) -> Self {
        Self {
            fault,
            after_calls,
            count,
        }
    }
}

#[derive(Default)]
pub struct FaultInjector {
    rules: RwLock<HashMap<String, Vec<FaultRule>>>,
    counters: RwLock<HashMap<String, AtomicUsize>>,
}

impl FaultInjector {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_rule(&self, operation: &str, rule: FaultRule) {
        let mut rules = self.rules.write().unwrap();
        rules.entry(operation.to_string()).or_default().push(rule);
    }

    pub fn clear(&self) {
        self.rules.write().unwrap().clear();
        self.counters.write().unwrap().clear();
    }

    pub fn check(&self, operation: &str) -> Result<Option<FaultType>> {
        let call_idx = {
            let counters = self.counters.read().unwrap();
            if let Some(counter) = counters.get(operation) {
                counter.fetch_add(1, Ordering::SeqCst)
            } else {
                drop(counters);
                let mut counters = self.counters.write().unwrap();
                let counter = counters
                    .entry(operation.to_string())
                    .or_insert_with(|| AtomicUsize::new(0));
                counter.fetch_add(1, Ordering::SeqCst)
            }
        };

        let rules = self.rules.read().unwrap();
        let op_rules = match rules.get(operation) {
            Some(r) => r,
            None => return Ok(None),
        };

        for rule in op_rules {
            if call_idx >= rule.after_calls && call_idx < rule.after_calls + rule.count {
                return match &rule.fault {
                    FaultType::RateLimit { retry_after_secs } => Err(DriftFsError::RateLimited {
                        retry_after_secs: *retry_after_secs,
                    }),
                    FaultType::AuthExpired => {
                        Err(DriftFsError::auth("mock: authentication expired (401)"))
                    }
                    FaultType::ServerError { message } => Err(DriftFsError::Provider {
                        message: message.clone(),
                        source: None,
                    }),
                    FaultType::NetworkDrop => Err(DriftFsError::Network {
                        message: "mock: network dropped mid-operation".into(),
                        source: None,
                    }),
                    FaultType::UploadFailure => Err(DriftFsError::Storage {
                        message: "mock: upload stream interrupted unexpectedly".into(),
                        source: None,
                    }),
                    FaultType::MidStreamReadCutoff { max_bytes } => {
                        Ok(Some(FaultType::MidStreamReadCutoff {
                            max_bytes: *max_bytes,
                        }))
                    }
                };
            }
        }

        Ok(None)
    }
}
