//! `slate`: launch a GPUI app hidden and screenshot it, without its window
//! ever showing or taking focus. The app must call the `slate` crate.

mod shoot;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use slate::Size;

#[derive(Debug, Parser)]
#[command(name = "slate", version, about = "Screenshot a GPUI app without its window showing or taking focus")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Launch the app hidden, capture its window, scale it to points and quit the app.
    ///
    /// Example: slate shoot --out shot.png --size 1280x820 -- target/debug/my-app --sample /tmp/s
    Shoot(ShootArgs),
}

#[derive(Debug, clap::Args)]
pub struct ShootArgs {
    /// Where to write the PNG (in points: a 1280x820 window gives a 1280x820 image).
    #[arg(long)]
    pub out: PathBuf,
    /// The window size in points, WIDTHxHEIGHT. The app's own size when left out.
    #[arg(long, value_parser = Size::parse)]
    pub size: Option<Size>,
    /// Milliseconds from opening the window to the shot, for the app to load.
    #[arg(long, default_value_t = 1000)]
    pub wait: u64,
    /// Give up when there's no shot this many milliseconds after the wait.
    #[arg(long, default_value_t = 30_000)]
    pub timeout: u64,
    /// The app and its arguments, after `--`.
    #[arg(last = true, required = true, value_name = "APP")]
    pub app: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Shoot(args) => shoot::run(&args),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problem) => {
            eprintln!("slate: {problem}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<ShootArgs, clap::Error> {
        let cli = Cli::try_parse_from(std::iter::once("slate").chain(args.iter().copied()))?;
        let Command::Shoot(args) = cli.command;
        Ok(args)
    }

    #[test]
    fn shoot_reads_its_options_and_the_app_after_the_dashes() {
        let args = parse(&[
            "shoot",
            "--out",
            "a.png",
            "--size",
            "1440x900",
            "--wait",
            "2500",
            "--",
            "kaepora-mac",
            "--sample",
            "/tmp/s",
        ])
        .unwrap();
        assert_eq!(args.out, PathBuf::from("a.png"));
        assert_eq!(args.size, Some(Size { width: 1440., height: 900. }));
        assert_eq!(args.wait, 2500);
        assert_eq!(args.app, ["kaepora-mac", "--sample", "/tmp/s"]);
    }

    #[test]
    fn shoot_defaults() {
        let args = parse(&["shoot", "--out", "a.png", "--", "app"]).unwrap();
        assert_eq!(args.size, None);
        assert_eq!(args.wait, 1000);
        assert_eq!(args.timeout, 30_000);
    }

    #[test]
    fn shoot_needs_an_app_and_a_good_size() {
        assert!(parse(&["shoot", "--out", "a.png"]).is_err());
        assert!(parse(&["shoot", "--out", "a.png", "--size", "big", "--", "app"]).is_err());
        assert!(parse(&["shoot", "--", "app"]).is_err(), "--out is required");
    }
}
