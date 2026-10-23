use std::fs;
use std::path::Path;

use super::escape_markup;

/// Scaffold `builds/web/` for a new project.
///
/// `name` is the Cargo package Trunk builds; `app_name` is the page title.
pub fn create(dir: &Path, name: &str, app_name: &str, _group: &str) {
    let web_dir = dir.join("builds/web");
    fs::create_dir_all(&web_dir).unwrap();

    // Generate Trunk.toml
    fs::write(
        web_dir.join("Trunk.toml"),
        include_str!("../../../templates/web/Trunk.toml.template"),
    )
    .unwrap();

    // Generate index.html
    fs::write(
        web_dir.join("index.html"),
        include_str!("../../../templates/web/index.html.template")
            .replace("${app_title}", &escape_markup(app_name))
            .replace("${bin_name}", name),
    )
    .unwrap();

    // Default favicon and icons
    fs::write(
        web_dir.join("favicon.ico"),
        include_bytes!("../../../templates/icons/favicon.ico"),
    )
    .unwrap();
    fs::write(
        web_dir.join("apple-touch-icon.png"),
        include_bytes!("../../../templates/icons/icon_180.png"),
    )
    .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_web_scaffold() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("test-app");
        fs::create_dir_all(&dir).unwrap();

        create(&dir, "test-app", "Test App", "com.example.test");

        let web_dir = dir.join("builds/web");
        assert!(web_dir.join("Trunk.toml").exists(), "missing Trunk.toml");
        assert!(web_dir.join("index.html").exists(), "missing index.html");
        assert!(web_dir.join("favicon.ico").exists(), "missing favicon.ico");
        assert!(
            web_dir.join("apple-touch-icon.png").exists(),
            "missing apple-touch-icon.png"
        );

        // Old Vite/npm files should NOT exist
        assert!(
            !web_dir.join("package.json").exists(),
            "package.json should not exist"
        );
        assert!(
            !web_dir.join("vite.config.ts").exists(),
            "vite.config.ts should not exist"
        );
        assert!(
            !web_dir.join("main.ts").exists(),
            "main.ts should not exist"
        );

        // index.html should contain Trunk data-trunk attribute
        let html = fs::read_to_string(web_dir.join("index.html")).unwrap();
        assert!(
            html.contains("data-trunk"),
            "index.html missing data-trunk attribute"
        );
        assert!(
            html.contains("<title>Test App</title>"),
            "index.html missing app name in title"
        );
        assert!(
            html.contains("data-bin=\"test-app\""),
            "index.html missing the Cargo package as the Trunk bin"
        );
        assert!(
            html.contains("id=\"aimer_app\""),
            "index.html missing #aimer_app div"
        );
    }
}
