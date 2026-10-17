//! 应用生命周期、参数及后台控制协调。
#[cfg(windows)]
pub(crate) mod controller;
#[cfg(windows)]
mod migration;
#[cfg(windows)]
pub(crate) use crate::windows::instance;

use std::{ffi::OsString, path::PathBuf, process::ExitCode};

#[derive(Debug)]
pub struct RunError {
    message: String,
    code: u8,
}
impl RunError {
    fn arguments(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 2,
        }
    }
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(self.code)
    }
}
impl From<String> for RunError {
    fn from(message: String) -> Self {
        Self { message, code: 3 }
    }
}
impl From<&str> for RunError {
    fn from(message: &str) -> Self {
        Self::from(message.to_owned())
    }
}
impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}
impl std::error::Error for RunError {}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Command {
    #[default]
    Launch,
    Show,
    Status,
    Start,
    Restart,
    Stop,
    Quit,
    Help,
    Version,
}
#[derive(Default, Debug, PartialEq, Eq)]
pub(crate) struct Options {
    pub config_dir: Option<PathBuf>,
    pub background: bool,
    pub command: Command,
}
fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Options, String> {
    let mut args = args.into_iter();
    let mut options = Options::default();
    let mut initial = false;
    while let Some(arg) = args.next() {
        if arg == "--config-dir" {
            if options.config_dir.is_some() {
                return Err("不能重复指定 --config-dir。".into());
            }
            let path = PathBuf::from(args.next().ok_or("--config-dir 缺少绝对目录。")?);
            if !path.is_absolute() {
                return Err("--config-dir 必须指定绝对目录，不会按当前工作目录补全。".into());
            }
            options.config_dir = Some(path);
        } else if arg == "--initial-action" {
            if initial || options.command != Command::Launch {
                return Err("内部初始化动作不能重复或与控制命令组合。".into());
            }
            options.command = match args.next().as_deref().and_then(|s| s.to_str()) {
                Some("start") => Command::Start,
                Some("restart") => Command::Restart,
                _ => return Err("内部初始化动作只允许 start/restart。".into()),
            };
            initial = true;
        } else if arg == "--background" {
            if options.background {
                return Err("不能重复指定 --background。".into());
            }
            options.background = true;
        } else {
            let command = match arg.to_str() {
                Some("show") => Command::Show,
                Some("status") => Command::Status,
                Some("stop") => Command::Stop,
                Some("quit") => Command::Quit,
                Some("--help" | "-h") => Command::Help,
                Some("--version" | "-V") => Command::Version,
                Some("start") => Command::Start,
                Some("restart") => Command::Restart,
                _ => return Err("不支持的命令行参数；使用 --help 查看用法。".into()),
            };
            if options.command != Command::Launch {
                return Err("每次只能指定一个控制命令。".into());
            }
            options.command = command;
        }
    }
    if initial && (!options.background || options.config_dir.is_none()) {
        return Err("内部初始化动作需要 --background 和绝对 --config-dir。".into());
    }
    if options.background && options.command != Command::Launch && !initial {
        return Err("--background 仅用于启动主实例。".into());
    }
    Ok(options)
}
pub fn run() -> Result<ExitCode, RunError> {
    let options = parse_args(std::env::args_os().skip(1)).map_err(RunError::arguments)?;
    match options.command {
        Command::Help => {
            println!(
                "AI Relay\n  ai-relay [--background] [--config-dir <绝对目录>]\n  ai-relay start|restart|show|status|stop|quit [--config-dir <绝对目录>]\n  ai-relay --help|--version\nstart/restart 必要时创建后台主实例，显式启动代理，不受 auto_start 开关限制；最多等待 40 秒，不自动重试。\n单实例范围为当前 Windows 用户和会话。\nstop 停止代理；quit 请求优雅退出。超时不自动重试。\n退出码：0 成功；1 未运行/未监听；2 参数或配置错误；3 IPC 或操作失败。"
            );
            return Ok(ExitCode::SUCCESS);
        }
        Command::Version => {
            println!("AI Relay {}", env!("CARGO_PKG_VERSION"));
            return Ok(ExitCode::SUCCESS);
        }
        _ => {}
    }
    #[cfg(windows)]
    {
        use instance::{Action, Instance, Request};
        let mut instance = Instance::acquire()?;
        if !instance.primary() {
            let action = match options.command {
                Command::Status => Action::Status,
                Command::Start => Action::Start,
                Command::Restart => Action::Restart,
                Command::Stop => Action::Stop,
                Command::Quit => Action::Quit,
                _ => Action::Show,
            };
            let directory = options
                .config_dir
                .as_ref()
                .map(|p| {
                    p.to_str()
                        .map(str::to_owned)
                        .ok_or_else(|| RunError::arguments("IPC 配置目录必须为有效 Unicode。"))
                })
                .transpose()?;
            let reply = instance.send(Request {
                version: 1,
                action,
                directory,
            })?;
            println!("{}", reply.message);
            return reply_exit_code(action, &reply);
        }
        match options.command {
            Command::Status => {
                println!("当前用户会话没有运行中的主实例。");
                Ok(ExitCode::from(1))
            }
            Command::Start | Command::Restart if !options.background => {
                launch_background(&mut instance, options)
            }
            Command::Stop | Command::Quit => {
                println!("主实例未运行，无需操作。");
                Ok(ExitCode::SUCCESS)
            }
            _ => {
                let mut server = instance.serve()?;
                let result =
                    crate::ui::run(options, server.take_receiver()?, std::sync::Arc::default());
                drop(server);
                drop(instance);
                result.map(|()| ExitCode::SUCCESS).map_err(RunError::from)
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = options;
        Err("AI Relay 仅支持 Windows；其他平台仅用于纯逻辑开发与检查。".into())
    }
}
#[cfg(windows)]
fn launch_background(
    instance: &mut instance::Instance,
    options: Options,
) -> Result<ExitCode, RunError> {
    use std::{
        os::windows::process::CommandExt,
        process::{Command as Process, Stdio},
        time::Duration,
    };
    // Resolve once in the short-lived launcher; the child receives the same absolute
    // selection even if portable markers change while it is starting.
    let selection = crate::windows::paths::discover_directory(options.config_dir.as_deref())
        .map_err(RunError::arguments)?;
    let config = crate::config::load_config(&selection.config_path())
        .map_err(|_| RunError::arguments("无法读取有效配置；请先在 GUI 中完成配置。"))?;
    config
        .validate_for_start()
        .map_err(|_| RunError::arguments("配置未通过启动校验；未创建后台实例。"))?;
    if !config.listen.address.is_loopback() || config.network.shutdown_grace_secs > 30 {
        return Err(RunError::arguments(
            "CLI 启动仅支持回环监听及不超过 30 秒的停止宽限期。",
        ));
    }
    let directory = selection
        .directory
        .to_str()
        .ok_or_else(|| RunError::arguments("IPC 配置目录必须为有效 Unicode。"))?
        .to_owned();
    let exe = std::env::current_exe().map_err(|_| RunError::from("无法定位当前 EXE。"))?;
    if !exe.is_absolute() {
        return Err("当前 EXE 路径不是绝对路径。".into());
    }
    let mut command = Process::new(&exe);
    command
        .args([
            "--background",
            "--initial-action",
            if options.command == Command::Restart {
                "restart"
            } else {
                "start"
            },
            "--config-dir",
        ])
        .arg(&selection.directory)
        .current_dir(exe.parent().ok_or("当前 EXE 缺少父目录。")?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
    instance.release_for_launch();
    // No shell, PATH lookup, credentials, inherited output pipes, or arbitrary action.
    // A racing launcher is resolved by the child's unchanged SID/session mutex + IPC.
    let mut child = command
        .spawn()
        .map_err(|_| RunError::from("创建后台主实例失败。"))?;
    let reply = instance.send_timeout(
        instance::Request {
            version: 1,
            action: instance::Action::Status,
            directory: Some(directory),
        },
        Duration::from_secs(40),
    );
    if let Some(status) = child
        .try_wait()
        .map_err(|_| RunError::from("检查后台启动进程失败。"))?
        && !status.success()
    {
        return Err(RunError {
            message: "后台启动进程失败；请检查配置及主实例状态。".into(),
            code: if status.code() == Some(2) { 2 } else { 3 },
        });
    }
    let reply = reply?;
    println!("{}", reply.message);
    if !reply.ok || !reply.running {
        return Err("后台主实例未确认代理监听；请检查 GUI/日志，不自动重试。".into());
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(windows)]
fn reply_exit_code(
    action: instance::Action,
    reply: &instance::Reply,
) -> Result<ExitCode, RunError> {
    if !reply.ok {
        return Err(RunError {
            message: "IPC 操作失败（详见状态输出）。".into(),
            code: match reply.exit_code {
                Some(2) => 2,
                _ => 3,
            },
        });
    }
    Ok(ExitCode::from(u8::from(
        action == instance::Action::Status && !reply.running,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(args: &[&str]) -> Result<Options, String> {
        parse_args(args.iter().map(OsString::from))
    }
    #[test]
    fn arguments_are_not_silently_ignored() {
        assert_eq!(parse(&[]).unwrap(), Options::default());
        assert!(parse(&["--background"]).unwrap().background);
        assert_eq!(parse(&["status"]).unwrap().command, Command::Status);
        assert_eq!(parse(&["start"]).unwrap().command, Command::Start);
        assert_eq!(parse(&["restart"]).unwrap().command, Command::Restart);
        for args in [
            vec!["start", "restart"],
            vec!["show", "quit"],
            vec!["--background", "stop"],
            vec!["--config-dir"],
            vec!["--config-dir", "relative"],
            vec!["secret-argument"],
        ] {
            assert!(!parse(&args).unwrap_err().contains("secret-argument"));
        }
    }
    #[test]
    fn internal_initial_action_is_restricted() {
        let directory = std::env::temp_dir().into_os_string();
        let options = parse_args([
            OsString::from("--background"),
            OsString::from("--initial-action"),
            OsString::from("start"),
            OsString::from("--config-dir"),
            directory,
        ])
        .unwrap();
        assert!(options.background);
        assert_eq!(options.command, Command::Start);
        for args in [
            vec!["--initial-action", "start"],
            vec!["--background", "--initial-action", "quit"],
            vec!["--background", "--initial-action", "start", "restart"],
            vec![
                "--background",
                "--initial-action",
                "start",
                "--initial-action",
                "start",
            ],
        ] {
            assert!(parse(&args).is_err());
        }
    }
    #[test]
    fn classified_exit_codes() {
        assert_eq!(
            RunError::arguments("invalid").exit_code(),
            ExitCode::from(2)
        );
        assert_eq!(RunError::from("operation").exit_code(), ExitCode::from(3));
        #[cfg(windows)]
        {
            use instance::{Action, Reply};
            let reply = Reply::new(true, false, "stopped");
            assert_eq!(
                reply_exit_code(Action::Status, &reply).unwrap(),
                ExitCode::from(1)
            );
            assert_eq!(
                reply_exit_code(Action::Stop, &reply).unwrap(),
                ExitCode::SUCCESS
            );
            let mut reply = Reply::new(false, false, "invalid config");
            reply.exit_code = Some(2);
            assert_eq!(
                reply_exit_code(Action::Start, &reply)
                    .unwrap_err()
                    .exit_code(),
                ExitCode::from(2)
            );
            reply.exit_code = Some(255);
            assert_eq!(
                reply_exit_code(Action::Start, &reply)
                    .unwrap_err()
                    .exit_code(),
                ExitCode::from(3)
            );
        }
    }
    #[test]
    fn absolute_directory_and_duplicate_option() {
        let path = std::env::temp_dir().into_os_string();
        let option = OsString::from("--config-dir");
        assert!(parse_args([option.clone(), path.clone()]).is_ok());
        assert!(parse_args([option.clone(), path.clone(), option, path]).is_err());
    }
}
