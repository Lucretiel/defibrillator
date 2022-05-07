mod duration;
mod rules;
mod task;

use std::{
    error::Error,
    future::Future,
    io,
    pin::Pin,
    process::{exit, Stdio},
    task::{Context, Poll},
    time::Duration,
};

use broadcast::error::RecvError;
use bytes::{Bytes, BytesMut};
use futures::{
    future::{join, pending, select, Either, FutureExt},
    pin_mut, select_biased,
    stream::TryStreamExt,
};
use memchr::memchr;
use reqwest::Client;
use rules::OrRules;
use structopt::StructOpt;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
    sync::broadcast::{self, Sender},
    task::JoinHandle,
    time::{sleep_until, Instant},
};
use tracing::{event, span, Instrument, Level};
use tracing_subscriber::{EnvFilter, FmtSubscriber};

use crate::duration::Duration as ParsableDuration;
use crate::task::ScopedTask;

#[derive(StructOpt)]
struct Args {
    /// The set of rules that determine when the server process is ready
    #[structopt(short, long)]
    rules: OrRules,

    /// The maximum time to wait for a server process to become ready
    #[structopt(short = "t", long)]
    ready_timeout: Option<ParsableDuration>,

    /// The maximum number of times to re-launch a crashed server if it never
    /// becomes ready
    #[structopt(short = "R", long)]
    retries: Option<u64>,

    /// The command to run
    command: Vec<String>,

    /// Filter directives to pass to the logger
    #[structopt(short, long)]
    log_filters: Option<String>,
}

#[tokio::main]
#[tracing::instrument]
async fn main() {
    let args: Args = Args::from_args();

    FmtSubscriber::builder()
        .with_env_filter(
            EnvFilter::try_new(args.log_filters.as_deref().unwrap_or(""))
                .expect("Failed to create env filter"),
        )
        .init();

    let client = match Client::builder()
        .user_agent(concat!(
            env!("CARGO_PKG_NAME"),
            "/",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            let err: &dyn Error = &err;
            event!(Level::ERROR, error = err, "Failed to create an HTTP client");
            exit(1);
        }
    };

    // Unwrap safety: Structopt enforces at least one argument here
    let program = &args.command[0];
    let program_args = &args.command[1..];

    let mut command_builder = Command::new(program);

    command_builder
        .args(program_args)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .stdout(Stdio::piped())
        .kill_on_drop(true);

    let mut attempts: u64 = 0;

    loop {
        let outcome = async {
            event!(Level::INFO, attempt = attempts + 1);
            run_server(
                &mut command_builder,
                &args.rules,
                args.ready_timeout.map(|duration| duration.get()),
                &client,
            )
            .await
        }
        .instrument(span!(Level::INFO, "running command"))
        .await;

        let _err = match outcome {
            RunServerOutcome::DidntSpawn(err) => {
                attempts += 1;
                Some(err)
            }
            RunServerOutcome::ExitedWhileStarting => {
                attempts += 1;
                None
            }
            RunServerOutcome::TimedOutWhileStarting => {
                attempts += 1;
                None
            }
            RunServerOutcome::ExitedWhileReady => {
                attempts = 0;
                None
            }
        };

        if let Some(retries) = args.retries {
            if attempts >= retries {
                event!(Level::ERROR, attempts = attempts, "command failed to start");
                return;
            }
        }
    }
}

enum RunServerOutcome {
    DidntSpawn(io::Error),
    ExitedWhileStarting,
    TimedOutWhileStarting,
    ExitedWhileReady,
}

pub async fn handle_stdout<T: Unpin + AsyncRead>(
    mut pipe: T,
    broadcast: Sender<Bytes>,
) -> io::Result<()> {
    let mut lines = broadcast.subscribe();

    let stdout_task = async move {
        let mut stdout = tokio::io::stdout();

        loop {
            match lines.recv().await {
                Ok(mut line) => stdout.write_all_buf(&mut line).await?,
                Err(err) => match err {
                    RecvError::Closed => return stdout.flush().await,

                    // TODO: log here
                    RecvError::Lagged(_) => {}
                },
            }
        }
    };

    let read_task = async move {
        let mut buffer = BytesMut::with_capacity(4096);
        let mut count: usize = 0;

        loop {
            let n = match pipe.read_buf(&mut buffer).await {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(err) => return Err(err),
            };

            if buffer.capacity() == 0 {
                buffer.reserve(4096);
            }

            let new_bytes = &buffer[count..n];
            count += n;

            let line_length = match memchr(b'\n', new_bytes) {
                None => continue,
                Some(idx) => count - n + idx + 1,
            };

            let line = buffer.split_to(line_length).freeze();
            count -= line.len();
            broadcast.send(line);
        }
    };

    let (read_result, stdout_result) = join(read_task, stdout_task).await;

    read_result?;
    stdout_result?;

    Ok(())
}

/// Run a single instance of the server, managing its lifecycle
#[tracing::instrument]
async fn run_server(
    builder: &mut Command,
    rules: &OrRules,
    starting_timeout: Option<Duration>,
    client: &Client,
) -> RunServerOutcome {
    let (stdout_task, mut child) = {
        let log_lines = {
            let (log_lines, _) = broadcast::channel(100);
            log_lines
        };

        let rules = rules
            .build(client, &log_lines)
            .wait()
            .instrument(span!(Level::TRACE, "rules"))
            .fuse();
        pin_mut!(rules);

        event!(Level::INFO, "spawning command");

        let mut child = match builder.spawn() {
            Ok(child) => child,
            Err(err) => {
                let dyn_err: &dyn Error = &err;
                event!(Level::ERROR, error = dyn_err, "command failed to spawn");
                return RunServerOutcome::DidntSpawn(err);
            }
        };

        // TODO: Create signal handlers here to kill the child if we get a sigkill, sighup, etc

        let child_stdout = child.stdout.take().unwrap();

        let stdout_task = ScopedTask::new(tokio::spawn(handle_stdout(child_stdout, log_lines)));

        let starting_timeout = match starting_timeout {
            Some(duration) => Either::Left(sleep_until(Instant::now() + duration).fuse()),
            None => Either::Right(pending()),
        };
        pin_mut!(starting_timeout);

        // State is now starting. Wait for the rules to signal readiness, or for
        // a timeout
        select_biased! {
            () = rules => {},
            _res = child.wait().fuse() => {
                // Server exited cleanly; finish forwarding stdout
                stdout_task.await;

                return RunServerOutcome::ExitedWhileStarting;
            },
            () = starting_timeout => {
                // Server timeed out; kill it and finish stdout
                child.kill().await;
                stdout_task.await;

                return RunServerOutcome::TimedOutWhileStarting;
            }
        };

        (stdout_task, child)
    };

    event!(Level::INFO, "server is now ready");

    // State is now started!
    let res = child.wait().await;

    // Child exited cleanly; finish forwarding stdout
    stdout_task.await;

    RunServerOutcome::ExitedWhileReady
}
