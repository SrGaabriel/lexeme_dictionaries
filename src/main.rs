mod build;
mod catalog;
mod manifest;

use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(about = "Build lexeme dictionaries from Wiktextract dumps")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Build(build::Args),
    Matrix(MatrixArgs),
    Manifest(ManifestArgs),
    Url(UrlArgs),
}

#[derive(clap::Args)]
struct MatrixArgs {
    #[arg(long, default_value = "languages.toml")]
    languages: PathBuf,
    #[arg(long, default_value = "")]
    only: String,
}

#[derive(clap::Args)]
struct ManifestArgs {
    #[arg(long)]
    dist: PathBuf,
    #[arg(long)]
    tag: String,
    #[arg(long)]
    repo: String,
    #[arg(long)]
    generated: String,
}

#[derive(clap::Args)]
struct UrlArgs {
    #[arg(long)]
    name: String,
}

fn main() {
    if let Err(err) = run() {
        eprintln!("lexeme-dictionaries: {err}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    match Cli::parse().command {
        Command::Build(args) => build::run(&args),
        Command::Matrix(args) => {
            let languages = catalog::select(catalog::load(&args.languages)?, &args.only)?;
            let matrix = catalog::Matrix {
                include: languages,
            };
            let json = serde_json::to_string(&matrix).map_err(io::Error::other)?;
            println!("{json}");
            Ok(())
        }
        Command::Manifest(args) => {
            manifest::assemble(&args.dist, &args.tag, &args.repo, &args.generated)
        }
        Command::Url(args) => {
            println!("{}", catalog::source_url(&args.name));
            Ok(())
        }
    }
}
