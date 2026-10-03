//! Local, current-user/session-only pipes with bounded overlapped I/O.
use crate::{MAX_FRAME, Request, Response};
use std::{
    io, mem, ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::Authorization::*,
    Security::*,
    Storage::FileSystem::*,
    System::{IO::*, Pipes::*, RemoteDesktop::*, Threading::*},
};

struct Handle(HANDLE);
// Owned kernel handles may be transferred to the worker; only that worker uses the pipe.
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
fn owned(h: HANDLE) -> io::Result<Handle> {
    if h.is_null() || h == INVALID_HANDLE_VALUE {
        Err(io::Error::last_os_error())
    } else {
        Ok(Handle(h))
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn identity(pid: u32) -> io::Result<(String, u32)> {
    unsafe {
        let process = owned(OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid))?;
        let mut token = ptr::null_mut();
        if OpenProcessToken(process.0, TOKEN_QUERY, &mut token) == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = owned(token)?;
        let mut length = 0;
        GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut length);
        let mut buffer = vec![0usize; (length as usize).div_ceil(mem::size_of::<usize>())];
        if GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            length,
            &mut length,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = &*buffer.as_ptr().cast::<TOKEN_USER>();
        let mut sid = ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut sid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut len = 0;
        while *sid.add(len) != 0 {
            len += 1;
        }
        let value = String::from_utf16_lossy(std::slice::from_raw_parts(sid, len));
        LocalFree(sid.cast());
        let mut session = 0;
        if ProcessIdToSessionId(pid, &mut session) == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((value, session))
    }
}
pub fn endpoint() -> io::Result<String> {
    let (sid, session) = identity(std::process::id())?;
    Ok(format!(r"\\.\pipe\LucidDesk.Control.v1.{sid}.{session}"))
}
fn verify_peer(pipe: HANDLE, server: bool) -> io::Result<()> {
    let mut pid = 0;
    let ok = unsafe {
        if server {
            GetNamedPipeServerProcessId(pipe, &mut pid)
        } else {
            GetNamedPipeClientProcessId(pipe, &mut pid)
        }
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    if identity(pid)? != identity(std::process::id())? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "pipe peer is not the current user/session",
        ));
    }
    Ok(())
}

/// Owns an overlapped operation until completion, including cancellation.
fn operation(
    handle: HANDLE,
    deadline: Instant,
    stop: Option<HANDLE>,
    start: impl FnOnce(*mut OVERLAPPED) -> i32,
) -> io::Result<u32> {
    let event = owned(unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) })?;
    let mut overlapped: OVERLAPPED = unsafe { mem::zeroed() };
    overlapped.hEvent = event.0;
    let ok = start(&mut overlapped);
    if ok == 0 && unsafe { GetLastError() } != ERROR_IO_PENDING {
        return Err(io::Error::last_os_error());
    }
    let wait = deadline
        .saturating_duration_since(Instant::now())
        .as_millis()
        .min(u32::MAX as u128 - 1) as u32;
    let handles = [event.0, stop.unwrap_or(event.0)];
    let result = unsafe {
        WaitForMultipleObjects(
            if stop.is_some() { 2 } else { 1 },
            handles.as_ptr(),
            0,
            wait,
        )
    };
    let mut transferred = 0;
    if result != WAIT_OBJECT_0 {
        unsafe {
            CancelIoEx(handle, &overlapped);
            // Buffers and OVERLAPPED must remain alive until cancellation completes.
            GetOverlappedResult(handle, &overlapped, &mut transferred, 1);
        }
        return Err(io::Error::new(
            if result == WAIT_TIMEOUT {
                io::ErrorKind::TimedOut
            } else {
                io::ErrorKind::Interrupted
            },
            "pipe request interrupted or timed out",
        ));
    }
    if unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, 0) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(transferred)
}
fn read_exact(
    h: HANDLE,
    mut bytes: &mut [u8],
    deadline: Instant,
    stop: Option<HANDLE>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let n = operation(h, deadline, stop, |ov| unsafe {
            ReadFile(
                h,
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                ptr::null_mut(),
                ov,
            )
        })? as usize;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        bytes = &mut bytes[n..];
    }
    Ok(())
}
fn write_all(
    h: HANDLE,
    mut bytes: &[u8],
    deadline: Instant,
    stop: Option<HANDLE>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let n = operation(h, deadline, stop, |ov| unsafe {
            WriteFile(h, bytes.as_ptr(), bytes.len() as u32, ptr::null_mut(), ov)
        })? as usize;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        bytes = &bytes[n..];
    }
    Ok(())
}
fn read_frame(h: HANDLE, deadline: Instant, stop: Option<HANDLE>) -> io::Result<Vec<u8>> {
    let mut header = [0; 4];
    read_exact(h, &mut header, deadline, stop)?;
    let len = u32::from_le_bytes(header) as usize;
    if len == 0 || len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds limit",
        ));
    }
    let mut bytes = vec![0; len];
    read_exact(h, &mut bytes, deadline, stop)?;
    Ok(bytes)
}
fn write_frame(h: HANDLE, bytes: &[u8], deadline: Instant, stop: Option<HANDLE>) -> io::Result<()> {
    if bytes.len() > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds limit",
        ));
    }
    write_all(h, &(bytes.len() as u32).to_le_bytes(), deadline, stop)?;
    write_all(h, bytes, deadline, stop)
}
pub fn call(request: &Request, timeout: Duration) -> io::Result<Response> {
    call_at(&endpoint()?, request, timeout)
}
pub fn call_at(name: &str, request: &Request, timeout: Duration) -> io::Result<Response> {
    let deadline = Instant::now() + timeout;
    let name = wide(name);
    let pipe = loop {
        let h = unsafe {
            CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                ptr::null_mut(),
            )
        };
        if h != INVALID_HANDLE_VALUE {
            break owned(h)?;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(ERROR_PIPE_BUSY as i32) {
            return Err(error);
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(io::ErrorKind::TimedOut.into());
        }
        unsafe {
            WaitNamedPipeW(name.as_ptr(), remaining.as_millis().min(100) as u32);
        }
    };
    verify_peer(pipe.0, true)?;
    write_frame(pipe.0, &serde_json::to_vec(request)?, deadline, None)?;
    let bytes = read_frame(pipe.0, deadline, None)?;
    write_all(pipe.0, &[1], deadline, None)?;
    let response: Response = serde_json::from_slice(&bytes)?;
    if response.request_id != request.request_id || response.protocol_version != crate::VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid response identity/version",
        ));
    }
    Ok(response)
}

pub struct Server {
    stop: Handle,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    pub fn start(handler: impl Fn(Request) -> Response + Send + 'static) -> io::Result<Self> {
        Self::start_at(&endpoint()?, handler)
    }
    pub fn start_at(
        name: &str,
        handler: impl Fn(Request) -> Response + Send + 'static,
    ) -> io::Result<Self> {
        let (sid, _) = identity(std::process::id())?;
        let sddl = wide(&format!("D:P(A;;GA;;;{sid})"));
        let mut descriptor = ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let attributes = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let name = wide(name);
        let raw = unsafe {
            CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                65536,
                65536,
                0,
                &attributes,
            )
        };
        let pipe = owned(raw);
        unsafe {
            LocalFree(descriptor);
        }
        let pipe = pipe?;
        let stop = owned(unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) })?;
        let stop_raw = stop.0 as usize;
        let thread = std::thread::Builder::new()
            .name("luciddesk-control".into())
            .spawn(move || {
                let pipe = pipe;
                let stop = stop_raw as HANDLE;
                while unsafe { WaitForSingleObject(stop, 0) } != WAIT_OBJECT_0 {
                    let connected = operation(
                        pipe.0,
                        Instant::now() + Duration::from_secs(86400),
                        Some(stop),
                        |ov| unsafe { ConnectNamedPipe(pipe.0, ov) },
                    );
                    if let Err(error) = connected {
                        if error.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                            if error.kind() == io::ErrorKind::Interrupted {
                                break;
                            }
                            continue;
                        }
                    }
                    let deadline = Instant::now() + Duration::from_secs(10);
                    let _ = (|| -> io::Result<()> {
                        verify_peer(pipe.0, false)?;
                        let bytes = read_frame(pipe.0, deadline, Some(stop))?;
                        let response = match serde_json::from_slice::<Request>(&bytes) {
                            Ok(request) => handler(request),
                            Err(error) => {
                                Response::failure("", "INVALID_REQUEST", error.to_string())
                            }
                        };
                        let mut bytes = serde_json::to_vec(&response)?;
                        if bytes.len() > MAX_FRAME {
                            bytes = serde_json::to_vec(&Response::failure(
                                &response.request_id,
                                "RESULT_TOO_LARGE",
                                "response exceeds 4 MiB",
                            ))?;
                        }
                        write_frame(pipe.0, &bytes, deadline, Some(stop))?;
                        let mut ack = [0];
                        read_exact(pipe.0, &mut ack, deadline, Some(stop))?;
                        Ok(())
                    })();
                    unsafe {
                        DisconnectNamedPipe(pipe.0);
                    }
                }
            })?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.stop.0);
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pipe_round_trip_repeated_and_exclusive() {
        let name = format!("{}-test-{}", endpoint().unwrap(), crate::request_id());
        let server = Server::start_at(&name, |r| {
            Response::success(
                &r.request_id,
                serde_json::json!({}),
                serde_json::json!({"title":"中文"}),
            )
        })
        .unwrap();
        assert!(Server::start_at(&name, |_| unreachable!()).is_err());
        for _ in 0..3 {
            let request = Request {
                protocol_version: 1,
                request_id: crate::request_id(),
                command: "status".into(),
                id: None,
                pane: None,
                unassigned: false,
                data_dir: None,
                plan: None,
                token: None,
            };
            let response = call_at(&name, &request, Duration::from_secs(2)).unwrap();
            assert_eq!(response.data.unwrap()["title"], "中文");
        }
        drop(server);
        let server = Server::start_at(&name, |_| unreachable!()).unwrap();
        let start = Instant::now();
        drop(server);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}

#[cfg(test)]
mod fault_tests {
    use super::*;
    fn request() -> Request {
        Request {
            protocol_version: 1,
            request_id: crate::request_id(),
            command: "status".into(),
            id: None,
            pane: None,
            unassigned: false,
            data_dir: None,
            plan: None,
            token: None,
        }
    }
    #[test]
    fn missing_server_does_not_create_anything() {
        let name = format!("{}-absent-{}", endpoint().unwrap(), crate::request_id());
        assert_eq!(
            call_at(&name, &request(), Duration::from_millis(100))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound
        );
    }
    #[test]
    fn read_timeout_cancels_pending_io() {
        let name = format!("{}-slow-{}", endpoint().unwrap(), crate::request_id());
        let server = Server::start_at(&name, |r| {
            std::thread::sleep(Duration::from_millis(150));
            Response::success(&r.request_id, serde_json::json!({}), serde_json::json!({}))
        })
        .unwrap();
        let start = Instant::now();
        assert_eq!(
            call_at(&name, &request(), Duration::from_millis(25))
                .unwrap_err()
                .kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < Duration::from_secs(1));
        drop(server);
    }
    #[test]
    fn oversized_frame_is_rejected_before_allocation_and_server_recovers() {
        let name = format!("{}-frame-{}", endpoint().unwrap(), crate::request_id());
        let server = Server::start_at(&name, |r| {
            Response::success(&r.request_id, serde_json::json!({}), serde_json::json!({}))
        })
        .unwrap();
        let pipe = owned(unsafe {
            CreateFileW(
                wide(&name).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                ptr::null_mut(),
            )
        })
        .unwrap();
        write_all(
            pipe.0,
            &((MAX_FRAME + 1) as u32).to_le_bytes(),
            Instant::now() + Duration::from_secs(1),
            None,
        )
        .unwrap();
        let mut byte = [0];
        assert!(
            read_exact(
                pipe.0,
                &mut byte,
                Instant::now() + Duration::from_secs(1),
                None
            )
            .is_err()
        );
        drop(pipe);
        assert!(
            call_at(&name, &request(), Duration::from_secs(2))
                .unwrap()
                .ok
        );
        drop(server);
    }
}
