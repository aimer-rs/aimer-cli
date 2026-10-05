#![feature(rustc_private)]

extern crate rustc_driver as _;

use clap::{Command, ValueEnum};
use ra_ap_proc_macro_api::ProtocolFormat;

fn main() -> std::io::Result<()> {
    let matches = Command::new("rust-analyzer-proc-macro-srv")
        .arg(
            clap::Arg::new("format")
                .long("format")
                .action(clap::ArgAction::Set)
                .default_value("json-legacy")
                .value_parser(clap::builder::EnumValueParser::<ProtocolFormatArg>::new()),
        )
        .arg(
            clap::Arg::new("version")
                .long("version")
                .action(clap::ArgAction::SetTrue),
        )
        .get_matches();
    if matches.get_flag("version") {
        println!("rust-analyzer-proc-macro-srv (Aimer RA release 03fcb772)");
        return Ok(());
    }
    if std::env::var_os("RUST_ANALYZER_INTERNALS_DO_NOT_USE").is_none() {
        eprintln!("This helper is an internal rust-analyzer implementation detail.");
        std::process::exit(122);
    }
    let format = matches
        .get_one::<ProtocolFormatArg>("format")
        .expect("format value should always be present")
        .0;

    let mut stdin = std::io::BufReader::new(std::io::stdin());
    let mut stdout = std::io::stdout();
    ra_ap_proc_macro_srv_cli::main_loop::run(&mut stdin, &mut stdout, format)
}

#[derive(Copy, Clone)]
struct ProtocolFormatArg(ProtocolFormat);

impl ValueEnum for ProtocolFormatArg {
    fn value_variants<'a>() -> &'a [Self] {
        &[
            Self(ProtocolFormat::JsonLegacy),
            Self(ProtocolFormat::BidirectionalPostcardPrototype),
        ]
    }

    fn to_possible_value(&self) -> Option<clap::builder::PossibleValue> {
        match self.0 {
            ProtocolFormat::JsonLegacy => Some(clap::builder::PossibleValue::new("json-legacy")),
            ProtocolFormat::BidirectionalPostcardPrototype => Some(
                clap::builder::PossibleValue::new("bidirectional-postcard-prototype"),
            ),
        }
    }

    fn from_str(input: &str, _ignore_case: bool) -> Result<Self, String> {
        match input {
            "json-legacy" => Ok(Self(ProtocolFormat::JsonLegacy)),
            "bidirectional-postcard-prototype" => {
                Ok(Self(ProtocolFormat::BidirectionalPostcardPrototype))
            }
            _ => Err(format!("unknown protocol format: {input}")),
        }
    }
}
