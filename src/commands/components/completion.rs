//! Shell completion for the component names given to `aimer components add`
//! and `aimer components remove`.

use std::ffi::OsStr;
use std::path::Path;

use clap_complete::engine::CompletionCandidate;

use super::catalog::COMPONENTS;
use super::{aimer_root, is_installed};

/// Components usable on this OS whose name starts with `current`; with
/// `installed_in`, only the ones installed in that Aimer directory.
fn candidates(current: &OsStr, installed_in: Option<&Path>) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    COMPONENTS
        .iter()
        .filter(|component| component.name.starts_with(current))
        .filter(|component| component.supported_here())
        .filter(|component| installed_in.is_none_or(|root| is_installed(root, component)))
        .map(|component| {
            CompletionCandidate::new(component.name).help(Some(component.purpose.into()))
        })
        .collect()
}

/// Every component that can be installed with `aimer components add`.
pub(crate) fn available(current: &OsStr) -> Vec<CompletionCandidate> {
    candidates(current, None)
}

/// Every installed component, for `aimer components remove`.
pub(crate) fn installed(current: &OsStr) -> Vec<CompletionCandidate> {
    match aimer_root() {
        Ok(root) => candidates(current, Some(&root)),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(candidates: Vec<CompletionCandidate>) -> Vec<String> {
        candidates
            .iter()
            .map(|candidate| candidate.get_value().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn available_offers_every_component_for_an_empty_word() {
        let names = values(available(OsStr::new("")));
        let expected: Vec<&str> = COMPONENTS
            .iter()
            .filter(|component| component.supported_here())
            .map(|component| component.name)
            .collect();
        assert_eq!(names, expected);
        assert!(names.starts_with(&["ndk".to_string(), "trunk".to_string()]));
    }

    #[test]
    fn available_skips_installers_for_other_systems() {
        let names = values(available(OsStr::new("")));
        assert_eq!(
            names.contains(&"appimagetool".to_string()),
            cfg!(target_os = "linux")
        );
        assert_eq!(
            names.contains(&"wix".to_string()),
            cfg!(target_os = "windows")
        );
    }

    #[test]
    fn available_filters_by_prefix() {
        assert_eq!(
            values(available(OsStr::new("wasm"))),
            ["wasm-bindgen", "wasm-pack"]
        );
        assert_eq!(values(available(OsStr::new("tr"))), ["trunk"]);
        assert!(available(OsStr::new("nope")).is_empty());
    }

    #[test]
    fn candidates_carry_the_purpose_as_help() {
        let found = available(OsStr::new("trunk"));
        let help = found[0].get_help().expect("help text").to_string();
        assert_eq!(help, "Web (wasm) dev server & bundler");
    }

    #[test]
    fn installed_only_lists_components_present_in_the_root() {
        let root = tempfile::tempdir().unwrap();
        assert!(candidates(OsStr::new(""), Some(root.path())).is_empty());

        let bin = super::super::bin_dir(root.path());
        std::fs::create_dir_all(&bin).unwrap();
        let file = if cfg!(windows) { "trunk.exe" } else { "trunk" };
        std::fs::write(bin.join(file), "").unwrap();
        let found = values(candidates(OsStr::new(""), Some(root.path())));
        assert_eq!(found, ["trunk"]);
    }
}
