use super::plugin_thread_main;
use crate::screen::ScreenInstruction;
use crate::{channels::SenderWithContext, thread_bus::Bus, ServerInstruction};
use insta::assert_snapshot;
use std::path::PathBuf;
use tempfile::tempdir;
use wasmer::Store;
use zellij_utils::data::{Event, Key, PluginCapabilities};
use zellij_utils::errors::ErrorContext;
use zellij_utils::input::layout::{Layout, RunPlugin, RunPluginLocation};
use zellij_utils::input::plugins::PluginsConfig;
use zellij_utils::ipc::ClientAttributes;
use zellij_utils::lazy_static::lazy_static;
use zellij_utils::pane_size::Size;

use crate::background_jobs::BackgroundJob;
use crate::pty_writer::PtyWriteInstruction;
use std::env::set_var;
use std::sync::{Arc, Mutex};

use crate::{plugins::PluginInstruction, pty::PtyInstruction};

use zellij_utils::channels::{self, ChannelWithContext, Receiver};

macro_rules! log_actions_in_thread {
    ( $arc_mutex_log:expr, $exit_event:path, $receiver:expr, $exit_after_count:expr ) => {
        std::thread::Builder::new()
            .name("logger thread".to_string())
            .spawn({
                let log = $arc_mutex_log.clone();
                let mut exit_event_count = 0;
                move || loop {
                    let (event, _err_ctx) = $receiver
                        .recv()
                        .expect("failed to receive event on channel");
                    match event {
                        $exit_event(..) => {
                            exit_event_count += 1;
                            log.lock().unwrap().push(event);
                            if exit_event_count == $exit_after_count {
                                break;
                            }
                        },
                        _ => {
                            log.lock().unwrap().push(event);
                        },
                    }
                }
            })
            .unwrap()
    };
}

macro_rules! log_actions_in_thread_naked_variant {
    ( $arc_mutex_log:expr, $exit_event:path, $receiver:expr, $exit_after_count:expr ) => {
        std::thread::Builder::new()
            .name("logger thread".to_string())
            .spawn({
                let log = $arc_mutex_log.clone();
                let mut exit_event_count = 0;
                move || loop {
                    let (event, _err_ctx) = $receiver
                        .recv()
                        .expect("failed to receive event on channel");
                    match event {
                        $exit_event => {
                            exit_event_count += 1;
                            log.lock().unwrap().push(event);
                            if exit_event_count == $exit_after_count {
                                break;
                            }
                        },
                        _ => {
                            log.lock().unwrap().push(event);
                        },
                    }
                }
            })
            .unwrap()
    };
}

fn create_plugin_thread(
    zellij_cwd: Option<PathBuf>,
) -> (
    SenderWithContext<PluginInstruction>,
    Receiver<(ScreenInstruction, ErrorContext)>,
    Box<dyn FnMut()>,
) {
    let zellij_cwd = zellij_cwd.unwrap_or_else(|| PathBuf::from("."));
    let (to_server, _server_receiver): ChannelWithContext<ServerInstruction> =
        channels::bounded(50);
    let to_server = SenderWithContext::new(to_server);

    let (to_screen, screen_receiver): ChannelWithContext<ScreenInstruction> = channels::unbounded();
    let to_screen = SenderWithContext::new(to_screen);

    let (to_plugin, plugin_receiver): ChannelWithContext<PluginInstruction> = channels::unbounded();
    let to_plugin = SenderWithContext::new(to_plugin);
    let (to_pty, _pty_receiver): ChannelWithContext<PtyInstruction> = channels::unbounded();
    let to_pty = SenderWithContext::new(to_pty);

    let (to_pty_writer, _pty_writer_receiver): ChannelWithContext<PtyWriteInstruction> =
        channels::unbounded();
    let to_pty_writer = SenderWithContext::new(to_pty_writer);

    let (to_background_jobs, _background_jobs_receiver): ChannelWithContext<BackgroundJob> =
        channels::unbounded();
    let to_background_jobs = SenderWithContext::new(to_background_jobs);

    let plugin_bus = Bus::new(
        vec![plugin_receiver],
        Some(&to_screen),
        Some(&to_pty),
        Some(&to_plugin),
        Some(&to_server),
        Some(&to_pty_writer),
        Some(&to_background_jobs),
        None,
    )
    .should_silently_fail();
    let store = Store::new(&wasmer::Universal::new(wasmer::Singlepass::default()).engine());
    let data_dir = PathBuf::from(tempdir().unwrap().path());
    let default_shell = PathBuf::from(".");
    let plugin_capabilities = PluginCapabilities::default();
    let client_attributes = ClientAttributes::default();
    let default_shell_action = None; // TODO: change me
    let _plugin_thread = std::thread::Builder::new()
        .name("plugin_thread".to_string())
        .spawn(move || {
            set_var("ZELLIJ_SESSION_NAME", "zellij-test");
            plugin_thread_main(
                plugin_bus,
                store,
                data_dir,
                PluginsConfig::default(),
                Box::new(Layout::default()),
                default_shell,
                zellij_cwd,
                plugin_capabilities,
                client_attributes,
                default_shell_action,
            )
            .expect("TEST")
        })
        .unwrap();
    let teardown = {
        let to_plugin = to_plugin.clone();
        move || {
            let _ = to_pty.send(PtyInstruction::Exit);
            let _ = to_pty_writer.send(PtyWriteInstruction::Exit);
            let _ = to_screen.send(ScreenInstruction::Exit);
            let _ = to_server.send(ServerInstruction::KillSession);
            let _ = to_plugin.send(PluginInstruction::Exit);
            std::thread::sleep(std::time::Duration::from_millis(100)); // we need to do this
                                                                       // otherwise there are race
                                                                       // conditions with removing
                                                                       // the plugin cache
        }
    };
    (to_plugin, screen_receiver, Box::new(teardown))
}

fn create_plugin_thread_with_server_receiver(
    zellij_cwd: Option<PathBuf>,
) -> (
    SenderWithContext<PluginInstruction>,
    Receiver<(ServerInstruction, ErrorContext)>,
    Box<dyn FnMut()>,
) {
    let zellij_cwd = zellij_cwd.unwrap_or_else(|| PathBuf::from("."));
    let (to_server, server_receiver): ChannelWithContext<ServerInstruction> = channels::bounded(50);
    let to_server = SenderWithContext::new(to_server);

    let (to_screen, _screen_receiver): ChannelWithContext<ScreenInstruction> =
        channels::unbounded();
    let to_screen = SenderWithContext::new(to_screen);

    let (to_plugin, plugin_receiver): ChannelWithContext<PluginInstruction> = channels::unbounded();
    let to_plugin = SenderWithContext::new(to_plugin);
    let (to_pty, _pty_receiver): ChannelWithContext<PtyInstruction> = channels::unbounded();
    let to_pty = SenderWithContext::new(to_pty);

    let (to_pty_writer, _pty_writer_receiver): ChannelWithContext<PtyWriteInstruction> =
        channels::unbounded();
    let to_pty_writer = SenderWithContext::new(to_pty_writer);

    let (to_background_jobs, _background_jobs_receiver): ChannelWithContext<BackgroundJob> =
        channels::unbounded();
    let to_background_jobs = SenderWithContext::new(to_background_jobs);

    let plugin_bus = Bus::new(
        vec![plugin_receiver],
        Some(&to_screen),
        Some(&to_pty),
        Some(&to_plugin),
        Some(&to_server),
        Some(&to_pty_writer),
        Some(&to_background_jobs),
        None,
    )
    .should_silently_fail();
    let store = Store::new(&wasmer::Universal::new(wasmer::Singlepass::default()).engine());
    let data_dir = PathBuf::from(tempdir().unwrap().path());
    let default_shell = PathBuf::from(".");
    let plugin_capabilities = PluginCapabilities::default();
    let client_attributes = ClientAttributes::default();
    let default_shell_action = None; // TODO: change me
    let _plugin_thread = std::thread::Builder::new()
        .name("plugin_thread".to_string())
        .spawn(move || {
            set_var("ZELLIJ_SESSION_NAME", "zellij-test");
            plugin_thread_main(
                plugin_bus,
                store,
                data_dir,
                PluginsConfig::default(),
                Box::new(Layout::default()),
                default_shell,
                zellij_cwd,
                plugin_capabilities,
                client_attributes,
                default_shell_action,
            )
            .expect("TEST")
        })
        .unwrap();
    let teardown = {
        let to_plugin = to_plugin.clone();
        move || {
            let _ = to_pty.send(PtyInstruction::Exit);
            let _ = to_pty_writer.send(PtyWriteInstruction::Exit);
            let _ = to_screen.send(ScreenInstruction::Exit);
            let _ = to_server.send(ServerInstruction::KillSession);
            let _ = to_plugin.send(PluginInstruction::Exit);
            std::thread::sleep(std::time::Duration::from_millis(100)); // we need to do this
                                                                       // otherwise there are race
                                                                       // conditions with removing
                                                                       // the plugin cache
        }
    };
    (to_plugin, server_receiver, Box::new(teardown))
}

fn create_plugin_thread_with_pty_receiver(
    zellij_cwd: Option<PathBuf>,
) -> (
    SenderWithContext<PluginInstruction>,
    Receiver<(PtyInstruction, ErrorContext)>,
    Box<dyn FnMut()>,
) {
    let zellij_cwd = zellij_cwd.unwrap_or_else(|| PathBuf::from("."));
    let (to_server, _server_receiver): ChannelWithContext<ServerInstruction> =
        channels::bounded(50);
    let to_server = SenderWithContext::new(to_server);

    let (to_screen, _screen_receiver): ChannelWithContext<ScreenInstruction> =
        channels::unbounded();
    let to_screen = SenderWithContext::new(to_screen);

    let (to_plugin, plugin_receiver): ChannelWithContext<PluginInstruction> = channels::unbounded();
    let to_plugin = SenderWithContext::new(to_plugin);
    let (to_pty, pty_receiver): ChannelWithContext<PtyInstruction> = channels::unbounded();
    let to_pty = SenderWithContext::new(to_pty);

    let (to_pty_writer, _pty_writer_receiver): ChannelWithContext<PtyWriteInstruction> =
        channels::unbounded();
    let to_pty_writer = SenderWithContext::new(to_pty_writer);

    let (to_background_jobs, _background_jobs_receiver): ChannelWithContext<BackgroundJob> =
        channels::unbounded();
    let to_background_jobs = SenderWithContext::new(to_background_jobs);

    let plugin_bus = Bus::new(
        vec![plugin_receiver],
        Some(&to_screen),
        Some(&to_pty),
        Some(&to_plugin),
        Some(&to_server),
        Some(&to_pty_writer),
        Some(&to_background_jobs),
        None,
    )
    .should_silently_fail();
    let store = Store::new(&wasmer::Universal::new(wasmer::Singlepass::default()).engine());
    let data_dir = PathBuf::from(tempdir().unwrap().path());
    let default_shell = PathBuf::from(".");
    let plugin_capabilities = PluginCapabilities::default();
    let client_attributes = ClientAttributes::default();
    let default_shell_action = None; // TODO: change me
    let _plugin_thread = std::thread::Builder::new()
        .name("plugin_thread".to_string())
        .spawn(move || {
            set_var("ZELLIJ_SESSION_NAME", "zellij-test");
            plugin_thread_main(
                plugin_bus,
                store,
                data_dir,
                PluginsConfig::default(),
                Box::new(Layout::default()),
                default_shell,
                zellij_cwd,
                plugin_capabilities,
                client_attributes,
                default_shell_action,
            )
            .expect("TEST")
        })
        .unwrap();
    let teardown = {
        let to_plugin = to_plugin.clone();
        move || {
            let _ = to_pty.send(PtyInstruction::Exit);
            let _ = to_pty_writer.send(PtyWriteInstruction::Exit);
            let _ = to_screen.send(ScreenInstruction::Exit);
            let _ = to_server.send(ServerInstruction::KillSession);
            let _ = to_plugin.send(PluginInstruction::Exit);
            std::thread::sleep(std::time::Duration::from_millis(100)); // we need to do this
                                                                       // otherwise there are race
                                                                       // conditions with removing
                                                                       // the plugin cache
        }
    };
    (to_plugin, pty_receiver, Box::new(teardown))
}

lazy_static! {
    static ref PLUGIN_FIXTURE: String = format!(
        // to populate this file, make sure to run the build-e2e CI job
        // (or compile the fixture plugin and copy the resulting .wasm blob to the below location)
        "{}/../target/e2e-data/plugins/fixture-plugin-for-tests.wasm",
        std::env::var_os("CARGO_MANIFEST_DIR")
            .unwrap()
            .to_string_lossy()
    );
}

#[test]
#[ignore]
pub fn load_new_plugin_from_hd() {
    // here we load our fixture plugin into the plugin thread, and then send it an update message
    // expecting tha thte plugin will log the received event and render it later after the update
    // message (this is what the fixture plugin does)
    // we then listen on our mock screen receiver to make sure we got a PluginBytes instruction
    // that contains said render, and assert against it
    let (plugin_thread_sender, screen_receiver, mut teardown) = create_plugin_thread(None);
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PluginBytes,
        screen_receiver,
        2
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::InputReceived,
    )])); // will be cached and sent to the plugin once it's loaded
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let plugin_bytes_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PluginBytes(plugin_bytes) = i {
                for (plugin_id, client_id, plugin_bytes) in plugin_bytes {
                    let plugin_bytes = String::from_utf8_lossy(plugin_bytes).to_string();
                    if plugin_bytes.contains("InputReceived") {
                        return Some((*plugin_id, *client_id, plugin_bytes));
                    }
                }
            }
            None
        });
    assert_snapshot!(format!("{:#?}", plugin_bytes_event));
}

#[test]
#[ignore]
pub fn plugin_workers() {
    let (plugin_thread_sender, screen_receiver, mut teardown) = create_plugin_thread(None);
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PluginBytes,
        screen_receiver,
        3
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    // we send a SystemClipboardFailure to trigger the custom handler in the fixture plugin that
    // will send a message to the worker and in turn back to the plugin to be rendered, so we know
    // that this cycle is working
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::SystemClipboardFailure,
    )])); // will be cached and sent to the plugin once it's loaded
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let plugin_bytes_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PluginBytes(plugin_bytes) = i {
                for (plugin_id, client_id, plugin_bytes) in plugin_bytes {
                    let plugin_bytes = String::from_utf8_lossy(plugin_bytes).to_string();
                    if plugin_bytes.contains("Payload from worker") {
                        return Some((*plugin_id, *client_id, plugin_bytes));
                    }
                }
            }
            None
        });
    assert_snapshot!(format!("{:#?}", plugin_bytes_event));
}

#[test]
#[ignore]
pub fn plugin_workers_persist_state() {
    let (plugin_thread_sender, screen_receiver, mut teardown) = create_plugin_thread(None);
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PluginBytes,
        screen_receiver,
        5
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    // we send a SystemClipboardFailure to trigger the custom handler in the fixture plugin that
    // will send a message to the worker and in turn back to the plugin to be rendered, so we know
    // that this cycle is working
    // we do this a second time so that the worker will log the first message on its own state and
    // then send us the "received 2 messages" indication we check for below, letting us know it
    // managed to persist its own state and act upon it
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::SystemClipboardFailure,
    )]));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::SystemClipboardFailure,
    )]));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let plugin_bytes_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PluginBytes(plugin_bytes) = i {
                for (plugin_id, client_id, plugin_bytes) in plugin_bytes {
                    let plugin_bytes = String::from_utf8_lossy(plugin_bytes).to_string();
                    if plugin_bytes.contains("received 2 messages") {
                        return Some((*plugin_id, *client_id, plugin_bytes));
                    }
                }
            }
            None
        });
    assert_snapshot!(format!("{:#?}", plugin_bytes_event));
}

#[test]
#[ignore]
pub fn can_subscribe_to_hd_events() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PluginBytes,
        screen_receiver,
        2
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    // extra long time because we only start the fs watcher on plugin load
    std::thread::sleep(std::time::Duration::from_millis(5000));
    std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .open(PathBuf::from(temp_folder.path()).join("test1"))
        .unwrap();
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let plugin_bytes_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PluginBytes(plugin_bytes) = i {
                for (plugin_id, client_id, plugin_bytes) in plugin_bytes {
                    let plugin_bytes = String::from_utf8_lossy(plugin_bytes).to_string();
                    if plugin_bytes.contains("FileSystemCreate") {
                        return Some((*plugin_id, *client_id, plugin_bytes));
                    }
                }
            }
            None
        });
    assert!(plugin_bytes_event.is_some());
}

#[test]
#[ignore]
pub fn switch_to_mode_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ChangeMode,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('a')), // this triggers a SwitchToMode(Tab) command in the fixture
                                    // plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let switch_to_mode_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ChangeMode(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", switch_to_mode_event));
}

#[test]
#[ignore]
pub fn new_tabs_with_layout_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::NewTab,
        screen_receiver,
        2
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('b')), // this triggers a new_tabs_with_layout command in the fixture
                                    // plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let first_new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::NewTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    let second_new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find_map(|i| {
            if let ScreenInstruction::NewTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", first_new_tab_event));
    assert_snapshot!(format!("{:#?}", second_new_tab_event));
}

#[test]
#[ignore]
pub fn new_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::NewTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('c')), // this triggers a new_tab command in the fixture
                                    // plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::NewTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn go_to_next_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::SwitchTabNext,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('d')), // this triggers the event in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::SwitchTabNext(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn go_to_previous_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::SwitchTabPrev,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('e')), // this triggers the event in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::SwitchTabPrev(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn resize_focused_pane_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::Resize,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('f')), // this triggers the event in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::Resize(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn resize_focused_pane_with_direction_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::Resize,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('g')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::Resize(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn focus_next_pane_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::FocusNextPane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('h')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::FocusNextPane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn focus_previous_pane_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::FocusPreviousPane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('i')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::FocusPreviousPane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn move_focus_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::MoveFocusLeft,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('j')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::MoveFocusLeft(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn move_focus_or_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::MoveFocusLeftOrPreviousTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('k')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::MoveFocusLeftOrPreviousTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn edit_scrollback_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::EditScrollback,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('m')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::EditScrollback(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn write_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::WriteCharacter,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('n')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::WriteCharacter(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn write_chars_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::WriteCharacter,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('o')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::WriteCharacter(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn toggle_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ToggleTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('p')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ToggleTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn move_pane_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::MovePane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('q')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::MovePane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn move_pane_with_direction_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::MovePaneLeft,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('r')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::MovePaneLeft(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn clear_screen_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ClearScreen,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('s')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ClearScreen(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn scroll_up_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ScrollUp,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('t')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ScrollUp(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn scroll_down_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ScrollDown,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('u')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ScrollDown(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn scroll_to_top_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ScrollToTop,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('v')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ScrollToTop(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn scroll_to_bottom_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ScrollToBottom,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('w')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ScrollToBottom(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn page_scroll_up_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PageScrollUp,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('x')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PageScrollUp(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn page_scroll_down_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PageScrollDown,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('y')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PageScrollDown(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn toggle_focus_fullscreen_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ToggleActiveTerminalFullscreen,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('z')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ToggleActiveTerminalFullscreen(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn toggle_pane_frames_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread_naked_variant!(
        received_screen_instructions,
        ScreenInstruction::TogglePaneFrames,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('1')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::TogglePaneFrames = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn toggle_pane_embed_or_eject_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::TogglePaneEmbedOrFloating,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('2')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::TogglePaneEmbedOrFloating(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn undo_rename_pane_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::UndoRenamePane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('3')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::UndoRenamePane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn close_focus_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::CloseFocusedPane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('4')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::CloseFocusedPane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn toggle_active_tab_sync_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::ToggleActiveSyncTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('5')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::ToggleActiveSyncTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn close_focused_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::CloseTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('6')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::CloseTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn undo_rename_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::UndoRenameTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('7')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::UndoRenameTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn previous_swap_layout_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::PreviousSwapLayout,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('a')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::PreviousSwapLayout(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn next_swap_layout_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::NextSwapLayout,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('b')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::NextSwapLayout(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn go_to_tab_name_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::GoToTabName,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('c')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::GoToTabName(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn focus_or_create_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::GoToTabName,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('d')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::GoToTabName(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn go_to_tab() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::GoToTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('e')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::GoToTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn start_or_reload_plugin() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::StartOrReloadPluginPane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('f')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::StartOrReloadPluginPane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn quit_zellij_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, server_receiver, mut teardown) =
        create_plugin_thread_with_server_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_server_instruction = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_server_instruction,
        ServerInstruction::ClientExit,
        server_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('8')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_server_instruction
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ServerInstruction::ClientExit(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn detach_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, server_receiver, mut teardown) =
        create_plugin_thread_with_server_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_server_instruction = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_server_instruction,
        ServerInstruction::DetachSession,
        server_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Char('l')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_server_instruction
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ServerInstruction::DetachSession(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_file_floating_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('h')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_file_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('g')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_file_with_line_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('i')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_file_with_line_floating_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('j')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_terminal_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('k')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_terminal_floating_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('l')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_command_pane_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('m')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn open_command_pane_floating_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, pty_receiver, mut teardown) =
        create_plugin_thread_with_pty_receiver(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        PtyInstruction::SpawnTerminal,
        pty_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('n')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let PtyInstruction::SpawnTerminal(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn switch_to_tab_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::GoToTab,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('o')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::GoToTab(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}

#[test]
#[ignore]
pub fn hide_self_plugin_command() {
    let temp_folder = tempdir().unwrap(); // placed explicitly in the test scope because its
                                          // destructor removes the directory
    let plugin_host_folder = PathBuf::from(temp_folder.path());
    let (plugin_thread_sender, screen_receiver, mut teardown) =
        create_plugin_thread(Some(plugin_host_folder));
    let plugin_should_float = Some(false);
    let plugin_title = Some("test_plugin".to_owned());
    let run_plugin = RunPlugin {
        _allow_exec_host_cmd: false,
        location: RunPluginLocation::File(PathBuf::from(&*PLUGIN_FIXTURE)),
    };
    let tab_index = 1;
    let client_id = 1;
    let size = Size {
        cols: 121,
        rows: 20,
    };
    let received_screen_instructions = Arc::new(Mutex::new(vec![]));
    let screen_thread = log_actions_in_thread!(
        received_screen_instructions,
        ScreenInstruction::SuppressPane,
        screen_receiver,
        1
    );

    let _ = plugin_thread_sender.send(PluginInstruction::AddClient(client_id));
    let _ = plugin_thread_sender.send(PluginInstruction::Load(
        plugin_should_float,
        plugin_title,
        run_plugin,
        tab_index,
        client_id,
        size,
    ));
    let _ = plugin_thread_sender.send(PluginInstruction::Update(vec![(
        None,
        Some(client_id),
        Event::Key(Key::Ctrl('p')), // this triggers the enent in the fixture plugin
    )]));
    std::thread::sleep(std::time::Duration::from_millis(100));
    screen_thread.join().unwrap(); // this might take a while if the cache is cold
    teardown();
    let new_tab_event = received_screen_instructions
        .lock()
        .unwrap()
        .iter()
        .find_map(|i| {
            if let ScreenInstruction::SuppressPane(..) = i {
                Some(i.clone())
            } else {
                None
            }
        })
        .clone();
    assert_snapshot!(format!("{:#?}", new_tab_event));
}
