// Platform detection and subsystem configuration
// Requirements: 18.1, 18.3, 18.4

#[derive(Debug, Clone, PartialEq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Arch {
    X86_64,
    Arm64,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum IpcMechanism {
    UnixDomainSocket,
    NamedPipe,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotStrategy {
    ApfsReflink,
    BtrfsCow,
    FileCopyFallback,
}

#[derive(Debug, Clone)]
pub struct PlatformConfig {
    pub os: Os,
    pub arch: Arch,
    pub ipc: IpcMechanism,
    pub snapshot_strategy: SnapshotStrategy,
    pub keychain_available: bool,
}

impl PlatformConfig {
    pub fn detect() -> Self {
        let os = detect_os();
        let arch = detect_arch();
        let ipc = detect_ipc(&os);
        let snapshot_strategy = detect_snapshot_strategy(&os);
        let keychain_available = detect_keychain(&os);
        PlatformConfig {
            os,
            arch,
            ipc,
            snapshot_strategy,
            keychain_available,
        }
    }
}

fn detect_os() -> Os {
    #[cfg(target_os = "macos")]
    return Os::MacOs;
    #[cfg(target_os = "windows")]
    return Os::Windows;
    #[cfg(target_os = "linux")]
    return Os::Linux;
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    return Os::Other(std::env::consts::OS.to_string());
}

fn detect_arch() -> Arch {
    #[cfg(target_arch = "x86_64")]
    return Arch::X86_64;
    #[cfg(target_arch = "aarch64")]
    return Arch::Arm64;
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    return Arch::Other(std::env::consts::ARCH.to_string());
}

pub fn detect_ipc(os: &Os) -> IpcMechanism {
    match os {
        Os::Windows => IpcMechanism::NamedPipe,
        _ => IpcMechanism::UnixDomainSocket,
    }
}

pub fn detect_snapshot_strategy(os: &Os) -> SnapshotStrategy {
    match os {
        Os::MacOs => SnapshotStrategy::ApfsReflink,
        Os::Linux => SnapshotStrategy::BtrfsCow,
        _ => SnapshotStrategy::FileCopyFallback,
    }
}

pub fn detect_keychain(os: &Os) -> bool {
    matches!(os, Os::MacOs | Os::Windows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_returns_current_os() {
        let config = PlatformConfig::detect();
        #[cfg(target_os = "macos")]
        assert_eq!(config.os, Os::MacOs);
        #[cfg(target_os = "windows")]
        assert_eq!(config.os, Os::Windows);
        #[cfg(target_os = "linux")]
        assert_eq!(config.os, Os::Linux);
    }

    #[test]
    fn test_detect_returns_current_arch() {
        let config = PlatformConfig::detect();
        #[cfg(target_arch = "x86_64")]
        assert_eq!(config.arch, Arch::X86_64);
        #[cfg(target_arch = "aarch64")]
        assert_eq!(config.arch, Arch::Arm64);
    }

    #[test]
    fn test_macos_subsystem_config() {
        let config = PlatformConfig {
            os: Os::MacOs,
            arch: Arch::Arm64,
            ipc: detect_ipc(&Os::MacOs),
            snapshot_strategy: detect_snapshot_strategy(&Os::MacOs),
            keychain_available: detect_keychain(&Os::MacOs),
        };
        assert_eq!(config.ipc, IpcMechanism::UnixDomainSocket);
        assert_eq!(config.snapshot_strategy, SnapshotStrategy::ApfsReflink);
        assert!(config.keychain_available);
    }

    #[test]
    fn test_windows_subsystem_config() {
        let config = PlatformConfig {
            os: Os::Windows,
            arch: Arch::X86_64,
            ipc: detect_ipc(&Os::Windows),
            snapshot_strategy: detect_snapshot_strategy(&Os::Windows),
            keychain_available: detect_keychain(&Os::Windows),
        };
        assert_eq!(config.ipc, IpcMechanism::NamedPipe);
        assert_eq!(config.snapshot_strategy, SnapshotStrategy::FileCopyFallback);
        assert!(config.keychain_available);
    }

    #[test]
    fn test_linux_subsystem_config() {
        let config = PlatformConfig {
            os: Os::Linux,
            arch: Arch::X86_64,
            ipc: detect_ipc(&Os::Linux),
            snapshot_strategy: detect_snapshot_strategy(&Os::Linux),
            keychain_available: detect_keychain(&Os::Linux),
        };
        assert_eq!(config.ipc, IpcMechanism::UnixDomainSocket);
        assert_eq!(config.snapshot_strategy, SnapshotStrategy::BtrfsCow);
        assert!(!config.keychain_available);
    }

    #[test]
    fn test_other_os_subsystem_config() {
        let other = Os::Other("freebsd".to_string());
        assert_eq!(detect_ipc(&other), IpcMechanism::UnixDomainSocket);
        assert_eq!(
            detect_snapshot_strategy(&other),
            SnapshotStrategy::FileCopyFallback
        );
        assert!(!detect_keychain(&other));
    }

    #[test]
    fn test_detected_ipc_matches_os() {
        let config = PlatformConfig::detect();
        #[cfg(target_os = "windows")]
        assert_eq!(config.ipc, IpcMechanism::NamedPipe);
        #[cfg(not(target_os = "windows"))]
        assert_eq!(config.ipc, IpcMechanism::UnixDomainSocket);
    }

    #[test]
    fn test_detected_snapshot_strategy_matches_os() {
        let config = PlatformConfig::detect();
        #[cfg(target_os = "macos")]
        assert_eq!(config.snapshot_strategy, SnapshotStrategy::ApfsReflink);
        #[cfg(target_os = "linux")]
        assert_eq!(config.snapshot_strategy, SnapshotStrategy::BtrfsCow);
        #[cfg(target_os = "windows")]
        assert_eq!(config.snapshot_strategy, SnapshotStrategy::FileCopyFallback);
    }

    #[test]
    fn test_detected_keychain_matches_os() {
        let config = PlatformConfig::detect();
        #[cfg(any(target_os = "macos", target_os = "windows"))]
        assert!(config.keychain_available);
        #[cfg(target_os = "linux")]
        assert!(!config.keychain_available);
    }
}
