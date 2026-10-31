use std::collections::HashMap;

use super::*;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    move |key| map.get(key).cloned()
}

fn spec() -> ToolSpec<'static> {
    ToolSpec {
        name: "mytool",
        env_var: "AIMER_MYTOOL",
        file_names: &["mytool", "mytool.exe"],
        extra_dirs: vec![
            PathBuf::from("/opt/mytool"),
            PathBuf::from("/usr/local/mytool"),
        ],
        how_to_get: "download it from https://example.com",
    }
}

#[test]
fn the_environment_variable_wins_over_the_search() {
    let env = env_of(&[("AIMER_MYTOOL", " /custom/mytool "), ("PATH", "/bin")]);

    let found = locate(&spec(), &env, &|_| true);

    assert_eq!(found.unwrap(), Path::new("/custom/mytool"));
}

#[test]
fn a_wrong_environment_path_is_an_error_and_never_a_fallback() {
    let env = env_of(&[("AIMER_MYTOOL", "/nope"), ("PATH", "/bin")]);

    let err = locate(&spec(), &env, &|p| p == Path::new("/bin/mytool"))
        .unwrap_err()
        .to_string();

    assert!(
        err.contains("AIMER_MYTOOL") && err.contains("/nope"),
        "{err}"
    );
}

#[test]
fn an_empty_variable_counts_as_unset() {
    let env = env_of(&[("AIMER_MYTOOL", "  "), ("PATH", "/bin")]);

    let found = locate(&spec(), &env, &|p| p == Path::new("/bin/mytool"));

    assert_eq!(found.unwrap(), Path::new("/bin/mytool"));
}

#[test]
fn path_is_searched_in_order_by_every_file_name() {
    let env = env_of(&[("PATH", "/a:/b")]);

    let first = locate(&spec(), &env, &|p| {
        p == Path::new("/a/mytool.exe") || p == Path::new("/b/mytool")
    });

    assert_eq!(first.unwrap(), Path::new("/a/mytool.exe"));
}

#[test]
fn the_usual_install_folders_are_searched_after_path() {
    let env = env_of(&[("PATH", "/bin")]);

    let found = locate(&spec(), &env, &|p| {
        p == Path::new("/usr/local/mytool/mytool")
    });
    assert_eq!(found.unwrap(), Path::new("/usr/local/mytool/mytool"));

    let both = locate(&spec(), &env, &|p| {
        p == Path::new("/bin/mytool") || p == Path::new("/opt/mytool/mytool")
    });
    assert_eq!(both.unwrap(), Path::new("/bin/mytool"));
}

#[test]
fn a_missing_tool_says_how_to_get_it_and_how_to_point_at_it() {
    let env = env_of(&[("PATH", "/bin")]);

    let err = locate(&spec(), &env, &|_| false).unwrap_err().to_string();

    assert!(err.starts_with("mytool was not found"), "{err}");
    assert!(
        err.contains("download it from https://example.com"),
        "{err}"
    );
    assert!(err.contains("AIMER_MYTOOL"), "{err}");
}

#[test]
fn without_a_path_variable_only_the_install_folders_are_searched() {
    let found = locate(&spec(), &env_of(&[]), &|p| {
        p == Path::new("/opt/mytool/mytool")
    });

    assert_eq!(found.unwrap(), Path::new("/opt/mytool/mytool"));
}
