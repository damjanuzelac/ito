use clap::Parser;
use std::process;
use std::thread;
use std::time::Duration;

use text_writer::type_text;

#[derive(Parser)]
#[command(name = "text-writer")]
#[command(about = "A cross-platform text typing utility")]
#[command(version = "0.1.0")]
struct Args {
    #[arg(help = "Text to type")]
    text: String,

    #[arg(
        short,
        long,
        default_value_t = 0,
        help = "Delay before typing (milliseconds)"
    )]
    delay: u64,

    #[arg(
        short,
        long,
        default_value_t = 0,
        help = "Delay between characters (milliseconds)"
    )]
    char_delay: u64,
}

fn main() {
    let args = Args::parse();

    if args.text.is_empty() {
        eprintln!("Error: Text cannot be empty");
        process::exit(1);
    }

    if args.delay > 0 {
        thread::sleep(Duration::from_millis(args.delay));
    }

    if let Err(e) = type_text(&args.text, args.char_delay) {
        eprintln!("Error typing text: {}", e);
        process::exit(1);
    }
}
