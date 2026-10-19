use super::super::{Command, Controller, Event};
use crate::{
    app::{
        self, Options,
        instance::{Action, Control, Reply, Request, Signals},
    },
    config::{self, Config},
};
use std::{
    fs, io,
    net::{SocketAddr, TcpListener, TcpStream},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const WAIT: Duration = Duration::from_secs(15);

struct StartupFixture {
    directory: PathBuf,
    address: SocketAddr,
    reservation: Option<TcpListener>,
    upstream: TcpListener,
    original_config: Option<Vec<u8>>,
    controller: Option<Controller>,
    ipc: Option<mpsc::SyncSender<Control>>,
    signals: Arc<Signals>,
}

impl StartupFixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let reservation = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = reservation.local_addr().unwrap();
        let upstream = TcpListener::bind("127.0.0.1:0").unwrap();
        upstream.set_nonblocking(true).unwrap();
        let temporary = std::env::temp_dir().canonicalize().unwrap();
        assert!(temporary.is_absolute());
        for _ in 0..128 {
            let directory = temporary.join(format!(
                "ai-relay-controller-startup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&directory) {
                Ok(()) => {
                    return Self {
                        directory,
                        address,
                        reservation: Some(reservation),
                        upstream,
                        original_config: None,
                        controller: None,
                        ipc: None,
                        signals: Arc::new(Signals::default()),
                    };
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("cannot create isolated startup directory: {error}"),
            }
        }
        panic!("cannot allocate a new startup test directory");
    }

    fn config(&self) -> Config {
        let mut config = Config::default();
        config.listen.port = self.address.port();
        config.upstream.url = format!("http://{}", self.upstream.local_addr().unwrap());
        config.network.shutdown_grace_secs = 1;
        config
    }

    fn save(&mut self, config: &Config) {
        assert!(self.controller.is_none() && self.original_config.is_none());
        let path = self.directory.join("config.toml");
        config::save_config_atomic(&path, config).unwrap();
        self.original_config = Some(fs::read(path).unwrap());
    }

    fn write_config(&mut self, bytes: &[u8]) {
        assert!(self.controller.is_none() && self.original_config.is_none());
        fs::write(self.directory.join("config.toml"), bytes).unwrap();
        self.original_config = Some(bytes.to_vec());
    }

    fn spawn(&mut self, command: app::Command, background: bool) {
        assert!(self.controller.is_none());
        let (sender, receiver) = mpsc::sync_channel(4);
        self.ipc = Some(sender);
        drop(self.reservation.take());
        self.controller = Some(
            Controller::spawn(
                Options {
                    config_dir: Some(self.directory.clone()),
                    background,
                    command,
                },
                receiver,
                Arc::clone(&self.signals),
            )
            .unwrap(),
        );
    }

    fn controller(&self) -> &Controller {
        self.controller.as_ref().unwrap()
    }

    fn event(&self) -> Event {
        let deadline = Instant::now() + WAIT;
        loop {
            if let Ok(event) = self.controller().events.try_recv() {
                assert_eq!(event.directory.as_ref(), Some(&self.directory));
                assert!(event.directory_locked);
                // 设置页把“自启”归类为 Windows 登录启动事件，代理消息不能误入该分支。
                assert!(!event.message.contains("自启"), "{}", event.message);
                return event;
            }
            assert!(
                !self.controller().is_finished(),
                "worker exited before event"
            );
            assert!(Instant::now() < deadline, "startup event timed out");
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn control(&self, action: Action) -> Reply {
        let (reply, received) = mpsc::sync_channel(1);
        self.ipc
            .as_ref()
            .unwrap()
            .try_send(Control {
                request: Request {
                    version: 1,
                    action,
                    directory: None,
                },
                reply,
            })
            .unwrap();
        received.recv_timeout(WAIT).unwrap()
    }

    fn expect_running(&self) {
        let event = self.event();
        assert!(event.running, "{}", event.message);
        assert!(event.message.contains("监听"), "{}", event.message);
    }

    fn assert_listening(&self) {
        let reply = self.control(Action::Status);
        assert!(reply.ok && reply.running, "{reply:?}");
        // 只建立并关闭回环 TCP 连接，不发送 HTTP、POST 或任何凭据。
        drop(TcpStream::connect_timeout(&self.address, Duration::from_millis(250)).unwrap());
        assert!(!self.controller().is_finished());
        assert!(!self.signals.quit.load(Ordering::Acquire));
        self.assert_config_unchanged();
    }

    fn assert_stopped(&self) {
        // Status 只有在内部 next 启动命令处理完毕后才会收到回复，可作启动完成屏障。
        let reply = self.control(Action::Status);
        assert!(reply.ok && !reply.running, "{reply:?}");
        assert!(
            TcpStream::connect_timeout(&self.address, Duration::from_millis(100)).is_err(),
            "unexpected listener at {}",
            self.address
        );
        assert!(!self.controller().is_finished());
        assert!(!self.signals.quit.load(Ordering::Acquire));
        self.assert_config_unchanged();
    }

    fn assert_no_events(&self) {
        match self.controller().events.try_recv() {
            Ok(event) => panic!(
                "unexpected event: running={}, {}",
                event.running, event.message
            ),
            Err(mpsc::TryRecvError::Empty) => {}
            Err(error) => panic!("event queue failed: {error}"),
        }
    }

    fn assert_config_unchanged(&self) {
        let path = self.directory.join("config.toml");
        match &self.original_config {
            Some(bytes) => assert_eq!(&fs::read(path).unwrap(), bytes),
            None => assert_eq!(fs::read(path).unwrap_err().kind(), io::ErrorKind::NotFound),
        }
        assert_eq!(
            fs::symlink_metadata(self.directory.join("config.toml.bak"))
                .unwrap_err()
                .kind(),
            io::ErrorKind::NotFound,
            "startup must not create a configuration backup"
        );
    }

    fn wait_for_exit(&self) -> bool {
        let Some(controller) = &self.controller else {
            return true;
        };
        controller.request_exit();
        let deadline = Instant::now() + WAIT;
        while !controller.is_finished() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        controller.is_finished()
    }

    fn finish(&mut self) {
        assert!(self.wait_for_exit(), "controller shutdown timed out");
        drop(self.controller.take());
        drop(self.ipc.take());
        self.assert_config_unchanged();
        match self.upstream.accept() {
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(error) => panic!("cannot inspect local upstream: {error}"),
            Ok(_) => panic!("startup unexpectedly connected to the upstream"),
        }
    }
}

impl Drop for StartupFixture {
    fn drop(&mut self) {
        // 断言 panic 同样先取消并等待；超时保留目录，不能与悬空 worker 竞争删除。
        if !self.wait_for_exit() {
            if thread::panicking() {
                eprintln!(
                    "controller shutdown timed out; retained {}",
                    self.directory.display()
                );
            } else {
                panic!(
                    "controller shutdown timed out; retained {}",
                    self.directory.display()
                );
            }
            return;
        }
        drop(self.controller.take());
        drop(self.ipc.take());
        if let Err(error) = fs::remove_dir_all(&self.directory) {
            if thread::panicking() {
                eprintln!("startup fixture cleanup failed: {error}");
            } else {
                panic!("startup fixture cleanup failed: {error}");
            }
        }
    }
}

#[test]
fn launch_starts_saved_proxy_with_implicit_or_explicit_auto_start() {
    for omit_proxy in [true, false] {
        let mut fixture = StartupFixture::new();
        let saved = fixture.config();
        if omit_proxy {
            let text = format!(
                "# Keep this legacy layout unchanged.\r\nschema_version = 1\r\n\
                 [upstream]\r\nurl = \"{}\"\r\n\
                 [listen]\r\naddress = \"127.0.0.1\"\r\nport = {}\r\n\
                 [network]\r\nshutdown_grace_secs = 1\r\n",
                saved.upstream.url, saved.listen.port
            );
            fixture.write_config(text.as_bytes());
        } else {
            assert!(saved.proxy.auto_start);
            fixture.save(&saved);
        }
        fixture.spawn(app::Command::Launch, false);
        let loaded = fixture.event();
        assert_eq!(loaded.config.as_ref(), Some(&saved));
        assert!(!loaded.running);
        assert!(loaded.message.contains("正在"), "{}", loaded.message);
        assert!(loaded.message.contains("已保存"), "{}", loaded.message);
        assert!(loaded.message.contains("自动启动"), "{}", loaded.message);
        // 不发送 GUI/IPC Start：必须由普通 Launch 的 Load 分支真正启动。
        fixture.expect_running();
        fixture.assert_listening();
        fixture.finish();
    }
}

#[test]
fn launch_preserves_opt_out_but_manual_and_ipc_start_still_work() {
    let mut fixture = StartupFixture::new();
    let mut saved = fixture.config();
    saved.proxy.auto_start = false;
    fixture.save(&saved);
    fixture.spawn(app::Command::Launch, false);
    let loaded = fixture.event();
    assert_eq!(loaded.config.as_ref(), Some(&saved));
    assert!(!loaded.running);
    assert!(loaded.message.contains("关闭"), "{}", loaded.message);
    assert!(loaded.message.contains("自动启动"), "{}", loaded.message);
    fixture.assert_stopped();
    fixture.assert_no_events();

    // 兼容参数是未配置的默认草稿；手动启动也只能使用已保存会话。
    fixture
        .controller()
        .send(Command::Start(Box::default()))
        .unwrap();
    fixture.expect_running();
    fixture.assert_listening();

    let reply = fixture.control(Action::Stop);
    assert!(reply.ok && !reply.running, "{reply:?}");
    let stopping = fixture.event();
    assert!(stopping.running && stopping.message.contains("停止"));
    let stopped = fixture.event();
    assert!(!stopped.running && stopped.message.contains("停止"));
    fixture.assert_stopped();

    for action in [Action::Start, Action::Restart] {
        let reply = fixture.control(action);
        assert!(reply.ok && reply.running, "{reply:?}");
        fixture.expect_running();
        fixture.assert_listening();
    }
    fixture.finish();
}

#[test]
fn launch_without_usable_saved_config_never_listens_or_rewrites_config() {
    #[derive(Clone, Copy)]
    enum Input {
        Missing,
        Corrupt,
        EmptyUpstream,
    }

    for input in [Input::Missing, Input::Corrupt, Input::EmptyUpstream] {
        let mut fixture = StartupFixture::new();
        let expected = match input {
            Input::Missing => Some(Config::default()),
            Input::Corrupt => {
                fixture.write_config(b"# Do not overwrite this broken file.\nschema_version = [\n");
                None
            }
            Input::EmptyUpstream => {
                let mut saved = fixture.config();
                saved.upstream.url.clear();
                fixture.save(&saved);
                Some(saved)
            }
        };
        fixture.spawn(app::Command::Launch, matches!(input, Input::Corrupt));
        let loaded = fixture.event();
        assert!(!loaded.running, "{}", loaded.message);
        assert_eq!(loaded.config, expected);
        match input {
            Input::Missing => assert!(
                loaded.message.contains("首次") || loaded.message.contains("未配置"),
                "{}",
                loaded.message
            ),
            Input::Corrupt => assert!(loaded.message.contains("失败"), "{}", loaded.message),
            Input::EmptyUpstream => assert!(
                loaded.message.contains("上游") && loaded.message.contains("未配置"),
                "{}",
                loaded.message
            ),
        }
        fixture.assert_stopped();
        fixture.assert_no_events();
        assert_eq!(
            fixture.signals.show.load(Ordering::Acquire),
            matches!(input, Input::Corrupt)
        );
        fixture.finish();
    }
}

#[test]
fn launch_and_cli_never_bind_unconfirmed_non_loopback_config() {
    for command in [
        app::Command::Launch,
        app::Command::Start,
        app::Command::Restart,
    ] {
        let mut fixture = StartupFixture::new();
        let mut saved = fixture.config();
        saved.listen.address = "0.0.0.0".parse().unwrap();
        saved.proxy.auto_start = command == app::Command::Launch;
        fixture.save(&saved);
        fixture.spawn(command, command != app::Command::Launch);
        let loaded = fixture.event();
        assert!(!loaded.running);
        assert_eq!(loaded.config.as_ref(), Some(&saved));
        if command == app::Command::Launch {
            assert!(loaded.message.contains("非回环"), "{}", loaded.message);
            assert!(loaded.message.contains("手动"), "{}", loaded.message);
            assert!(loaded.message.contains("风险"), "{}", loaded.message);
            fixture.assert_stopped();
            fixture.assert_no_events();
            // 已有主实例的 CLI 同样不能绕过一次性 GUI 风险确认。
            for action in [Action::Start, Action::Restart] {
                let reply = fixture.control(action);
                assert!(!reply.ok && !reply.running, "{reply:?}");
                assert!(reply.message.contains("非回环") && reply.message.contains("风险"));
                let failed = fixture.event();
                assert!(!failed.running);
                assert!(failed.message.contains("非回环") && failed.message.contains("风险"));
            }
        } else {
            let failed = fixture.event();
            assert!(!failed.running);
            assert!(failed.message.contains("非回环") && failed.message.contains("风险"));
        }
        fixture.assert_stopped();
        fixture.assert_no_events();
        assert!(fixture.signals.show.load(Ordering::Acquire));
        fixture.finish();
    }
}

#[test]
fn occupied_port_reports_failure_shows_gui_and_does_not_retry_when_released() {
    let mut fixture = StartupFixture::new();
    let occupied = fixture.reservation.take().unwrap();
    let saved = fixture.config();
    fixture.save(&saved);
    fixture.spawn(app::Command::Launch, true);
    let loaded = fixture.event();
    assert!(!loaded.running);
    assert_eq!(loaded.config.as_ref(), Some(&saved));
    assert!(loaded.message.contains("自动启动"), "{}", loaded.message);
    let failed = fixture.event();
    assert!(!failed.running);
    assert!(failed.message.contains("失败"), "{}", failed.message);
    assert!(failed.message.contains("监听") || failed.message.contains("端口"));
    assert!(fixture.signals.show.load(Ordering::Acquire));
    let reply = fixture.control(Action::Status);
    assert!(reply.ok && !reply.running, "{reply:?}");
    fixture.assert_config_unchanged();

    drop(occupied);
    // 保持端口空闲，跨越多个命令轮询和至少一个概览 tick；不靠再次占用端口阻止重试。
    let deadline = Instant::now() + Duration::from_millis(1200);
    loop {
        fixture.assert_stopped();
        fixture.assert_no_events();
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(25));
    }
    fixture
        .controller()
        .send(Command::Start(Box::default()))
        .unwrap();
    fixture.expect_running();
    fixture.assert_listening();
    fixture.finish();
}

#[test]
fn initial_cli_start_and_restart_override_saved_auto_start_opt_out() {
    for command in [app::Command::Start, app::Command::Restart] {
        let mut fixture = StartupFixture::new();
        let mut saved = fixture.config();
        saved.proxy.auto_start = false;
        fixture.save(&saved);
        fixture.spawn(command, true);
        let loaded = fixture.event();
        assert!(!loaded.running);
        assert_eq!(loaded.config.as_ref(), Some(&saved));
        fixture.expect_running();
        fixture.assert_listening();
        assert!(!fixture.signals.show.load(Ordering::Acquire));
        fixture.finish();
    }
}
