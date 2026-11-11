//! Adding the live activity widget extension to the iOS `project.pbxproj`.
//!
//! The project is text, and the scaffold gives its objects fixed identifiers,
//! so the extension is spliced in at those: each insertion is anchored to an
//! object that must occur exactly once. A project that was edited away from the
//! scaffold simply lacks an anchor; [`add_extension`] then changes nothing and
//! says which one, and the caller tells the user to add the target by hand.

/// The app target the scaffold generates.
const APP_TARGET: &str = "7648BF25D679D16DF8E7E6F4";
/// The app target's `Resources` build phase.
const APP_RESOURCES: &str = "D2F62A4BCEF723F1016DEB73";
/// The group holding the app's sources.
const APP_GROUP: &str = "835F924D3E646A9F762B34B2";
/// The `<name>.app` product reference.
const APP_PRODUCT: &str = "992093C02504A3F306172469";
/// `main.swift` in the app's `Sources` phase.
const APP_MAIN_BUILD_FILE: &str = "C7A3B2D1F5E83D7B1E6F5092";
/// The `PBXProject` object.
const PROJECT: &str = "F745F1A60BF1B4F8EBEABC66";

/// The file that tells a project already has the extension.
pub(super) const WIDGET_FILE: &str = "AimerLiveActivityWidget.swift";
pub(super) const ATTRIBUTES_FILE: &str = "AimerLiveActivityAttributes.swift";
pub(super) const BRIDGE_FILE: &str = "AimerLiveActivityBridge.swift";
pub(super) const PLIST_FILE: &str = "Info.plist";

/// What the extension is called and where its files are.
pub(super) struct Extension<'a> {
    /// The target, the group folder and the product name, `<package>LiveActivity`.
    pub name: &'a str,
    /// The Swift module of the extension, a valid identifier.
    pub module: &'a str,
    /// The bundle identifier: the app's with a suffix.
    pub bundle_id: &'a str,
}

/// An object identifier of the extension. They share a prefix no other
/// generated project uses, and differ in the counter.
fn id(n: u32) -> String {
    format!("AE1A{n:020X}")
}

// The numbers of the objects the extension adds.
const TARGET: u32 = 1;
const PRODUCT: u32 = 2;
const GROUP: u32 = 3;
const SOURCES: u32 = 4;
const DEBUG: u32 = 5;
const RELEASE: u32 = 6;
const CONFIGS: u32 = 7;
const PROXY: u32 = 8;
const DEPENDENCY: u32 = 9;
const EMBED: u32 = 10;
const BRIDGE_REF: u32 = 11;
const ATTRIBUTES_REF: u32 = 12;
const WIDGET_REF: u32 = 13;
const PLIST_REF: u32 = 14;
const BRIDGE_BUILD: u32 = 16;
const ATTRIBUTES_BUILD: u32 = 17;
const ATTRIBUTES_EXT_BUILD: u32 = 18;
const WIDGET_BUILD: u32 = 19;
const PRODUCT_BUILD: u32 = 20;

/// Pins the extension's bundle name. `aimer assemble` passes
/// `PRODUCT_NAME=<app>` on the `xcodebuild` command line, which overrides the
/// product name of every target, the extension's included: it would be built
/// as `<app>.appex` while the embed phase copies `<name>.appex`. Command-line
/// values do not reach `WRAPPER_NAME`, so the bundle keeps the name the project
/// refers to.
fn wrapper_setting(name: &str) -> String {
    format!("WRAPPER_NAME = \"{name}.appex\";")
}

/// `pbxproj` with the extension's bundle name pinned, for a project generated
/// before that setting existed; `None` when there is nothing to change or an
/// extension configuration cannot be found.
pub(super) fn pin_wrapper_name(pbxproj: &str, name: &str) -> Option<String> {
    let setting = wrapper_setting(name);
    let mut text = pbxproj.to_owned();
    for (number, config) in [(DEBUG, "Debug"), (RELEASE, "Release")] {
        let start = text.find(&format!("\t\t{} /* {config} */ = {{", id(number)))?;
        let end = start + text[start..].find("\t\t};\n")?;
        if text[start..end].contains("WRAPPER_NAME") {
            continue;
        }
        let anchor = start + text[start..end].find("\t\t\t\tSKIP_INSTALL = YES;\n")?;
        let line_end = anchor + text[anchor..].find('\n')? + 1;
        text.insert_str(line_end, &format!("\t\t\t\t{setting}\n"));
    }
    (text != pbxproj).then_some(text)
}

/// Whether the project already carries the extension.
pub(super) fn has_extension(pbxproj: &str) -> bool {
    pbxproj.contains(WIDGET_FILE)
}

/// `pbxproj` with the extension target added, or the reason it cannot be.
pub(super) fn add_extension(pbxproj: &str, ext: &Extension) -> Result<String, String> {
    let mut text = pbxproj.to_owned();
    let name = ext.name;

    // First, while the app target's is still the only empty dependency list.
    replace_once(
        &mut text,
        "\t\t\tdependencies = (\n\t\t\t);\n",
        &format!("\t\t\tdependencies = (\n\t\t\t\t{} /* PBXTargetDependency */,\n\t\t\t);\n", id(DEPENDENCY)),
    )?;

    // Objects added to sections that exist.
    insert_before(&mut text, "/* End PBXBuildFile section */", &build_files(name))?;
    insert_before(&mut text, "/* End PBXFileReference section */", &file_references(name))?;
    insert_before(&mut text, "/* End PBXNativeTarget section */", &native_target(name))?;
    insert_before(&mut text, "/* End PBXSourcesBuildPhase section */", &sources_phase())?;
    insert_before(&mut text, "/* End XCBuildConfiguration section */", &build_configurations(ext))?;
    insert_before(&mut text, "/* End XCConfigurationList section */", &configuration_list(name))?;
    insert_before(&mut text, "/* End PBXGroup section */", &extension_group(name))?;

    // Sections the scaffold has no object for.
    for section in ["PBXContainerItemProxy", "PBXCopyFilesBuildPhase", "PBXTargetDependency"] {
        if text.contains(&format!("/* Begin {section} section */")) {
            return Err(format!("the project already has a {section} section"));
        }
    }
    insert_before(
        &mut text,
        "/* Begin PBXFileReference section */",
        &format!(
            "/* Begin PBXContainerItemProxy section */\n{}/* End PBXContainerItemProxy section */\n\n\
             /* Begin PBXCopyFilesBuildPhase section */\n{}/* End PBXCopyFilesBuildPhase section */\n\n",
            container_item_proxy(name),
            embed_phase(name)
        ),
    )?;
    insert_before(
        &mut text,
        "/* Begin XCBuildConfiguration section */",
        &format!(
            "/* Begin PBXTargetDependency section */\n{}/* End PBXTargetDependency section */\n\n",
            target_dependency(name)
        ),
    )?;

    // Objects the app already has: the extension joins their lists.
    insert_after_line(
        &mut text,
        APP_MAIN_BUILD_FILE,
        &format!(
            "\t\t\t\t{} /* {BRIDGE_FILE} in Sources */,\n\t\t\t\t{} /* {ATTRIBUTES_FILE} in Sources */,\n",
            id(BRIDGE_BUILD),
            id(ATTRIBUTES_BUILD)
        ),
    )?;
    insert_after_line(
        &mut text,
        APP_RESOURCES,
        &format!("\t\t\t\t{} /* Embed Foundation Extensions */,\n", id(EMBED)),
    )?;
    insert_after_line(&mut text, APP_GROUP, &format!("\t\t\t\t{} /* {name} */,\n", id(GROUP)))?;
    insert_after_line(
        &mut text,
        APP_PRODUCT,
        &format!("\t\t\t\t{} /* {name}.appex */,\n", id(PRODUCT)),
    )?;
    insert_after_line(&mut text, APP_TARGET, &format!("\t\t\t\t{} /* {name} */,\n", id(TARGET)))?;
    insert_after_line_of(
        &mut text,
        APP_GROUP,
        "main.swift",
        &format!(
            "\t\t\t\t{} /* {BRIDGE_FILE} */,\n\t\t\t\t{} /* {ATTRIBUTES_FILE} */,\n",
            id(BRIDGE_REF),
            id(ATTRIBUTES_REF)
        ),
    )?;
    Ok(text)
}

fn build_files(name: &str) -> String {
    let (bridge, attributes) = (id(BRIDGE_REF), id(ATTRIBUTES_REF));
    format!(
        "\t\t{} /* {BRIDGE_FILE} in Sources */ = {{isa = PBXBuildFile; fileRef = {bridge} /* {BRIDGE_FILE} */; }};\n\
         \t\t{} /* {ATTRIBUTES_FILE} in Sources */ = {{isa = PBXBuildFile; fileRef = {attributes} /* {ATTRIBUTES_FILE} */; }};\n\
         \t\t{} /* {ATTRIBUTES_FILE} in Sources */ = {{isa = PBXBuildFile; fileRef = {attributes} /* {ATTRIBUTES_FILE} */; }};\n\
         \t\t{} /* {WIDGET_FILE} in Sources */ = {{isa = PBXBuildFile; fileRef = {} /* {WIDGET_FILE} */; }};\n\
         \t\t{} /* {name}.appex in Embed Foundation Extensions */ = {{isa = PBXBuildFile; fileRef = {} /* {name}.appex */; settings = {{ATTRIBUTES = (RemoveHeadersOnCopy, ); }}; }};\n",
        id(BRIDGE_BUILD),
        id(ATTRIBUTES_BUILD),
        id(ATTRIBUTES_EXT_BUILD),
        id(WIDGET_BUILD),
        id(WIDGET_REF),
        id(PRODUCT_BUILD),
        id(PRODUCT),
    )
}

fn file_references(name: &str) -> String {
    format!(
        "\t\t{} /* {BRIDGE_FILE} */ = {{isa = PBXFileReference; fileEncoding = 4; lastKnownFileType = sourcecode.swift; path = {BRIDGE_FILE}; sourceTree = \"<group>\"; }};\n\
         \t\t{} /* {ATTRIBUTES_FILE} */ = {{isa = PBXFileReference; fileEncoding = 4; lastKnownFileType = sourcecode.swift; path = {ATTRIBUTES_FILE}; sourceTree = \"<group>\"; }};\n\
         \t\t{} /* {WIDGET_FILE} */ = {{isa = PBXFileReference; fileEncoding = 4; lastKnownFileType = sourcecode.swift; path = {WIDGET_FILE}; sourceTree = \"<group>\"; }};\n\
         \t\t{} /* {PLIST_FILE} */ = {{isa = PBXFileReference; lastKnownFileType = text.plist.xml; path = {PLIST_FILE}; sourceTree = \"<group>\"; }};\n\
         \t\t{} /* {name}.appex */ = {{isa = PBXFileReference; explicitFileType = \"wrapper.app-extension\"; includeInIndex = 0; path = {name}.appex; sourceTree = BUILT_PRODUCTS_DIR; }};\n",
        id(BRIDGE_REF),
        id(ATTRIBUTES_REF),
        id(WIDGET_REF),
        id(PLIST_REF),
        id(PRODUCT),
    )
}

fn extension_group(name: &str) -> String {
    format!(
        "\t\t{} /* {name} */ = {{\n\t\t\tisa = PBXGroup;\n\t\t\tchildren = (\n\t\t\t\t{} /* {WIDGET_FILE} */,\n\t\t\t\t{} /* {PLIST_FILE} */,\n\t\t\t);\n\t\t\tpath = {name};\n\t\t\tsourceTree = \"<group>\";\n\t\t}};\n",
        id(GROUP),
        id(WIDGET_REF),
        id(PLIST_REF),
    )
}

fn container_item_proxy(name: &str) -> String {
    format!(
        "\t\t{} /* PBXContainerItemProxy */ = {{\n\t\t\tisa = PBXContainerItemProxy;\n\t\t\tcontainerPortal = {PROJECT} /* Project object */;\n\t\t\tproxyType = 1;\n\t\t\tremoteGlobalIDString = {};\n\t\t\tremoteInfo = {name};\n\t\t}};\n",
        id(PROXY),
        id(TARGET),
    )
}

fn embed_phase(name: &str) -> String {
    format!(
        "\t\t{} /* Embed Foundation Extensions */ = {{\n\t\t\tisa = PBXCopyFilesBuildPhase;\n\t\t\tbuildActionMask = 2147483647;\n\t\t\tdstPath = \"\";\n\t\t\tdstSubfolderSpec = 13;\n\t\t\tfiles = (\n\t\t\t\t{} /* {name}.appex in Embed Foundation Extensions */,\n\t\t\t);\n\t\t\tname = \"Embed Foundation Extensions\";\n\t\t\trunOnlyForDeploymentPostprocessing = 0;\n\t\t}};\n",
        id(EMBED),
        id(PRODUCT_BUILD),
    )
}

fn target_dependency(name: &str) -> String {
    format!(
        "\t\t{} /* PBXTargetDependency */ = {{\n\t\t\tisa = PBXTargetDependency;\n\t\t\ttarget = {} /* {name} */;\n\t\t\ttargetProxy = {} /* PBXContainerItemProxy */;\n\t\t}};\n",
        id(DEPENDENCY),
        id(TARGET),
        id(PROXY),
    )
}

fn sources_phase() -> String {
    format!(
        "\t\t{} /* Sources */ = {{\n\t\t\tisa = PBXSourcesBuildPhase;\n\t\t\tbuildActionMask = 2147483647;\n\t\t\tfiles = (\n\t\t\t\t{} /* {WIDGET_FILE} in Sources */,\n\t\t\t\t{} /* {ATTRIBUTES_FILE} in Sources */,\n\t\t\t);\n\t\t\trunOnlyForDeploymentPostprocessing = 0;\n\t\t}};\n",
        id(SOURCES),
        id(WIDGET_BUILD),
        id(ATTRIBUTES_EXT_BUILD),
    )
}

fn native_target(name: &str) -> String {
    format!(
        "\t\t{} /* {name} */ = {{\n\t\t\tisa = PBXNativeTarget;\n\t\t\tbuildConfigurationList = {} /* Build configuration list for PBXNativeTarget \"{name}\" */;\n\t\t\tbuildPhases = (\n\t\t\t\t{} /* Sources */,\n\t\t\t);\n\t\t\tbuildRules = (\n\t\t\t);\n\t\t\tdependencies = (\n\t\t\t);\n\t\t\tname = {name};\n\t\t\tproductName = {name};\n\t\t\tproductReference = {} /* {name}.appex */;\n\t\t\tproductType = \"com.apple.product-type.app-extension\";\n\t\t}};\n",
        id(TARGET),
        id(CONFIGS),
        id(SOURCES),
        id(PRODUCT),
    )
}

fn build_configurations(ext: &Extension) -> String {
    let settings = |config: &str, number: u32| {
        format!(
            "\t\t{} /* {config} */ = {{\n\t\t\tisa = XCBuildConfiguration;\n\t\t\tbuildSettings = {{\n\
             \t\t\t\tAPPLICATION_EXTENSION_API_ONLY = YES;\n\
             \t\t\t\tGENERATE_INFOPLIST_FILE = NO;\n\
             \t\t\t\tINFOPLIST_FILE = \"{name}/{PLIST_FILE}\";\n\
             \t\t\t\tIPHONEOS_DEPLOYMENT_TARGET = 16.2;\n\
             \t\t\t\tLD_RUNPATH_SEARCH_PATHS = \"$(inherited) @executable_path/Frameworks @executable_path/../../Frameworks\";\n\
             \t\t\t\tPRODUCT_BUNDLE_IDENTIFIER = \"{bundle_id}\";\n\
             \t\t\t\tPRODUCT_MODULE_NAME = {module};\n\
             \t\t\t\tSDKROOT = iphoneos;\n\
             \t\t\t\tSKIP_INSTALL = YES;\n\
             \t\t\t\tSWIFT_EMIT_LOC_STRINGS = YES;\n\
             \t\t\t\tTARGETED_DEVICE_FAMILY = \"1,2\";\n\
             \t\t\t\t{wrapper}\n\
             \t\t\t}};\n\t\t\tname = {config};\n\t\t}};\n",
            id(number),
            name = ext.name,
            wrapper = wrapper_setting(ext.name),
            bundle_id = ext.bundle_id,
            module = ext.module,
        )
    };
    settings("Debug", DEBUG) + &settings("Release", RELEASE)
}

fn configuration_list(name: &str) -> String {
    format!(
        "\t\t{} /* Build configuration list for PBXNativeTarget \"{name}\" */ = {{\n\t\t\tisa = XCConfigurationList;\n\t\t\tbuildConfigurations = (\n\t\t\t\t{} /* Release */,\n\t\t\t\t{} /* Debug */,\n\t\t\t);\n\t\t\tdefaultConfigurationIsVisible = 0;\n\t\t\tdefaultConfigurationName = Release;\n\t\t}};\n",
        id(CONFIGS),
        id(RELEASE),
        id(DEBUG),
    )
}

/// Inserts `addition` before `marker`, which must occur exactly once.
fn insert_before(text: &mut String, marker: &str, addition: &str) -> Result<(), String> {
    let at = unique(text, marker)?;
    text.insert_str(at, addition);
    Ok(())
}

/// Replaces `old`, which must occur exactly once, with `new`.
fn replace_once(text: &mut String, old: &str, new: &str) -> Result<(), String> {
    let at = unique(text, old)?;
    text.replace_range(at..at + old.len(), new);
    Ok(())
}

/// Inserts `addition` after the list entry of object `object`: the line that
/// starts with its identifier, four tabs deep. It must be the only one.
fn insert_after_line(text: &mut String, object: &str, addition: &str) -> Result<(), String> {
    let needle = format!("\n\t\t\t\t{object} /* ");
    let at = unique(text, &needle)? + 1;
    let end = text[at..].find('\n').ok_or("unterminated project file")? + at + 1;
    text.insert_str(end, addition);
    Ok(())
}

/// Like [`insert_after_line`], but for the entry of `group`'s child whose
/// comment is `child`: the entries of a group are listed under its identifier,
/// so the group's `children` is searched for the line.
fn insert_after_line_of(
    text: &mut String,
    group: &str,
    child: &str,
    addition: &str,
) -> Result<(), String> {
    let header = format!("\t\t{group} /* ");
    let start = unique(text, &format!("\n{header}"))?;
    let end_of_group = text[start..].find("\t\t};\n").ok_or("unterminated group")? + start;
    let entry = format!("/* {child} */,\n");
    let line_end = text[start..end_of_group]
        .find(&entry)
        .ok_or_else(|| format!("the group {group} has no {child}"))?
        + start
        + entry.len();
    text.insert_str(line_end, addition);
    Ok(())
}

/// The offset of `needle`, which must occur exactly once.
fn unique(text: &str, needle: &str) -> Result<usize, String> {
    let mut found = text.match_indices(needle);
    match (found.next(), found.next()) {
        (Some((at, _)), None) => Ok(at),
        (None, _) => Err(format!("`{}` is missing", needle.trim())),
        (Some(_), Some(_)) => Err(format!("`{}` is not unique", needle.trim())),
    }
}
