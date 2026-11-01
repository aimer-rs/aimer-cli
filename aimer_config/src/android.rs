//! The `[android]` table of `Aimer.toml`.

use serde::{Deserialize, Serialize};

/// Settings that only apply to the Android target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidMeta {
    /// The `[android.signing]` table used by `aimer package android`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing: Option<AndroidSigningMeta>,
    /// The `versionCode` of packaged builds. Google Play needs it to grow with
    /// every upload; when unset it is derived from `[package] version`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version_code: Option<u32>,
}

/// Signing settings for `aimer package android`.
///
/// Names and paths only. The keystore and key passwords are read from
/// environment variables and never stored here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AndroidSigningMeta {
    /// The keystore (`.jks` or `.p12`), relative to the project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keystore: Option<String>,
    /// The alias of the key inside the keystore.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_alias: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::config::AimerManifest;

    #[test]
    fn the_android_tables_are_read_from_the_manifest() {
        let manifest: AimerManifest = toml::from_str(
            r#"
            [package]
            name = "App"

            [android]
            version_code = 42

            [android.signing]
            keystore = "keys/upload.jks"
            key_alias = "upload"
            "#,
        )
        .unwrap();

        let android = manifest.android.unwrap();
        assert_eq!(android.version_code, Some(42));
        let signing = android.signing.unwrap();
        assert_eq!(signing.keystore.as_deref(), Some("keys/upload.jks"));
        assert_eq!(signing.key_alias.as_deref(), Some("upload"));
    }

    #[test]
    fn manifests_without_an_android_table_still_load() {
        let manifest: AimerManifest = toml::from_str("[package]\nname = \"App\"\n").unwrap();

        assert_eq!(manifest.android, None);
    }

    #[test]
    fn an_unset_android_table_is_not_written_back() {
        let manifest = AimerManifest::new("App", "1.0.0", "", "", "");

        assert!(!toml::to_string(&manifest).unwrap().contains("android"));
    }
}
