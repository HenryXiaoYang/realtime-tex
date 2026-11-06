//! The channel between the host and the paragraph server. Requests go to the server, framed
//! responses (u32-LE length, then the body) come back on a channel of their own because the
//! server's stdout is not clean (the banner is printed even in batch mode).
//!
//! - Unix: requests on the server's stdin, responses on a FIFO named by `$RTEX_RESP`.
//! - Windows: two named pipes, `$RTEX_REQ` (requests) and `$RTEX_RESP` (responses). The server
//!   opens the request pipe in binary mode: its stdin would be in text mode, where CR LF in a
//!   source becomes LF (the frame's byte count no longer matches) and Ctrl-Z ends the input.

use anyhow::{bail, Context, Result};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, Command};
use std::time::Duration;

/// A frame body read from the response channel (everything after the `u32` length), or the
/// read error that ended the stream.
pub(crate) type Frame = std::io::Result<Vec<u8>>;

/// Largest frame the reader accepts (a corrupt length must not allocate gigabytes).
const MAX_FRAME: usize = 256 << 20;

/// The connected channel: the request writer, the response frames, and the first frame when
/// connecting already read it.
pub(crate) struct Connected {
    pub requests: File,
    pub frames: Frames,
    pub first: Option<Frame>,
}

pub(crate) use crossbeam_channel::{RecvTimeoutError, TryRecvError};

/// Where response frames come from: a reader thread that drains the channel (macOS, Windows),
/// or on Linux the FIFO itself, read by the thread that waits for the reply. Linux's `poll`
/// reports a FIFO that becomes readable while it waits (macOS's does not, see `spawn_reader`)
/// and the FIFO is given room for 1 MB, so the server still finishes writing while the host
/// is not reading; the hand-off through a reader thread cost a thread wake-up per result.
pub(crate) enum Frames {
    #[cfg_attr(target_os = "linux", allow(dead_code))]
    Thread(crossbeam_channel::Receiver<Frame>),
    #[cfg(target_os = "linux")]
    Direct(direct::Reader),
}

impl Frames {
    pub fn try_recv(&mut self) -> Result<Frame, TryRecvError> {
        match self {
            Frames::Thread(rx) => rx.try_recv(),
            #[cfg(target_os = "linux")]
            Frames::Direct(r) => r.try_recv(),
        }
    }

    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Frame, RecvTimeoutError> {
        match self {
            Frames::Thread(rx) => rx.recv_timeout(timeout),
            #[cfg(target_os = "linux")]
            Frames::Direct(r) => r.recv_timeout(timeout),
        }
    }
}

#[cfg(target_os = "linux")]
mod direct {
    use super::*;
    use std::os::fd::AsFd;

    /// The FIFO's read end in non-blocking mode, with the bytes of a frame not complete yet.
    pub(crate) struct Reader {
        f: File,
        buf: Vec<u8>,
        chunk: Box<[u8]>,
        /// The stream ended (end of file, a read error, a corrupt length): the error is
        /// reported once, then the reader is disconnected (like the reader thread's channel).
        ended: Option<Option<std::io::Error>>,
    }

    impl Reader {
        pub fn new(f: File) -> Reader {
            // room for a result of 1 MB (the default limit for unprivileged processes) in the
            // FIFO, so the server finishes writing it even while the host is not reading yet
            // (64 KB otherwise; when this fails, a larger result waits for the next read)
            let fd = std::os::fd::AsRawFd::as_raw_fd(&f);
            let _ = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_SETPIPE_SZ(1 << 20));
            Reader {
                f,
                buf: Vec::with_capacity(1 << 16),
                chunk: vec![0; 1 << 16].into_boxed_slice(),
                ended: None,
            }
        }

        /// The first whole frame in `buf`, split off.
        fn take_frame(&mut self) -> Option<Frame> {
            let n = u32::from_le_bytes(self.buf.get(..4)?.try_into().unwrap()) as usize;
            if n > MAX_FRAME {
                self.ended = Some(None);
                return Some(Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("response frame of {n} bytes"),
                )));
            }
            if self.buf.len() < 4 + n {
                return None;
            }
            let body = self.buf[4..4 + n].to_vec();
            self.buf.drain(..4 + n);
            Some(Ok(body))
        }

        /// Read what has arrived, without blocking: a whole frame once one is complete.
        pub fn try_recv(&mut self) -> Result<Frame, TryRecvError> {
            loop {
                if let Some(e) = &mut self.ended {
                    return e.take().map(Err).ok_or(TryRecvError::Disconnected);
                }
                if let Some(f) = self.take_frame() {
                    return Ok(f);
                }
                match (&self.f).read(&mut self.chunk) {
                    Ok(0) => self.ended = Some(Some(std::io::ErrorKind::UnexpectedEof.into())),
                    Ok(n) => self.buf.extend_from_slice(&self.chunk[..n]),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        return Err(TryRecvError::Empty)
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(e) => self.ended = Some(Some(e)),
                }
            }
        }

        pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Frame, RecvTimeoutError> {
            use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
            let deadline = std::time::Instant::now() + timeout.min(Duration::from_secs(1 << 30));
            loop {
                match self.try_recv() {
                    Ok(f) => return Ok(f),
                    Err(TryRecvError::Disconnected) => return Err(RecvTimeoutError::Disconnected),
                    Err(TryRecvError::Empty) => {}
                }
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    return Err(RecvTimeoutError::Timeout);
                }
                // whole milliseconds, rounded up: a timeout of 0 would spin
                let ms = left.as_micros().div_ceil(1000).min(i32::MAX as u128) as i32;
                let mut fds = [PollFd::new(self.f.as_fd(), PollFlags::POLLIN)];
                match poll(
                    &mut fds,
                    PollTimeout::try_from(ms).unwrap_or(PollTimeout::MAX),
                ) {
                    Ok(_) | Err(nix::errno::Errno::EINTR) => {}
                    Err(e) => self.ended = Some(Some(e.into())),
                }
            }
        }
    }
}

/// Read frames into `tx` until end of file, an error, or the receiver is gone.
fn read_frames(f: File, tx: crossbeam_channel::Sender<Frame>) {
    let mut r = BufReader::with_capacity(1 << 16, f);
    loop {
        let mut hdr = [0u8; 4];
        if let Err(e) = r.read_exact(&mut hdr) {
            let _ = tx.send(Err(e));
            return;
        }
        let n = u32::from_le_bytes(hdr) as usize;
        if n > MAX_FRAME {
            let _ = tx.send(Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("response frame of {n} bytes"),
            )));
            return;
        }
        let mut buf = vec![0u8; n];
        if let Err(e) = r.read_exact(&mut buf) {
            let _ = tx.send(Err(e));
            return;
        }
        if tx.send(Ok(buf)).is_err() {
            return;
        }
    }
}

/// Drain the response channel on a thread of its own (macOS, Windows): blocking reads, frame by
/// frame, into a channel. Nothing polls it: on macOS `poll` did not report a frame that arrived while it
/// waited (only data already there), and a FIFO there holds less than a large result, so the
/// server blocked writing a 9 KB display list while the host waited out its watchdog. Draining
/// continuously also means the server never blocks on a full channel. The thread ends at end
/// of file (the server exited or was killed) or when the server object is dropped.
/// `before`: run on the thread before the first read (Windows waits there for the server to
/// connect).
#[cfg_attr(target_os = "linux", allow(dead_code))]
fn spawn_reader(
    f: File,
    before: impl FnOnce(&File) -> std::io::Result<()> + Send + 'static,
) -> Result<Frames> {
    let (tx, rx) = crossbeam_channel::unbounded();
    std::thread::Builder::new()
        .name("rtex-response-reader".into())
        .spawn(move || match before(&f) {
            Ok(()) => read_frames(f, tx),
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        })
        .context("spawning the response reader")?;
    Ok(Frames::Thread(rx))
}

#[cfg(unix)]
pub(crate) use unix::Transport;
#[cfg(windows)]
pub(crate) use windows::Transport;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::path::PathBuf;
    use std::process::Stdio;
    use std::time::Instant;

    pub(crate) struct Transport {
        fifo: PathBuf,
    }

    impl Transport {
        pub fn create(work_dir: &Path) -> Result<Transport> {
            let fifo = work_dir.join(format!("resp-{}.fifo", std::process::id()));
            let _ = std::fs::remove_file(&fifo);
            nix::unistd::mkfifo(&fifo, nix::sys::stat::Mode::S_IRWXU).context("mkfifo")?;
            Ok(Transport { fifo })
        }

        pub fn configure(&self, cmd: &mut Command) {
            cmd.env("RTEX_RESP", &self.fifo).stdin(Stdio::piped());
        }

        /// Wait (up to `timeout`, failing early if the child exits) until the server opened
        /// the response FIFO and wrote its first frame.
        pub fn connect(self, child: &mut Child, timeout: Duration) -> Result<Connected> {
            let stdin = child.stdin.take().context("no stdin")?;
            let resp = open_fifo_with_timeout(&self.fifo, child, timeout)?;
            #[cfg(target_os = "linux")]
            let frames = Frames::Direct(direct::Reader::new(resp));
            #[cfg(not(target_os = "linux"))]
            let frames = spawn_reader(set_blocking(resp)?, |_| Ok(()))?;
            Ok(Connected {
                requests: File::from(std::os::fd::OwnedFd::from(stdin)),
                frames,
                first: None,
            })
        }
    }

    fn open_fifo_with_timeout(fifo: &Path, child: &mut Child, timeout: Duration) -> Result<File> {
        use nix::poll::{poll, PollFd, PollFlags, PollTimeout};
        use std::os::fd::AsFd;
        use std::os::unix::fs::OpenOptionsExt;
        // Open the read end once, non-blocking, so the open itself never blocks and the
        // server's own open(2) for writing succeeds as soon as it gets there. Never close and
        // reopen: a write into a FIFO without a reader would be lost (EPIPE on the server side).
        let f = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(fifo)?;
        let t0 = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                bail!("lualatex server exited during startup with {status}");
            }
            if t0.elapsed() > timeout {
                bail!("timed out waiting for the server to open the response FIFO");
            }
            // Linux does not report POLLHUP for a FIFO that never had a writer, so poll blocks
            // until the first bytes ("ready" frame) arrive.
            let mut fds = [PollFd::new(f.as_fd(), PollFlags::POLLIN)];
            let n = poll(&mut fds, PollTimeout::from(50u16))?;
            if n > 0 {
                if let Some(ev) = fds[0].revents() {
                    if ev.contains(PollFlags::POLLIN) {
                        break;
                    }
                    if ev.contains(PollFlags::POLLHUP) {
                        bail!("server closed the response FIFO during startup");
                    }
                }
            }
        }
        Ok(f)
    }

    /// Back to blocking mode for the reader thread's framed reads.
    #[cfg(not(target_os = "linux"))]
    fn set_blocking(f: File) -> Result<File> {
        let fd = std::os::unix::io::AsRawFd::as_raw_fd(&f);
        let flags = nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_GETFL)?;
        let mut oflags = nix::fcntl::OFlag::from_bits_truncate(flags);
        oflags.remove(nix::fcntl::OFlag::O_NONBLOCK);
        nix::fcntl::fcntl(fd, nix::fcntl::FcntlArg::F_SETFL(oflags))?;
        Ok(f)
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use std::process::Stdio;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;
    use windows_sys::Win32::Foundation::{ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
        PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    static NEXT: AtomicU64 = AtomicU64::new(0);

    pub(crate) struct Transport {
        req_name: String,
        resp_name: String,
        req: OwnedHandle,
        resp: OwnedHandle,
    }

    /// One server end of a byte-mode pipe, one instance, local clients only. The handle is not
    /// inheritable: the child opens the pipe by name.
    fn create_pipe(name: &str, inbound: bool) -> Result<OwnedHandle> {
        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let access = if inbound {
            PIPE_ACCESS_INBOUND
        } else {
            PIPE_ACCESS_OUTBOUND
        };
        // SAFETY: `wide` is NUL-terminated and outlives the call; no security attributes.
        let h = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                access | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                1 << 16,
                1 << 16,
                0,
                std::ptr::null(),
            )
        };
        if h == INVALID_HANDLE_VALUE || h.is_null() {
            return Err(std::io::Error::last_os_error())
                .with_context(|| format!("creating the named pipe {name}"));
        }
        // SAFETY: a fresh handle we own.
        Ok(unsafe { OwnedHandle::from_raw_handle(h as _) })
    }

    /// Wait for the client to open the pipe (immediately done when it already has).
    fn wait_client(h: &impl AsRawHandle) -> std::io::Result<()> {
        // SAFETY: a valid pipe handle; synchronous (no OVERLAPPED).
        if unsafe { ConnectNamedPipe(h.as_raw_handle() as _, std::ptr::null_mut()) } != 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(ERROR_PIPE_CONNECTED as i32) {
            Ok(())
        } else {
            Err(e)
        }
    }

    impl Transport {
        pub fn create(_work_dir: &Path) -> Result<Transport> {
            let base = format!(
                r"\\.\pipe\rtex-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            );
            let (req_name, resp_name) = (format!("{base}-req"), format!("{base}-resp"));
            Ok(Transport {
                req: create_pipe(&req_name, false)?,
                resp: create_pipe(&resp_name, true)?,
                req_name,
                resp_name,
            })
        }

        pub fn configure(&self, cmd: &mut Command) {
            cmd.env("RTEX_REQ", &self.req_name)
                .env("RTEX_RESP", &self.resp_name)
                .stdin(Stdio::null());
        }

        /// Wait (up to `timeout`, failing early if the child exits) for the server's first
        /// frame. The server opens the request pipe before it writes that frame.
        pub fn connect(self, child: &mut Child, timeout: Duration) -> Result<Connected> {
            let mut frames = spawn_reader(File::from(self.resp), wait_client)?;
            let t0 = Instant::now();
            let failed = |why: String| -> Result<Connected> {
                // a reader still waiting for the server to connect: connect and leave, so it
                // reads end of file and its thread ends
                let _ = std::fs::OpenOptions::new()
                    .write(true)
                    .open(&self.resp_name);
                bail!(why)
            };
            let first = loop {
                match frames.recv_timeout(Duration::from_millis(50)) {
                    Ok(f) => break f,
                    Err(crossbeam_channel::RecvTimeoutError::Disconnected) => {
                        bail!("the response reader ended during startup")
                    }
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                }
                if let Some(status) = child.try_wait()? {
                    return failed(format!(
                        "lualatex server exited during startup with {status}"
                    ));
                }
                if t0.elapsed() > timeout {
                    return failed(
                        "timed out waiting for the server to open the response pipe".into(),
                    );
                }
            };
            if let Err(e) = &first {
                bail!("the server closed the response pipe during startup: {e}");
            }
            wait_client(&self.req)
                .with_context(|| format!("the server did not open {}", self.req_name))?;
            Ok(Connected {
                requests: File::from(self.req),
                frames,
                first: Some(first),
            })
        }
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn direct_reader_splits_frames_and_reports_the_end_once() {
        let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_NONBLOCK).unwrap();
        let mut w = File::from(w);
        let mut rd = direct::Reader::new(File::from(r));
        assert!(matches!(rd.try_recv(), Err(TryRecvError::Empty)));
        assert!(matches!(
            rd.recv_timeout(Duration::from_millis(5)),
            Err(RecvTimeoutError::Timeout)
        ));
        // a frame in two writes, then two frames in one
        w.write_all(&[3, 0, 0, 0, b'a']).unwrap();
        assert!(matches!(rd.try_recv(), Err(TryRecvError::Empty)));
        w.write_all(b"bc\x01\0\0\0d\0\0\0\0").unwrap();
        assert_eq!(rd.try_recv().unwrap().unwrap(), b"abc");
        assert_eq!(
            rd.recv_timeout(Duration::from_secs(1)).unwrap().unwrap(),
            b"d"
        );
        assert_eq!(rd.try_recv().unwrap().unwrap(), b"");
        drop(w);
        let e = rd
            .recv_timeout(Duration::from_secs(1))
            .unwrap()
            .unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::UnexpectedEof);
        assert!(matches!(rd.try_recv(), Err(TryRecvError::Disconnected)));
    }
}
