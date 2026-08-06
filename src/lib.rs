mod cli;
mod commands;
mod editor;
mod model;
mod storage;
mod tui;

use anyhow::Result;

pub fn run() -> Result<()> {
    cli::run()
}
