//! `slate`: launch a GPUI app hidden and screenshot or drive it, without its
//! window ever showing or taking focus. The app must call the `slate` crate.

mod drive;
mod shoot;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};
use slate::Size;

#[derive(Debug, Parser)]
#[command(
    name = "slate",
    version,
    about = "Screenshot and drive a GPUI app without its window showing or taking focus"
)]
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
    /// Launch the app hidden, play a steps file in it (keys, typing, clicks,
    /// scrolls, resizes, shots), save each shot as <out>/<name>.png in points
    /// and quit the app.
    ///
    /// Example: slate drive --steps open-palette.json --out shots -- target/debug/my-app
    Drive(DriveArgs),
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

#[derive(Debug, clap::Args)]
pub struct DriveArgs {
    /// The steps file: a JSON list such as [{"click": {"x": 40, "y": 60}}, {"type": "hi"}, {"key": "cmd-k"},
    /// {"shot": "palette"}]. See the README for every step.
    #[arg(long)]
    pub steps: PathBuf,
    /// The folder for the shots, made if missing.
    #[arg(long)]
    pub out: PathBuf,
    /// The window size in points, WIDTHxHEIGHT. The app's own size when left out.
    #[arg(long, value_parser = Size::parse)]
    pub size: Option<Size>,
    /// Milliseconds from opening the window to the first step, for the app to load.
    #[arg(long, default_value_t = 1000)]
    pub wait: u64,
    /// Give up when the steps haven't finished this many milliseconds after the wait and their own waits.
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
        Command::Drive(args) => drive::run(&args),
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

    fn command(args: &[&str]) -> Result<Command, clap::Error> {
        Ok(Cli::try_parse_from(std::iter::once("slate").chain(args.iter().copied()))?.command)
    }

    fn parse(args: &[&str]) -> Result<ShootArgs, clap::Error> {
        match command(args)? {
            Command::Shoot(args) => Ok(args),
            other => panic!("not a shoot: {other:?}"),
        }
    }

    fn parse_drive(args: &[&str]) -> Result<DriveArgs, clap::Error> {
        match command(args)? {
            Command::Drive(args) => Ok(args),
            other => panic!("not a drive: {other:?}"),
        }
    }

    #[test]
    fn drive_reads_its_steps_out_folder_and_app() {
        let args =
            parse_drive(&["drive", "--steps", "s.json", "--out", "shots", "--size", "800x500", "--", "app", "-v"])
                .unwrap();
        assert_eq!(args.steps, PathBuf::from("s.json"));
        assert_eq!(args.out, PathBuf::from("shots"));
        assert_eq!(args.size, Some(Size { width: 800., height: 500. }));
        assert_eq!((args.wait, args.timeout), (1000, 30_000));
        assert_eq!(args.app, ["app", "-v"]);
    }

    #[test]
    fn drive_needs_steps_an_out_folder_and_an_app() {
        assert!(parse_drive(&["drive", "--out", "o", "--", "app"]).is_err());
        assert!(parse_drive(&["drive", "--steps", "s.json", "--", "app"]).is_err());
        assert!(parse_drive(&["drive", "--steps", "s.json", "--out", "o"]).is_err());
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
