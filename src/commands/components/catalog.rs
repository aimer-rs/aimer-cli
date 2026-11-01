/// How a component gets onto the machine.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Source {
    /// Built from a crates.io package with `cargo install`.
    Cargo {
        /// The package `cargo install` builds.
        package: &'static str,
        /// The executable the package puts in the managed `bin` directory.
        binary: &'static str,
    },
    /// The Android NDK, downloaded from Google into `~/.aimer/ndk`.
    Ndk,
    /// A .NET global tool, installed with `dotnet tool install --tool-path`.
    Dotnet {
        /// The NuGet package `dotnet tool install` fetches.
        package: &'static str,
        /// The executable the package puts in the managed `bin` directory.
        binary: &'static str,
    },
    /// `appimagetool`, downloaded from its GitHub release into `bin`.
    AppImageTool,
    /// Inno Setup, installed silently into `~/.aimer/innosetup`.
    InnoSetup,
}

/// A tool Aimer can install into its own managed directory.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Component {
    /// Name used on the command line, e.g. `aimer components add trunk`.
    pub name: &'static str,
    /// Where the component comes from.
    pub source: Source,
    /// What Aimer uses the tool for.
    pub purpose: &'static str,
    /// The only operating system (`std::env::consts::OS`) it installs on.
    pub only_on: Option<&'static str>,
}

impl Component {
    /// Whether this component can be installed on the machine Aimer runs on.
    pub(crate) fn supported_here(&self) -> bool {
        self.only_on.is_none_or(|os| os == std::env::consts::OS)
    }
}

/// Every component `aimer components` knows how to manage.
pub(crate) const COMPONENTS: &[Component] = &[
    Component {
        name: "ndk",
        source: Source::Ndk,
        purpose: "Android NDK for building Android targets",
        only_on: None,
    },
    Component {
        name: "trunk",
        source: Source::Cargo {
            package: "trunk",
            binary: "trunk",
        },
        purpose: "Web (wasm) dev server & bundler",
        only_on: None,
    },
    Component {
        name: "wasm-bindgen",
        source: Source::Cargo {
            package: "wasm-bindgen-cli",
            binary: "wasm-bindgen",
        },
        purpose: "Web (wasm) JavaScript bindings generator",
        only_on: None,
    },
    Component {
        name: "wasm-pack",
        source: Source::Cargo {
            package: "wasm-pack",
            binary: "wasm-pack",
        },
        purpose: "Web (wasm) package builder",
        only_on: None,
    },
    Component {
        name: "appimagetool",
        source: Source::AppImageTool,
        purpose: "Linux .AppImage generator for `aimer package linux`",
        only_on: Some("linux"),
    },
    Component {
        name: "inno-setup",
        source: Source::InnoSetup,
        purpose: "Windows setup.exe generator for `aimer package windows`",
        only_on: Some("windows"),
    },
    Component {
        name: "wix",
        source: Source::Dotnet {
            package: "wix",
            binary: "wix",
        },
        purpose: "Windows .msi generator for `aimer package windows -f msi`",
        only_on: Some("windows"),
    },
];

/// The component called `name`, if there is one.
pub(crate) fn find(name: &str) -> Option<&'static Component> {
    COMPONENTS.iter().find(|component| component.name == name)
}

/// The component names, for error messages.
pub(crate) fn names() -> String {
    COMPONENTS
        .iter()
        .map(|component| component.name)
        .collect::<Vec<_>>()
        .join(", ")
}
