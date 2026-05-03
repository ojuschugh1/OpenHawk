use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use sysinfo::{Pid, System};
use tokio::sync::mpsc;

use crate::types::ProcessId;

#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub cpu_percent: u8,
    pub memory_mb: u64,
    pub max_open_fds: u64,
}

#[derive(Debug, Clone)]
pub struct ResourceSnapshot {
    pub pid: ProcessId,
    pub cpu_percent: f32,
    pub memory_bytes: u64,
}

#[derive(Debug)]
pub enum ResourceEvent {
    MemoryExceeded {
        pid: ProcessId,
        memory_bytes: u64,
        limit_bytes: u64,
    },
}

pub struct ResourceMonitor {
    pub limits: Arc<Mutex<HashMap<ProcessId, ResourceLimits>>>,
    pub event_tx: mpsc::UnboundedSender<ResourceEvent>,
    pub event_rx: Arc<Mutex<mpsc::UnboundedReceiver<ResourceEvent>>>,
}

impl ResourceMonitor {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            limits: Arc::new(Mutex::new(HashMap::new())),
            event_tx: tx,
            event_rx: Arc::new(Mutex::new(rx)),
        }
    }

    pub fn register(&self, pid: ProcessId, limits: ResourceLimits) {
        self.limits.lock().unwrap().insert(pid, limits);
    }

    pub fn deregister(&self, pid: ProcessId) {
        self.limits.lock().unwrap().remove(&pid);
    }

    pub fn start_background(self: Arc<Self>, interval: Duration) {
        let monitor = Arc::clone(&self);
        tokio::spawn(async move {
            let mut sys = System::new_all();
            loop {
                tokio::time::sleep(interval).await;
                sys.refresh_all();

                let snapshot: Vec<(ProcessId, ResourceLimits)> =
                    monitor.limits.lock().unwrap().clone().into_iter().collect();

                for (pid, limits) in snapshot {
                    let sysinfo_pid = Pid::from_u32(pid);
                    if let Some(proc) = sys.process(sysinfo_pid) {
                        let mem = proc.memory();
                        let limit_bytes = limits.memory_mb * 1024 * 1024;
                        if mem > limit_bytes {
                            suspend_process(pid);
                            let _ = monitor.event_tx.send(ResourceEvent::MemoryExceeded {
                                pid,
                                memory_bytes: mem,
                                limit_bytes,
                            });
                        }
                    }
                }
            }
        });
    }

    pub fn sample(&self, pid: ProcessId) -> Option<ResourceSnapshot> {
        let mut sys = System::new_all();
        sys.refresh_all();
        sys.process(Pid::from_u32(pid)).map(|p| ResourceSnapshot {
            pid,
            cpu_percent: p.cpu_usage(),
            memory_bytes: p.memory(),
        })
    }
}

impl Default for ResourceMonitor {
    fn default() -> Self {
        Self::new()
    }
}

fn suspend_process(pid: ProcessId) {
    #[cfg(target_family = "unix")]
    {
        use nix::sys::signal::{kill, Signal};
        use nix::unistd::Pid as NixPid;
        let _ = kill(NixPid::from_raw(pid as i32), Signal::SIGSTOP);
    }
    #[cfg(not(target_family = "unix"))]
    let _ = pid;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_deregister() {
        let monitor = ResourceMonitor::new();
        monitor.register(
            1234,
            ResourceLimits {
                cpu_percent: 25,
                memory_mb: 512,
                max_open_fds: 64,
            },
        );
        assert!(monitor.limits.lock().unwrap().contains_key(&1234));
        monitor.deregister(1234);
        assert!(!monitor.limits.lock().unwrap().contains_key(&1234));
    }

    #[test]
    fn test_memory_limit_bytes_calculation() {
        let limits = ResourceLimits {
            cpu_percent: 10,
            memory_mb: 512,
            max_open_fds: 64,
        };
        assert_eq!(limits.memory_mb * 1024 * 1024, 512 * 1024 * 1024);
    }

    #[tokio::test]
    async fn test_memory_exceeded_event_emitted() {
        let monitor = ResourceMonitor::new();
        monitor
            .event_tx
            .send(ResourceEvent::MemoryExceeded {
                pid: 42,
                memory_bytes: 600 * 1024 * 1024,
                limit_bytes: 512 * 1024 * 1024,
            })
            .unwrap();

        let event = monitor.event_rx.lock().unwrap().try_recv().unwrap();
        match event {
            ResourceEvent::MemoryExceeded {
                pid,
                memory_bytes,
                limit_bytes,
            } => {
                assert_eq!(pid, 42);
                assert!(memory_bytes > limit_bytes);
            }
        }
    }
}
