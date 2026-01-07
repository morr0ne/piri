use std::cell::LazyCell;

use anyhow::{Result, bail};
use niri_ipc::socket::Socket;
use niri_ipc::{Action, Event, Request, Response, Window, WorkspaceReferenceArg};
use regex::Regex;
use sap::{Argument, Parser};
use tracing::{debug, info};
use tracing_subscriber::filter::LevelFilter;

const TITLE_REGEX: LazyCell<Regex> =
    LazyCell::new(|| Regex::new(r"^Picture-in-Picture$").expect("Invalid regex"));

const APP_ID_REGEX: LazyCell<Regex> =
    LazyCell::new(|| Regex::new(r"firefox$").expect("Invalid regex"));

const VERSION_TEXT: &str = "piri 0.1.0\n";

const HELP_TEXT: &str = "piri - Make Firefox Picture-in-Picture windows persist across workspaces

USAGE:
    piri [OPTIONS]

OPTIONS:
    -l, --log-level <LEVEL>    Set the log level [default: info]
                               Possible values: trace, debug, info, warn, error
    -h, --help                 Print this help message
    -v, --version              Print version information
";

fn main() -> Result<()> {
    let mut parser = Parser::from_arbitrary(std::env::args())?;
    let mut level_filter = LevelFilter::INFO;

    while let Some(arg) = parser.forward()? {
        match arg {
            Argument::Short('l') | Argument::Long("log-level") => {
                if let Some(level) = parser.value() {
                    level_filter = match level.as_str() {
                        "trace" => LevelFilter::TRACE,
                        "debug" => LevelFilter::DEBUG,
                        "info" => LevelFilter::INFO,
                        "warn" => LevelFilter::WARN,
                        "error" => LevelFilter::ERROR,
                        _ => {
                            bail!("Invalid log level: {level}.");
                        }
                    };

                    continue;
                }

                bail!("A value must be provided for log-level");
            }
            Argument::Short('h') | Argument::Long("help") => {
                print!("{HELP_TEXT}");
                return Ok(());
            }
            Argument::Short('v') | Argument::Long("version") => {
                print!("{VERSION_TEXT}");
                return Ok(());
            }
            arg => return Err(arg.into_error(None).into()),
        }
    }

    tracing_subscriber::fmt()
        .with_max_level(level_filter)
        .init();

    let mut events_socket = Socket::connect()?;
    let mut requests_socket = Socket::connect()?;

    let mut pip_window = None;

    if matches!(
        events_socket.send(Request::EventStream)?,
        Ok(Response::Handled)
    ) {
        info!("Trying to fetch existing windows...");
        if let Ok(Response::Windows(windows)) = requests_socket.send(Request::Windows)? {
            for window in windows {
                if window_matches(&window) {
                    info!("Found a matching window with id {}", window.id);
                    pip_window = Some(window.id);
                    break;
                }

                debug!(
                    "Ignoring window \"{}\"",
                    window.title.unwrap_or(window.id.to_string())
                )
            }
        }

        let mut read_event = events_socket.read_events();

        info!("Starting read of events");

        while let Ok(event) = read_event() {
            match event {
                Event::WorkspaceActivated { id, focused } => {
                    if focused && let Some(window) = pip_window {
                        info!("Workspace {} focused. Moving window {}", id, window);

                        let _ = requests_socket.send(Request::Action(
                            Action::MoveWindowToWorkspace {
                                window_id: Some(window),
                                reference: WorkspaceReferenceArg::Id(id),
                                focus: false,
                            },
                        ))?;
                    } else {
                        debug!("Workspace {} focused but no window was detected", id);
                    }
                }
                Event::WindowOpenedOrChanged { ref window } => {
                    if window_matches(window) && pip_window != Some(window.id) {
                        info!("Window {} matched regexs", window.id);
                        pip_window = Some(window.id);
                    }
                }
                Event::WindowClosed { id } => {
                    if let Some(window) = pip_window
                        && window == id
                    {
                        info!("Window {} got closed", window);

                        pip_window = None
                    }
                }
                _ => (),
            }
        }
    }

    Ok(())
}

fn window_matches(window: &Window) -> bool {
    let app_id_matches = if let Some(ref app_id) = window.app_id {
        APP_ID_REGEX.is_match(app_id)
    } else {
        true
    };

    if let Some(ref title) = window.title {
        return TITLE_REGEX.is_match(title) && app_id_matches;
    }

    false
}
