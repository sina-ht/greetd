mod config;
mod context;
mod error;
mod pam;
mod scrambler;
mod server;
mod session;
mod terminal;

use std::os::unix::{
    io::{FromRawFd, RawFd},
    net::UnixDatagram,
};

use nix::{
    fcntl::{fcntl, FcntlArg, FdFlag},
    sys::mman::{mlockall, MlockAllFlags},
};
use tokio::task;

use crate::{error::Error, session::worker};

async fn session_worker_main(config: config::Config) -> Result<(), Error> {
    let raw_fd = config.internal.session_worker as RawFd;
    let mut cur_flags = FdFlag::from_bits_retain(fcntl(raw_fd, FcntlArg::F_GETFD)?);
    cur_flags.insert(FdFlag::FD_CLOEXEC);
    fcntl(raw_fd, FcntlArg::F_SETFD(cur_flags))?;
    let sock = unsafe { UnixDatagram::from_raw_fd(raw_fd) };
    worker::main(&sock)
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let config = match config::read_config() {
        Ok(config) => config,
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    };
    if cfg!(feature = "debug") {
        eprintln!("config: {:?}", config);
    }
    // The session worker must be able to make MAP_LOCKED allocations as the
    // unprivileged target user (e.g. via pam modules such as pam_fscrypt).
    // As root, mlockall() would succeed despite exceeding RLIMIT_MEMLOCK, but
    // the kernel then refuses further MAP_LOCKED allocations from the dropped
    // privileges as mm->locked_vm already covers the entire address space.
    if config.internal.session_worker == 0 {
        mlockall(MlockAllFlags::all()).expect("unable to lock pages");
    }
    let res = task::LocalSet::new()
        .run_until(async move {
            if config.internal.session_worker > 0 {
                session_worker_main(config).await
            } else {
                server::main(config).await
            }
        })
        .await;
    if let Err(e) = res {
        eprintln!("error: {}", e);
    }
}
