//! Current-user/session IPC. Same-user processes are inside the trust boundary;
//! this is not executable-signing authentication or protection from that user.
use serde::{Deserialize, Serialize};
use std::{
    ffi::c_void,
    mem::size_of,
    os::windows::io::AsRawHandle,
    ptr::null_mut,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::windows::named_pipe::{ClientOptions, ServerOptions},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, LocalFree},
    Security::{
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SE_KERNEL_OBJECT,
        },
        GetTokenInformation, OWNER_SECURITY_INFORMATION, SECURITY_ATTRIBUTES, TOKEN_QUERY,
        TOKEN_USER, TokenUser,
    },
    System::{
        Pipes::{GetNamedPipeClientProcessId, GetNamedPipeServerProcessId},
        RemoteDesktop::ProcessIdToSessionId,
        Threading::{
            CreateMutexW, GetCurrentProcessId, OpenProcess, OpenProcessToken,
            PROCESS_QUERY_LIMITED_INFORMATION,
        },
    },
};
const LIMIT: usize = 4096;
const DEADLINE: Duration = Duration::from_secs(40);
const INPUT_DEADLINE: Duration = Duration::from_secs(2);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    #[default]
    Show,
    Status,
    Start,
    Restart,
    Stop,
    Quit,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Request {
    pub version: u32,
    pub action: Action,
    pub directory: Option<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reply {
    pub version: u32,
    pub ok: bool,
    pub running: bool,
    pub message: String,
    /// Optional classified failure: 2 for arguments/configuration, 3 for operations.
    /// Omitted by legacy peers; the client defaults failed replies to 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<u8>,
}
impl Reply {
    pub fn new(ok: bool, running: bool, message: &str) -> Self {
        Self {
            version: 1,
            ok,
            running,
            message: message.into(),
            exit_code: None,
        }
    }
}
pub(crate) struct Control {
    pub request: Request,
    pub reply: SyncSender<Reply>,
}
#[derive(Default)]
pub(crate) struct Signals {
    pub show: AtomicBool,
    pub quit: AtomicBool,
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: This wrapper uniquely owns a successful Win32 handle.
        unsafe {
            CloseHandle(self.0);
        }
    }
}
struct Descriptor(*mut c_void);
impl Drop for Descriptor {
    fn drop(&mut self) {
        // SAFETY: Conversion allocated this descriptor with LocalAlloc.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn error(operation: &str) -> String {
    crate::windows::last_error(operation)
}
fn identity(pid: u32) -> Result<(String, u32), String> {
    // SAFETY: Only query access is requested; handle is checked and uniquely owned.
    let process = Handle(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) });
    if process.0.is_null() {
        return Err(error("查询 IPC 进程"));
    }
    let mut token = null_mut();
    // SAFETY: Live process handle and writable output pointer.
    if unsafe { OpenProcessToken(process.0, TOKEN_QUERY, &mut token) } == 0 {
        return Err(error("查询 IPC 用户"));
    }
    let token = Handle(token);
    let mut length = 0;
    // SAFETY: First call only obtains the required allocation size.
    unsafe {
        GetTokenInformation(token.0, TokenUser, null_mut(), 0, &mut length);
    }
    if length == 0 || length > 65536 {
        return Err("IPC 用户令牌长度无效。".into());
    }
    let mut data = vec![0usize; (length as usize).div_ceil(size_of::<usize>())];
    // SAFETY: Aligned allocation has at least length bytes and remains live through SID conversion.
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            data.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(error("读取 IPC 用户"));
    }
    let mut text = null_mut();
    // SAFETY: Successful TokenUser contains a valid SID within the live allocation.
    if unsafe {
        ConvertSidToStringSidW((*(data.as_ptr().cast::<TOKEN_USER>())).User.Sid, &mut text)
    } == 0
    {
        return Err(error("转换用户 SID"));
    }
    let allocation = Descriptor(text.cast());
    let mut len = 0;
    // SAFETY: Successful conversion returns a NUL-terminated UTF-16 allocation.
    let sid = unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    drop(allocation);
    let mut session = 0;
    // SAFETY: Writable output and a live process identifier.
    if unsafe { ProcessIdToSessionId(pid, &mut session) } == 0 {
        return Err(error("查询 IPC 会话"));
    }
    Ok((sid, session))
}
fn descriptor(sid: &str) -> Result<Descriptor, String> {
    // Protected DACL: no inherited administrators/Everyone ACEs. Owner is explicit.
    let sddl = crate::windows::wide(&format!("O:{sid}D:P(A;;GA;;;{sid})"));
    let mut pointer = null_mut();
    // SAFETY: NUL-terminated SDDL, correct revision, writable output.
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut pointer,
            null_mut(),
        )
    } == 0
    {
        return Err(error("建立 IPC 权限"));
    }
    Ok(Descriptor(pointer))
}
fn verify_owner(handle: HANDLE, expected: &str) -> Result<(), String> {
    let mut owner = null_mut();
    let mut sd = null_mut();
    // SAFETY: Live mutex, writable outputs; returned descriptor is LocalFree-owned.
    let result = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            null_mut(),
            null_mut(),
            &mut sd,
        )
    };
    if result != 0 {
        return Err("无法验证单实例 Mutex 所有者；安全退出。".into());
    }
    let _sd = Descriptor(sd);
    let mut text = null_mut();
    // SAFETY: GetSecurityInfo returned owner SID within the retained descriptor.
    if owner.is_null() || unsafe { ConvertSidToStringSidW(owner, &mut text) } == 0 {
        return Err("Mutex 所有者无效。".into());
    }
    let _text = Descriptor(text.cast());
    let mut len = 0;
    // SAFETY: Conversion returned NUL-terminated UTF-16, retained above.
    let actual = unsafe {
        while *text.add(len) != 0 {
            len += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(text, len))
    };
    if actual != expected {
        return Err("Mutex 被其他用户抢占；未发送 IPC。".into());
    }
    Ok(())
}
fn attributes(sd: &Descriptor) -> SECURITY_ATTRIBUTES {
    SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: 0,
    }
}
pub(crate) struct Instance {
    _mutex: Option<Handle>,
    pipe: String,
    sid: String,
    session: u32,
    primary: bool,
}
impl Instance {
    pub fn acquire() -> Result<Self, String> {
        // SAFETY: Current process ID has no preconditions.
        let (sid, session) = identity(unsafe { GetCurrentProcessId() })?;
        let sd = descriptor(&sid)?;
        let name = crate::windows::wide(&format!("Local\\AIRelay.{sid}.{session}.v1"));
        // SAFETY: Descriptor/name live through call; non-inheritable mutex retained for process lifetime.
        let mutex = unsafe { CreateMutexW(&attributes(&sd), 0, name.as_ptr()) };
        // SAFETY: Read immediately after CreateMutexW.
        let existing = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
        if mutex.is_null() {
            return Err(error("建立单实例 Mutex（可能存在命名抢占或权限冲突）"));
        }
        let mutex = Handle(mutex);
        verify_owner(mutex.0, &sid)?;
        Ok(Self {
            _mutex: Some(mutex),
            pipe: format!(r"\\.\pipe\AIRelay.{sid}.{session}.v1"),
            sid,
            session,
            primary: !existing,
        })
    }
    pub fn primary(&self) -> bool {
        self.primary
    }
    /// Release the launcher's reservation before spawning the real GUI host.
    /// The retained client identity still verifies both SID and session on every send.
    pub fn release_for_launch(&mut self) {
        self._mutex.take();
        self.primary = false;
    }
    pub fn send(&self, request: Request) -> Result<Reply, String> {
        self.send_timeout(request, DEADLINE)
    }
    pub fn send_timeout(&self, request: Request, deadline: Duration) -> Result<Reply, String> {
        runtime()?.block_on(async {
            tokio::time::timeout(deadline, async {
                let client = loop {
                    match ClientOptions::new().open(&self.pipe) {
                        Ok(client) => break client,
                        Err(e) if matches!(e.raw_os_error(), Some(2 | 231)) => {
                            tokio::time::sleep(Duration::from_millis(50)).await
                        }
                        Err(_) => return Err("无法连接受限 IPC；未发送命令。".into()),
                    }
                };
                let mut pid = 0;
                // SAFETY: Live pipe handle and writable PID; no payload sent before identity verification.
                if unsafe { GetNamedPipeServerProcessId(client.as_raw_handle(), &mut pid) } == 0 {
                    return Err(error("验证 IPC 服务端"));
                }
                if identity(pid)? != (self.sid.clone(), self.session) {
                    return Err("IPC 服务端用户或会话不匹配；未发送命令。".into());
                }
                let mut client = client;
                write_frame(&mut client, &request).await?;
                let reply: Reply = read_frame(&mut client).await?;
                if reply.version != 1 {
                    return Err("IPC 响应版本不兼容。".into());
                }
                client.write_u8(1).await.map_err(|_| "IPC 响应确认失败。")?;
                // Do not return until the server disconnected this transaction;
                // otherwise the next process can reopen the old connected instance.
                let mut end = [0u8; 1];
                match client.read(&mut end).await {
                    Ok(0) | Err(_) => {}
                    Ok(_) => return Err("IPC 响应包含多余数据。".into()),
                }
                Ok(reply)
            })
            .await
            .map_err(|_| "IPC 总时限已到；命令可能已执行，请查询状态，不自动重试。".to_string())?
        })
    }
    pub fn serve(&self) -> Result<Server, String> {
        let pipe = self.pipe.clone();
        let expected_identity = (self.sid.clone(), self.session);
        let sd = descriptor(&self.sid)?;
        let rt = runtime()?;
        let mut server = {
            let _enter = rt.enter();
            // SAFETY: Valid SECURITY_ATTRIBUTES/descriptor live for creation. Tokio owns overlapped handle.
            unsafe {
                ServerOptions::new()
                    .first_pipe_instance(true)
                    .reject_remote_clients(true)
                    .max_instances(2)
                    .in_buffer_size(LIMIT as u32)
                    .out_buffer_size(LIMIT as u32)
                    .create_with_security_attributes_raw(
                        &pipe,
                        (&mut attributes(&sd) as *mut SECURITY_ATTRIBUTES).cast(),
                    )
            }
            .map_err(|_| "无法独占创建 IPC 管道（命名抢占或权限冲突）；安全退出。".to_string())?
        };
        let (tx, receiver) = mpsc::sync_channel(4);
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        std::thread::Builder::new()
            .name("ai-relay-ipc".into())
            .spawn(move || {
                rt.block_on(async {
                    let Ok(sd) = descriptor(&expected_identity.0) else {
                        return;
                    };
                    while !stopping.load(Ordering::Acquire) {
                        match tokio::time::timeout(Duration::from_millis(100), server.connect())
                            .await
                        {
                            Err(_) => continue,
                            Ok(Err(_)) => break,
                            Ok(Ok(())) => {}
                        }
                        let mut client_pid = 0;
                        // SAFETY: Connected live pipe and writable PID output. Reject other sessions before payload parsing.
                        if unsafe {
                            GetNamedPipeClientProcessId(server.as_raw_handle(), &mut client_pid)
                        } == 0
                            || identity(client_pid).ok().as_ref() != Some(&expected_identity)
                        {
                            let Ok(next) = replacement(&pipe, &sd) else {
                                break;
                            };
                            let _ = server.disconnect();
                            server = next;
                            continue;
                        }
                        let _ = tokio::time::timeout(DEADLINE, async {
                            let request: Request =
                                tokio::time::timeout(INPUT_DEADLINE, read_frame(&mut server))
                                    .await
                                    .map_err(|_| "IPC 输入超时。".to_string())??;
                            let reply = if request.version != 1 {
                                Reply::new(false, false, "IPC 协议版本不兼容。")
                            } else {
                                let (reply, response) = mpsc::sync_channel(1);
                                if tx.try_send(Control { request, reply }).is_err() {
                                    Reply::new(false, false, "控制器繁忙或已退出。")
                                } else {
                                    loop {
                                        match response.try_recv() {
                                            Ok(reply) => break reply,
                                            Err(mpsc::TryRecvError::Empty) => {
                                                tokio::time::sleep(Duration::from_millis(10)).await
                                            }
                                            Err(_) => {
                                                break Reply::new(false, false, "控制器已退出。");
                                            }
                                        }
                                    }
                                }
                            };
                            write_frame(&mut server, &reply).await?;
                            // Wait for receipt before DisconnectNamedPipe discards unread data.
                            server
                                .read_u8()
                                .await
                                .map_err(|_| "IPC 客户端未确认响应。".to_string())
                                .map(|_| ())
                        })
                        .await;
                        let Ok(next) = replacement(&pipe, &sd) else {
                            break;
                        };
                        let _ = server.disconnect();
                        server = next;
                    }
                });
            })
            .map_err(|_| "无法创建 IPC 线程。".to_string())?;
        Ok(Server {
            receiver: Some(receiver),
            stop,
        })
    }
}
fn replacement(
    pipe: &str,
    sd: &Descriptor,
) -> Result<tokio::net::windows::named_pipe::NamedPipeServer, std::io::Error> {
    // SAFETY: Valid descriptor/attributes live through creation; the preceding
    // instance remains open, preventing a foreign first-instance takeover gap.
    unsafe {
        ServerOptions::new()
            .reject_remote_clients(true)
            .max_instances(2)
            .in_buffer_size(LIMIT as u32)
            .out_buffer_size(LIMIT as u32)
            .create_with_security_attributes_raw(
                pipe,
                (&mut attributes(sd) as *mut SECURITY_ATTRIBUTES).cast(),
            )
    }
}
pub(crate) struct Server {
    receiver: Option<Receiver<Control>>,
    stop: Arc<AtomicBool>,
}
impl Server {
    pub fn take_receiver(&mut self) -> Result<Receiver<Control>, String> {
        self.receiver
            .take()
            .ok_or_else(|| "IPC 接收端已被接管。".into())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "无法创建 IPC 运行时。".into())
}
async fn read_frame<T: for<'a> Deserialize<'a>>(
    mut stream: impl tokio::io::AsyncRead + Unpin,
) -> Result<T, String> {
    let length = stream
        .read_u32_le()
        .await
        .map_err(|_| "IPC 帧头读取失败。")? as usize;
    if length == 0 || length > LIMIT {
        return Err("IPC 消息长度无效。".into());
    }
    let mut bytes = vec![0; length];
    stream
        .read_exact(&mut bytes)
        .await
        .map_err(|_| "IPC 消息读取失败。")?;
    serde_json::from_slice(&bytes).map_err(|_| "IPC 消息格式或命令无效。".into())
}
async fn write_frame<T: Serialize>(
    mut stream: impl tokio::io::AsyncWrite + Unpin,
    value: &T,
) -> Result<(), String> {
    let bytes = serde_json::to_vec(value).map_err(|_| "IPC 编码失败。")?;
    if bytes.len() > LIMIT {
        return Err("IPC 消息过大。".into());
    }
    stream
        .write_u32_le(bytes.len() as u32)
        .await
        .map_err(|_| "IPC 写入失败。")?;
    stream
        .write_all(&bytes)
        .await
        .map_err(|_| "IPC 写入失败。".into())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_restart_and_legacy_reply_wire_format() {
        for (action, name) in [(Action::Start, "start"), (Action::Restart, "restart")] {
            let request = Request {
                version: 1,
                action,
                directory: None,
            };
            let json = serde_json::to_vec(&request).unwrap();
            assert_eq!(
                serde_json::from_slice::<Request>(&json).unwrap().action,
                action
            );
            assert_eq!(serde_json::to_value(action).unwrap(), name);
        }
        let reply: Reply =
            serde_json::from_str(r#"{"version":1,"ok":false,"running":false,"message":"legacy"}"#)
                .unwrap();
        assert_eq!(reply.exit_code, None);
        let mut reply = Reply::new(false, false, "configuration");
        reply.exit_code = Some(2);
        assert_eq!(
            serde_json::from_slice::<Reply>(&serde_json::to_vec(&reply).unwrap())
                .unwrap()
                .exit_code,
            Some(2)
        );
    }
    #[test]
    fn bounded_protocol_and_stalled_input() {
        runtime().unwrap().block_on(async {
            let (mut writer, mut reader) = tokio::io::duplex(8192);
            writer.write_u32_le((LIMIT + 1) as u32).await.unwrap();
            assert!(read_frame::<Request>(&mut reader).await.is_err());
            let (_writer, mut reader) = tokio::io::duplex(64);
            assert!(
                tokio::time::timeout(
                    Duration::from_millis(30),
                    read_frame::<Request>(&mut reader)
                )
                .await
                .is_err()
            );
            let (mut writer, mut reader) = tokio::io::duplex(8192);
            write_frame(
                &mut writer,
                &Request {
                    version: 1,
                    action: Action::Status,
                    directory: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(
                read_frame::<Request>(&mut reader).await.unwrap().action,
                Action::Status
            );
            assert!(
                serde_json::from_str::<Request>(
                    r#"{"version":1,"action":"execute","directory":null}"#
                )
                .is_err()
            );
            assert!(
                serde_json::from_str::<Request>(
                    r#"{"version":1,"action":"show","directory":null,"key":"not-allowed"}"#
                )
                .is_err()
            );
        });
    }
    #[test]
    fn token_descriptor_and_owner_are_valid() {
        // SAFETY: Current PID has no preconditions; this test creates no named application objects.
        let (sid, _) = identity(unsafe { GetCurrentProcessId() }).unwrap();
        let sd = descriptor(&sid).unwrap();
        // SAFETY: Valid descriptor, null name creates a private unnamed mutex.
        let mutex = Handle(unsafe { CreateMutexW(&attributes(&sd), 0, std::ptr::null()) });
        assert!(!mutex.0.is_null());
        verify_owner(mutex.0, &sid).unwrap();
        assert!(verify_owner(mutex.0, "S-1-0-0").is_err());
    }
}
