//! Interactive questions shared by `aimer create` and `aimer init`.

use inquire::ui::{Color, RenderConfig, Styled};
use inquire::{Confirm, MultiSelect, Text};

use super::validate_app_name_input;

macro_rules! prompt_abortable {
    ($prompt:expr) => {
        loop {
            match $prompt.prompt() {
                Ok(val) => break val,
                Err(inquire::error::InquireError::OperationInterrupted) => {
                    println!("press 'ctrl + c' again to exit");
                    crossterm::terminal::enable_raw_mode().unwrap();
                    if let Ok(crossterm::event::Event::Key(event)) = crossterm::event::read() {
                        if event.code == crossterm::event::KeyCode::Char('c')
                            && event
                                .modifiers
                                .contains(crossterm::event::KeyModifiers::CONTROL)
                        {
                            crossterm::terminal::disable_raw_mode().unwrap();
                            std::process::exit(1);
                        }
                    }
                    crossterm::terminal::disable_raw_mode().unwrap();
                }
                Err(inquire::error::InquireError::OperationCanceled) => {
                    std::process::exit(1);
                }
                Err(e) => {
                    eprintln!("Error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    };
}

/// Values the prompts offer as their pre-filled answers.
pub(crate) struct Defaults<'a> {
    pub app_name: &'a str,
    pub version: &'a str,
    pub description: &'a str,
    pub author: &'a str,
}

/// What the user answered.
pub(crate) struct Answers {
    pub app_name: String,
    pub version: String,
    pub description: String,
    pub author: String,
    pub group: String,
    pub targets: Vec<&'static str>,
}

/// Apply the shared look of the project prompts.
pub(crate) fn install_render_config() {
    let config = RenderConfig {
        prompt_prefix: Styled::new("◆").with_fg(Color::LightCyan),
        highlighted_option_prefix: Styled::new("│  ❯").with_fg(Color::LightCyan),
        unhighlighted_option_prefix: Styled::new("│   ").with_fg(Color::DarkGrey),
        selected_checkbox: Styled::new("●").with_fg(Color::LightGreen),
        unselected_checkbox: Styled::new("○").with_fg(Color::DarkGrey),
        ..RenderConfig::default()
    };
    inquire::set_global_render_config(config);
}

/// Ask for the application metadata and the platforms to scaffold.
pub(crate) fn collect(defaults: &Defaults<'_>) -> Answers {
    let app_name = prompt_abortable!(
        Text::new("App name:")
            .with_default(defaults.app_name)
            .with_validator(validate_app_name_input)
    );
    let description = prompt_abortable!(with_optional_default(
        Text::new("Description:"),
        defaults.description
    ));
    let version = prompt_abortable!(Text::new("Version:").with_default(defaults.version));
    let author = prompt_abortable!(with_optional_default(Text::new("Author:"), defaults.author));
    let group = prompt_abortable!(
        Text::new("Group (e.g. com.example.app):").with_default("com.example.app")
    );
    let targets = prompt_abortable!(MultiSelect::new(
        "Targets:",
        vec!["macos", "windows", "linux", "android", "ios", "web"]
    ));

    Answers {
        app_name: app_name.trim().to_string(),
        version,
        description,
        author,
        group,
        targets,
    }
}

/// Print the collected configuration and ask for confirmation.
pub(crate) fn confirm(project_name: &str, answers: &Answers) -> bool {
    println!(
        "\nProject config:\n- Name: {}\n- App name: {}\n- Version: {}\n- Description: {}\n- Author: {}\n- Group: {}\n- Targets: {:?}",
        project_name,
        answers.app_name,
        answers.version,
        answers.description,
        answers.author,
        answers.group,
        answers.targets
    );
    prompt_abortable!(Confirm::new("Is this okay?").with_default(true))
}

/// An empty default would render as an empty `()` hint, so only set it when
/// there is something to offer.
fn with_optional_default<'a, 'b>(prompt: Text<'a, 'b>, default: &'a str) -> Text<'a, 'b> {
    if default.is_empty() {
        prompt
    } else {
        prompt.with_default(default)
    }
}
