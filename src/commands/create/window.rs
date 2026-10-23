use std::fs;
use std::path::Path;

use super::escape_markup;

/// Scaffold `builds/windows/` for a new project.
///
/// Both files are consumed by
/// [`package_desktop`](crate::commands::assemble::package_desktop), which copies
/// them into the portable bundle: the icon as `app.ico` and the manifest as
/// `<exe>.manifest`, the name the Windows loader reads as an external manifest.
/// Embedding either one into the PE header would need `rc.exe`, so they ship
/// next to the executable instead. `app_name` is the description the manifest
/// carries.
pub fn create(dir: &Path, _name: &str, app_name: &str, group: &str) {
    let app_name = escape_markup(app_name);
    fs::create_dir_all(dir.join("builds/windows")).unwrap();
    fs::write(
        dir.join("builds/windows/app.manifest"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="1.0.0.0" processorArchitecture="*" name="{group}" type="win32"/>
  <description>{app_name}</description>
  <trustInfo xmlns="urn:schemas-microsoft-com:asm.v3">
    <security>
      <requestedPrivileges>
        <requestedExecutionLevel level="asInvoker" uiAccess="false"/>
      </requestedPrivileges>
    </security>
  </trustInfo>
</assembly>
"#
        ),
    )
    .unwrap();

    // Default application icon
    fs::write(
        dir.join("builds/windows/app.ico"),
        include_bytes!("../../../templates/icons/app.ico"),
    )
    .unwrap();
}
