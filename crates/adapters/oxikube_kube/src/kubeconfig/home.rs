//! Choosing the home directory (where `~/.kube/config` lives), as client-go does.
//!
//! `homedir.HomeDir` in client-go: on Unix, `$HOME`. On Windows, three variables are candidates
//! (`HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE`) and the choice depends on what is on disk:
//!
//! 1. the first of `HOME`, `HOMEDRIVE`+`HOMEPATH`, `USERPROFILE` that contains `.kube\config`;
//! 2. else the first of `HOME`, `USERPROFILE`, `HOMEDRIVE`+`HOMEPATH` that is an existing,
//!    writable directory;
//! 3. else the first of those that exists;
//! 4. else the first of those that is set.
//!
//! The decision is a pure function over a [`HomeProbe`], so tests inject the file system.

use std::path::{Path, PathBuf};

/// What the probe can say about a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathState {
    /// Does not exist (or cannot be inspected).
    Missing,
    /// Exists but is not a writable directory.
    Exists,
    /// An existing directory the owner can write to.
    WritableDir,
}

/// The file-system questions the choice asks.
pub(super) trait HomeProbe {
    /// Whether `<home>/.kube/config` exists.
    fn kube_config_exists(&self, home: &Path) -> bool;
    /// The state of `path`.
    fn state(&self, path: &Path) -> PathState;
}

/// The environment variables that name a home directory (empty ones are `None`).
#[derive(Debug, Clone, Default)]
pub(super) struct HomeVars {
    pub home: Option<PathBuf>,
    pub home_drive: Option<PathBuf>,
    pub home_path: Option<PathBuf>,
    pub user_profile: Option<PathBuf>,
}

impl HomeVars {
    /// `HOMEDRIVE` + `HOMEPATH`, when both are set.
    fn drive_path(&self) -> Option<PathBuf> {
        let (drive, path) = (self.home_drive.as_ref()?, self.home_path.as_ref()?);
        let mut joined = drive.clone().into_os_string();
        joined.push(path);
        Some(PathBuf::from(joined))
    }
}

/// Choose the home directory for `platform` (see the module docs).
pub(super) fn pick_home(
    platform: super::Platform,
    vars: &HomeVars,
    probe: &dyn HomeProbe,
) -> Option<PathBuf> {
    if platform != super::Platform::Windows {
        return vars.home.clone();
    }
    let drive_path = vars.drive_path();
    let with_config = [&vars.home, &drive_path, &vars.user_profile];
    if let Some(found) = with_config
        .into_iter()
        .flatten()
        .find(|p| probe.kube_config_exists(p))
    {
        return Some(found.clone());
    }
    let by_preference: Vec<&PathBuf> = [&vars.home, &vars.user_profile, &drive_path]
        .into_iter()
        .flatten()
        .collect();
    by_preference
        .iter()
        .find(|p| probe.state(p) == PathState::WritableDir)
        .or_else(|| {
            by_preference
                .iter()
                .find(|p| probe.state(p) != PathState::Missing)
        })
        .or_else(|| by_preference.first())
        .map(|p| (*p).clone())
}

/// The real file system.
pub(super) struct FsProbe;

impl HomeProbe for FsProbe {
    fn kube_config_exists(&self, home: &Path) -> bool {
        home.join(".kube").join("config").exists()
    }

    fn state(&self, path: &Path) -> PathState {
        match std::fs::metadata(path) {
            Err(_) => PathState::Missing,
            Ok(meta) if meta.is_dir() && !meta.permissions().readonly() => PathState::WritableDir,
            Ok(_) => PathState::Exists,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::*;
    use crate::kubeconfig::Platform;

    #[derive(Default)]
    struct FakeFs {
        configs: HashSet<PathBuf>,
        states: HashMap<PathBuf, PathState>,
    }

    impl HomeProbe for FakeFs {
        fn kube_config_exists(&self, home: &Path) -> bool {
            self.configs.contains(home)
        }
        fn state(&self, path: &Path) -> PathState {
            self.states.get(path).copied().unwrap_or(PathState::Missing)
        }
    }

    fn p(s: &str) -> Option<PathBuf> {
        Some(PathBuf::from(s))
    }

    fn vars() -> HomeVars {
        HomeVars {
            home: p("C:\\h"),
            home_drive: p("D:"),
            home_path: p("\\dp"),
            user_profile: p("C:\\up"),
        }
    }

    #[test]
    fn unix_is_home_only() {
        let fs = FakeFs::default();
        assert_eq!(pick_home(Platform::Unix, &vars(), &fs), p("C:\\h"));
        let none = HomeVars {
            home: None,
            ..vars()
        };
        assert_eq!(pick_home(Platform::Unix, &none, &fs), None);
    }

    #[test]
    fn windows_prefers_the_candidate_holding_a_kube_config() {
        // Order for this rule: HOME, HOMEDRIVE+HOMEPATH, USERPROFILE.
        let mut fs = FakeFs::default();
        fs.configs.insert(PathBuf::from("D:\\dp"));
        fs.configs.insert(PathBuf::from("C:\\up"));
        assert_eq!(pick_home(Platform::Windows, &vars(), &fs), p("D:\\dp"));

        fs.configs.insert(PathBuf::from("C:\\h"));
        assert_eq!(pick_home(Platform::Windows, &vars(), &fs), p("C:\\h"));
    }

    #[test]
    fn windows_without_a_config_prefers_the_first_writable_dir() {
        // Order for this rule: HOME, USERPROFILE, HOMEDRIVE+HOMEPATH.
        let mut fs = FakeFs::default();
        fs.states.insert(PathBuf::from("C:\\h"), PathState::Exists);
        fs.states
            .insert(PathBuf::from("D:\\dp"), PathState::WritableDir);
        fs.states
            .insert(PathBuf::from("C:\\up"), PathState::WritableDir);
        assert_eq!(pick_home(Platform::Windows, &vars(), &fs), p("C:\\up"));
    }

    #[test]
    fn windows_falls_back_to_first_existing_then_first_set() {
        let mut fs = FakeFs::default();
        fs.states.insert(PathBuf::from("C:\\up"), PathState::Exists);
        fs.states.insert(PathBuf::from("D:\\dp"), PathState::Exists);
        assert_eq!(pick_home(Platform::Windows, &vars(), &fs), p("C:\\up"));

        let nothing = FakeFs::default();
        assert_eq!(pick_home(Platform::Windows, &vars(), &nothing), p("C:\\h"));

        let only_profile = HomeVars {
            user_profile: p("C:\\up"),
            ..HomeVars::default()
        };
        assert_eq!(
            pick_home(Platform::Windows, &only_profile, &nothing),
            p("C:\\up")
        );
        assert_eq!(
            pick_home(Platform::Windows, &HomeVars::default(), &nothing),
            None
        );
    }

    #[test]
    fn drive_and_path_must_both_be_set() {
        let half = HomeVars {
            home_drive: p("D:"),
            ..HomeVars::default()
        };
        assert_eq!(
            pick_home(Platform::Windows, &half, &FakeFs::default()),
            None
        );
    }
}
