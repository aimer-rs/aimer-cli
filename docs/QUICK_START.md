[← Back to README](../README.md)

# Quick Start

## 1. Create a New Project

Run `aimer create` followed by your desired project name:

```bash
aimer create my_aimer_app
```

The interactive wizard will prompt you for:

- **App name**: Display name of your application, stored as `package.name` in `Aimer.toml` and used as the bundled app
  name (defaults to the project name)
- **Description**: Short description
- **Version**: Initial version (default: `0.1.0`)
- **Author**: Your name or team
- **Group**: Unique reverse domain identifier (e.g. `com.example.app`)
- **Targets**: Select which platform scaffolds to generate (`macos`, `windows`, `linux`, `android`, `ios`, `web`)

## 2. Project Layout

The generated project structure looks like this:

```text
my_aimer_app/
├── Aimer.toml           # Aimer framework manifest & asset declarations
├── Cargo.toml           # Rust package configuration
├── README.md            # App documentation
├── src/
│   └── main.rs          # Application entry point & UI widget definitions
└── builds/              # Platform scaffolds and build output directories
    ├── macos/
    ├── ios/
    ├── android/
    ├── linux/
    ├── windows/
    └── web/
```

## 3. Run the Application

Enter your project directory:

```bash
cd my_aimer_app
```

Launch the interactive device picker:

```bash
aimer run
```

Or target a specific platform directly:

```bash
# macOS desktop
aimer run -t macos

# Web browser (launches local web server via Trunk)
aimer run -t web

# Android device or emulator
aimer run -t android

# iOS simulator
aimer run -t ios
```

See [Command Reference](COMMANDS.md) for every available `aimer` subcommand and option.
