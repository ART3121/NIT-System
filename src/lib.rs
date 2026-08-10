mod ai;
mod cli;
mod commands;
mod editor;
mod ids;
mod model;
mod storage;
mod tui;
mod workspace;

use anyhow::Result;

pub fn run() -> Result<()> {
    cli::run()
}
