use crate::config::Config;
use crate::config::Mode;
pub use crate::directory_buffer::DirectoryBuffer;
use crate::explorer;
use crate::input::{InputOperation, Key};
use crate::lua;
pub use crate::msg::in_::external::Command;
pub use crate::msg::in_::external::ExplorerConfig;
pub use crate::msg::in_::external::NodeFilter;
pub use crate::msg::in_::external::NodeFilterApplicable;
pub use crate::msg::in_::external::NodeSorter;
pub use crate::msg::in_::external::NodeSorterApplicable;
pub use crate::msg::in_::ExternalMsg;
pub use crate::msg::in_::InternalMsg;
pub use crate::msg::in_::MsgIn;
pub use crate::msg::out::MsgOut;
pub use crate::node::Node;
pub use crate::node::ResolvedNode;
pub use crate::pipe::Pipe;
use crate::ui::Layout;
use anyhow::{bail, Result};
use chrono::{DateTime, Local};
use indexmap::set::IndexSet;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::path::PathBuf;
use tui_input::{Input, InputRequest};

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const TEMPLATE_TABLE_ROW: &str = "TEMPLATE_TABLE_ROW";
pub const UNSUPPORTED_STR: &str = "???";

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub msg: MsgIn,
    pub key: Option<Key>,
}

impl Task {
    pub fn new(msg: MsgIn, key: Option<Key>) -> Self {
        Self { msg, key }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Info,
    Warning,
    Success,
    Error,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Log {
    pub level: LogLevel,
    pub message: String,
    pub created_at: DateTime<Local>,
}

impl Log {
    pub fn new(level: LogLevel, message: String) -> Self {
        Self {
            level,
            message,
            created_at: Local::now(),
        }
    }
}

impl std::fmt::Display for Log {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let level_str = match self.level {
            LogLevel::Info => "INFO   ",
            LogLevel::Warning => "WARNING",
            LogLevel::Success => "SUCCESS",
            LogLevel::Error => "ERROR  ",
        };
        write!(f, "[{}] {} {}", &self.created_at, level_str, &self.message)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum HelpMenuLine {
    KeyMap(String, Vec<String>, String),
    Paragraph(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct History {
    pub loc: usize,
    pub paths: Vec<String>,
}

impl History {
    fn push(mut self, path: String) -> Self {
        if self.peek() != Some(&path) {
            self.paths = self.paths.into_iter().take(self.loc + 1).collect();
            self.paths.push(path);
            self.loc = self.paths.len().max(1) - 1;
        }
        self
    }

    fn visit_last(mut self) -> Self {
        self.loc = self.loc.max(1) - 1;
        self
    }

    fn visit_next(mut self) -> Self {
        self.loc = (self.loc + 1).min(self.paths.len().max(1) - 1);
        self
    }

    fn peek(&self) -> Option<&String> {
        self.paths.get(self.loc)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LuaContextHeavy {
    pub version: String,
    pub pwd: String,
    pub focused_node: Option<Node>,
    pub directory_buffer: Option<DirectoryBuffer>,
    pub selection: IndexSet<Node>,
    pub mode: Mode,
    pub layout: Layout,
    pub input_buffer: Option<String>,
    pub pid: u32,
    pub session_path: String,
    pub explorer_config: ExplorerConfig,
    pub history: History,
    pub last_modes: Vec<Mode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LuaContextLight {
    pub version: String,
    pub pwd: String,
    pub focused_node: Option<Node>,
    pub selection: IndexSet<Node>,
    pub mode: Mode,
    pub layout: Layout,
    pub input_buffer: Option<String>,
    pub pid: u32,
    pub session_path: String,
    pub explorer_config: ExplorerConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub version: String,
    pub config: Config,
    pub pwd: String,
    pub directory_buffer: Option<DirectoryBuffer>,
    pub last_focus: HashMap<String, Option<String>>,
    pub selection: IndexSet<Node>,
    pub msg_out: VecDeque<MsgOut>,
    pub mode: Mode,
    pub layout: Layout,
    pub input: Option<Input>,
    pub pid: u32,
    pub session_path: String,
    pub pipe: Pipe,
    pub explorer_config: ExplorerConfig,
    pub logs: Vec<Log>,
    pub logs_hidden: bool,
    pub history: History,
    pub last_modes: Vec<Mode>,
}

impl App {
    pub fn create(
        pwd: PathBuf,
        lua: &mlua::Lua,
        config_file: Option<PathBuf>,
        extra_config_files: Vec<PathBuf>,
    ) -> Result<Self> {
        let mut config = lua::init(lua)?;

        let config_file = if let Some(path) = config_file {
            Some(path)
        } else if let Some(dir) = dirs::home_dir() {
            let path = dir.join(".config").join("xplr").join("init.lua");
            if path.exists() {
                Some(path)
            } else {
                None
            }
        } else {
            let path =
                PathBuf::from("/").join("etc").join("xplr").join("init.lua");
            if path.exists() {
                Some(path)
            } else {
                None
            }
        };

        let config_files = config_file
            .into_iter()
            .chain(extra_config_files.into_iter());

        let mut load_errs = vec![];
        for config_file in config_files {
            match lua::extend(lua, &config_file.to_string_lossy().to_string()) {
                Ok(c) => {
                    config = c;
                }
                Err(e) => {
                    load_errs.push(e.to_string());
                }
            }
        }

        let mode = match config.modes.get(
            &config
                .general
                .initial_mode
                .to_owned()
                .unwrap_or_else(|| "default".into()),
        ) {
            Some(m) => m.clone().sanitized(config.general.read_only),
            None => {
                bail!("'default' mode is missing")
            }
        };

        let layout = match config.layouts.get(
            &config
                .general
                .initial_layout
                .to_owned()
                .unwrap_or_else(|| "default".into()),
        ) {
            Some(l) => l.clone(),
            None => {
                bail!("'default' layout is missing")
            }
        };

        let pid = std::process::id();
        let mut session_path = dirs::runtime_dir()
            .unwrap_or_else(env::temp_dir)
            .join("xplr")
            .join("session")
            .join(&pid.to_string())
            .to_string_lossy()
            .to_string();

        if fs::create_dir_all(&session_path).is_err() {
            session_path = env::temp_dir()
                .join("xplr")
                .join("session")
                .join(&pid.to_string())
                .to_string_lossy()
                .to_string();
            fs::create_dir_all(&session_path)?;
        }

        let mut explorer_config = ExplorerConfig::default();
        if !config.general.show_hidden {
            explorer_config.filters.replace(NodeFilterApplicable::new(
                NodeFilter::RelativePathDoesNotStartWith,
                ".".into(),
            ));
        }

        if let Some(sorters) = &config.general.initial_sorting {
            explorer_config.sorters = sorters.clone();
        };

        env::set_current_dir(&pwd)?;
        let pwd = pwd.to_string_lossy().to_string();

        let mut app = Self {
            version: VERSION.to_string(),
            config,
            pwd,
            directory_buffer: Default::default(),
            last_focus: Default::default(),
            selection: Default::default(),
            msg_out: Default::default(),
            mode,
            layout,
            input: Default::default(),
            pid,
            session_path: session_path.clone(),
            pipe: Pipe::from_session_path(&session_path)?,
            explorer_config,
            logs: Default::default(),
            logs_hidden: Default::default(),
            history: Default::default(),
            last_modes: Default::default(),
        };

        let has_errs = !load_errs.is_empty();
        for err in load_errs {
            app = app.log_error(err)?
        }

        if has_errs && !app.config.general.disable_debug_error_mode {
            app = app.switch_mode_builtin("debug_error")?;
        }

        Ok(app)
    }

    pub fn focused_node(&self) -> Option<&Node> {
        self.directory_buffer
            .as_ref()
            .and_then(|d| d.focused_node())
    }

    pub fn focused_node_str(&self) -> String {
        self.focused_node()
            .map(|n| n.absolute_path.clone())
            .unwrap_or_default()
    }

    fn enqueue(mut self, task: Task) -> Self {
        self.msg_out.push_back(MsgOut::Enque(task));
        self
    }

    pub fn handle_batch_external_msgs(
        mut self,
        msgs: Vec<ExternalMsg>,
    ) -> Result<Self> {
        for task in msgs
            .into_iter()
            .map(|msg| Task::new(MsgIn::External(msg), None))
        {
            self = match task.msg {
                MsgIn::Internal(msg) => self.handle_internal(msg)?,
                MsgIn::External(msg) => self.handle_external(msg, task.key)?,
            };
        }
        self.refresh()
    }

    pub fn handle_task(self, task: Task) -> Result<Self> {
        let app = match task.msg {
            MsgIn::Internal(msg) => self.handle_internal(msg)?,
            MsgIn::External(msg) => self.handle_external(msg, task.key)?,
        };
        app.refresh()
    }

    fn handle_internal(self, msg: InternalMsg) -> Result<Self> {
        match msg {
            InternalMsg::SetDirectory(dir) => self.set_directory(dir),
            InternalMsg::AddLastFocus(parent, focus_path) => {
                self.add_last_focus(parent, focus_path)
            }
            InternalMsg::HandleKey(key) => self.handle_key(key),
        }
    }

    fn handle_external(
        self,
        msg: ExternalMsg,
        key: Option<Key>,
    ) -> Result<Self> {
        if self.config.general.read_only && !msg.is_read_only() {
            self.log_error("Cannot execute code in read-only mode.".into())
        } else {
            match msg {
                ExternalMsg::ExplorePwd => self.explore_pwd(),
                ExternalMsg::ExploreParentsAsync => {
                    self.explore_parents_async()
                }
                ExternalMsg::ExplorePwdAsync => self.explore_pwd_async(),
                ExternalMsg::Refresh => self.refresh(),
                ExternalMsg::ClearScreen => self.clear_screen(),
                ExternalMsg::FocusFirst => self.focus_first(true),
                ExternalMsg::FocusLast => self.focus_last(),
                ExternalMsg::FocusPrevious => self.focus_previous(),
                ExternalMsg::FocusPreviousByRelativeIndex(i) => {
                    self.focus_previous_by_relative_index(i)
                }

                ExternalMsg::FocusPreviousByRelativeIndexFromInput => {
                    self.focus_previous_by_relative_index_from_input()
                }
                ExternalMsg::FocusNext => self.focus_next(),
                ExternalMsg::FocusNextByRelativeIndex(i) => {
                    self.focus_next_by_relative_index(i)
                }
                ExternalMsg::FocusNextByRelativeIndexFromInput => {
                    self.focus_next_by_relative_index_from_input()
                }
                ExternalMsg::FocusPath(p) => self.focus_path(&p, true),
                ExternalMsg::FocusPathFromInput => self.focus_path_from_input(),
                ExternalMsg::FocusByIndex(i) => self.focus_by_index(i),
                ExternalMsg::FocusByIndexFromInput => {
                    self.focus_by_index_from_input()
                }
                ExternalMsg::FocusByFileName(n) => {
                    self.focus_by_file_name(&n, true)
                }
                ExternalMsg::ChangeDirectory(dir) => {
                    self.change_directory(&dir, true)
                }
                ExternalMsg::Enter => self.enter(),
                ExternalMsg::Back => self.back(),
                ExternalMsg::LastVisitedPath => self.last_visited_path(),
                ExternalMsg::NextVisitedPath => self.next_visited_path(),
                ExternalMsg::FollowSymlink => self.follow_symlink(),
                ExternalMsg::UpdateInputBuffer(op) => {
                    self.update_input_buffer(op)
                }
                ExternalMsg::UpdateInputBufferFromKey => {
                    self.update_input_buffer_from_key(key)
                }
                ExternalMsg::BufferInput(input) => self.buffer_input(&input),
                ExternalMsg::BufferInputFromKey => {
                    self.buffer_input_from_key(key)
                }
                ExternalMsg::SetInputBuffer(input) => {
                    self.set_input_buffer(input)
                }
                ExternalMsg::RemoveInputBufferLastCharacter => {
                    self.remove_input_buffer_last_character()
                }
                ExternalMsg::RemoveInputBufferLastWord => {
                    self.remove_input_buffer_last_word()
                }
                ExternalMsg::ResetInputBuffer => self.reset_input_buffer(),
                ExternalMsg::SwitchMode(mode) => self.switch_mode(&mode),
                ExternalMsg::SwitchModeKeepingInputBuffer(mode) => {
                    self.switch_mode_keeping_input_buffer(&mode)
                }
                ExternalMsg::SwitchModeBuiltin(mode) => {
                    self.switch_mode_builtin(&mode)
                }
                ExternalMsg::SwitchModeBuiltinKeepingInputBuffer(mode) => {
                    self.switch_mode_builtin_keeping_input_buffer(&mode)
                }
                ExternalMsg::SwitchModeCustom(mode) => {
                    self.switch_mode_custom(&mode)
                }
                ExternalMsg::SwitchModeCustomKeepingInputBuffer(mode) => {
                    self.switch_mode_custom_keeping_input_buffer(&mode)
                }
                ExternalMsg::PopMode => self.pop_mode(),
                ExternalMsg::PopModeKeepingInputBuffer => {
                    self.pop_mode_keeping_input_buffer()
                }
                ExternalMsg::SwitchLayout(mode) => self.switch_layout(&mode),
                ExternalMsg::SwitchLayoutBuiltin(mode) => {
                    self.switch_layout_builtin(&mode)
                }
                ExternalMsg::SwitchLayoutCustom(mode) => {
                    self.switch_layout_custom(&mode)
                }
                ExternalMsg::Call(cmd) => self.call(cmd),
                ExternalMsg::CallSilently(cmd) => self.call_silently(cmd),
                ExternalMsg::BashExec(cmd) => self.bash_exec(cmd),
                ExternalMsg::BashExecSilently(cmd) => {
                    self.bash_exec_silently(cmd)
                }
                ExternalMsg::CallLua(func) => self.call_lua(func),
                ExternalMsg::CallLuaSilently(func) => {
                    self.call_lua_silently(func)
                }
                ExternalMsg::LuaEval(code) => self.lua_eval(code),
                ExternalMsg::LuaEvalSilently(code) => {
                    self.lua_eval_silently(code)
                }
                ExternalMsg::Select => self.select(),
                ExternalMsg::SelectAll => self.select_all(),
                ExternalMsg::SelectPath(p) => self.select_path(p),
                ExternalMsg::UnSelect => self.un_select(),
                ExternalMsg::UnSelectAll => self.un_select_all(),
                ExternalMsg::UnSelectPath(p) => self.un_select_path(p),
                ExternalMsg::ToggleSelection => self.toggle_selection(),
                ExternalMsg::ToggleSelectAll => self.toggle_select_all(),
                ExternalMsg::ToggleSelectionByPath(p) => {
                    self.toggle_selection_by_path(p)
                }
                ExternalMsg::ClearSelection => self.clear_selection(),
                ExternalMsg::AddNodeFilter(f) => self.add_node_filter(f),
                ExternalMsg::AddNodeFilterFromInput(f) => {
                    self.add_node_filter_from_input(f)
                }
                ExternalMsg::RemoveNodeFilter(f) => self.remove_node_filter(f),
                ExternalMsg::RemoveNodeFilterFromInput(f) => {
                    self.remove_node_filter_from_input(f)
                }
                ExternalMsg::ToggleNodeFilter(f) => self.toggle_node_filter(f),
                ExternalMsg::RemoveLastNodeFilter => {
                    self.remove_last_node_filter()
                }
                ExternalMsg::ResetNodeFilters => self.reset_node_filters(),
                ExternalMsg::ClearNodeFilters => self.clear_node_filters(),
                ExternalMsg::AddNodeSorter(f) => self.add_node_sorter(f),
                ExternalMsg::RemoveNodeSorter(f) => self.remove_node_sorter(f),
                ExternalMsg::ReverseNodeSorter(f) => {
                    self.reverse_node_sorter(f)
                }
                ExternalMsg::ToggleNodeSorter(f) => self.toggle_node_sorter(f),
                ExternalMsg::RemoveLastNodeSorter => {
                    self.remove_last_node_sorter()
                }
                ExternalMsg::ReverseNodeSorters => self.reverse_node_sorters(),
                ExternalMsg::ResetNodeSorters => self.reset_node_sorters(),
                ExternalMsg::ClearNodeSorters => self.clear_node_sorters(),
                ExternalMsg::EnableMouse => self.enable_mouse(),
                ExternalMsg::DisableMouse => self.disable_mouse(),
                ExternalMsg::ToggleMouse => self.toggle_mouse(),
                ExternalMsg::StartFifo(f) => self.start_fifo(f),
                ExternalMsg::StopFifo => self.stop_fifo(),
                ExternalMsg::ToggleFifo(f) => self.toggle_fifo(f),
                ExternalMsg::LogInfo(l) => self.log_info(l),
                ExternalMsg::LogSuccess(l) => self.log_success(l),
                ExternalMsg::LogWarning(l) => self.log_warning(l),
                ExternalMsg::LogError(l) => self.log_error(l),
                ExternalMsg::Quit => self.quit(),
                ExternalMsg::PrintPwdAndQuit => self.print_pwd_and_quit(),
                ExternalMsg::PrintFocusPathAndQuit => {
                    self.print_focus_path_and_quit()
                }
                ExternalMsg::PrintSelectionAndQuit => {
                    self.print_selection_and_quit()
                }
                ExternalMsg::PrintResultAndQuit => self.print_result_and_quit(),
                ExternalMsg::PrintAppStateAndQuit => {
                    self.print_app_state_and_quit()
                }
                ExternalMsg::Debug(path) => self.debug(path),
                ExternalMsg::Terminate => bail!(""),
            }
        }?
        .refresh_selection()
    }

    fn handle_key(mut self, key: Key) -> Result<Self> {
        let kb = self.mode.key_bindings.clone();
        let key_str = key.to_string();
        let msgs = kb
            .on_key
            .get(&key_str)
            .map(|a| a.messages.clone())
            .or_else(|| {
                if key.is_alphabet() {
                    kb.on_alphabet.as_ref().map(|a| a.messages.clone())
                } else if key.is_number() {
                    kb.on_number.as_ref().map(|a| a.messages.clone())
                } else if key.is_special_character() {
                    kb.on_special_character.as_ref().map(|a| a.messages.clone())
                } else if key.is_navigation() {
                    kb.on_navigation.as_ref().map(|a| a.messages.clone())
                } else if key.is_function() {
                    kb.on_function.as_ref().map(|a| a.messages.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                if key.is_alphanumeric() {
                    kb.on_alphanumeric.as_ref().map(|a| a.messages.clone())
                } else {
                    None
                }
            })
            .or_else(|| {
                if key.is_character() {
                    kb.on_character.as_ref().map(|a| a.messages.clone())
                } else {
                    None
                }
            })
            .or_else(|| kb.default.as_ref().map(|a| a.messages.clone()))
            .unwrap_or_else(|| {
                if self.config.general.enable_recover_mode {
                    vec![ExternalMsg::SwitchModeBuiltin("recover".into())]
                } else {
                    vec![ExternalMsg::LogWarning("Key map not found.".into())]
                }
            });

        for msg in msgs {
            self = self.enqueue(Task::new(MsgIn::External(msg), Some(key)));
        }

        Ok(self)
    }

    pub fn explore_pwd(mut self) -> Result<Self> {
        let focus = &self.last_focus.get(&self.pwd).cloned().unwrap_or(None);
        let pwd = self.pwd.clone();
        self = self.add_last_focus(pwd, focus.clone())?;
        let dir = explorer::explore_sync(
            self.explorer_config.clone(),
            self.pwd.clone().into(),
            focus.as_ref().map(PathBuf::from),
            self.directory_buffer.as_ref().map(|d| d.focus).unwrap_or(0),
        )?;
        self.set_directory(dir)
    }

    fn explore_pwd_async(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::ExplorePwdAsync);
        Ok(self)
    }

    fn explore_parents_async(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::ExploreParentsAsync);
        Ok(self)
    }

    fn refresh(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::Refresh);
        Ok(self)
    }

    fn clear_screen(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::ClearScreen);
        Ok(self)
    }

    pub fn focus_first(mut self, save_history: bool) -> Result<Self> {
        let mut history = self.history.clone();
        if let Some(dir) = self.directory_buffer_mut() {
            if save_history {
                if let Some(n) = dir.focused_node() {
                    history = history.push(n.absolute_path.clone());
                }
            }

            dir.focus = 0;

            if save_history {
                if let Some(n) = self.clone().focused_node() {
                    self.history = history.push(n.absolute_path.clone())
                }
            }
        };
        Ok(self)
    }

    fn focus_last(mut self) -> Result<Self> {
        let mut history = self.history.clone();
        if let Some(dir) = self.directory_buffer_mut() {
            if let Some(n) = dir.focused_node() {
                history = history.push(n.absolute_path.clone());
            }

            dir.focus = dir.total.max(1) - 1;

            if let Some(n) = dir.focused_node() {
                self.history = history.push(n.absolute_path.clone());
            }
        };
        Ok(self)
    }

    fn focus_previous(mut self) -> Result<Self> {
        let bounded = self.config.general.enforce_bounded_index_navigation;

        if let Some(dir) = self.directory_buffer_mut() {
            dir.focus = if dir.focus == 0 {
                if bounded {
                    dir.focus
                } else {
                    dir.total.max(1) - 1
                }
            } else {
                dir.focus.max(1) - 1
            };
        };
        Ok(self)
    }

    fn focus_previous_by_relative_index(
        mut self,
        index: usize,
    ) -> Result<Self> {
        let mut history = self.history.clone();
        if let Some(dir) = self.directory_buffer_mut() {
            if let Some(n) = dir.focused_node() {
                history = history.push(n.absolute_path.clone());
            }

            dir.focus = dir.focus.max(index) - index;
            if let Some(n) = self.focused_node() {
                self.history = history.push(n.absolute_path.clone());
            }
        };
        Ok(self)
    }

    fn focus_previous_by_relative_index_from_input(self) -> Result<Self> {
        if let Some(index) = self
            .input
            .as_ref()
            .and_then(|i| i.value().parse::<usize>().ok())
        {
            self.focus_previous_by_relative_index(index)
        } else {
            Ok(self)
        }
    }

    fn focus_next(mut self) -> Result<Self> {
        let bounded = self.config.general.enforce_bounded_index_navigation;

        if let Some(dir) = self.directory_buffer_mut() {
            dir.focus = if (dir.focus + 1) == dir.total {
                if bounded {
                    dir.focus
                } else {
                    0
                }
            } else {
                dir.focus + 1
            }
        };
        Ok(self)
    }

    fn focus_next_by_relative_index(mut self, index: usize) -> Result<Self> {
        let mut history = self.history.clone();
        if let Some(dir) = self.directory_buffer_mut() {
            if let Some(n) = dir.focused_node() {
                history = history.push(n.absolute_path.clone());
            }

            dir.focus = (dir.focus + index).min(dir.total.max(1) - 1);
            if let Some(n) = self.focused_node() {
                self.history = history.push(n.absolute_path.clone());
            }
        };
        Ok(self)
    }

    fn focus_next_by_relative_index_from_input(self) -> Result<Self> {
        if let Some(index) = self
            .input
            .as_ref()
            .and_then(|i| i.value().parse::<usize>().ok())
        {
            self.focus_next_by_relative_index(index)
        } else {
            Ok(self)
        }
    }

    fn follow_symlink(self) -> Result<Self> {
        if let Some(pth) = self
            .focused_node()
            .and_then(|n| n.symlink.to_owned().map(|s| s.absolute_path))
        {
            self.focus_path(&pth, true)
        } else {
            Ok(self)
        }
    }

    fn change_directory(
        mut self,
        dir: &str,
        save_history: bool,
    ) -> Result<Self> {
        let mut dir = PathBuf::from(dir);
        if dir.is_relative() {
            dir = PathBuf::from(self.pwd.clone()).join(dir);
        }

        match env::set_current_dir(&dir) {
            Ok(()) => {
                let pwd = self.pwd.clone();
                let focus =
                    self.focused_node().map(|n| n.relative_path.clone());
                self = self.add_last_focus(pwd, focus)?;
                self.pwd = dir.to_string_lossy().to_string();
                if save_history {
                    self.history = self.history.push(format!("{}/", self.pwd));
                }
                self.explore_pwd()
            }
            Err(e) => self.log_error(e.to_string()),
        }
    }

    fn enter(self) -> Result<Self> {
        self.focused_node()
            .map(|n| n.absolute_path.clone())
            .map(|p| self.clone().change_directory(&p, true))
            .unwrap_or(Ok(self))
    }

    fn back(self) -> Result<Self> {
        PathBuf::from(self.pwd.clone())
            .parent()
            .map(|p| {
                self.clone()
                    .change_directory(&p.to_string_lossy().to_string(), true)
            })
            .unwrap_or(Ok(self))
    }

    fn last_visited_path(mut self) -> Result<Self> {
        self.history = self.history.visit_last();
        if let Some(target) = self.history.peek() {
            if target.ends_with('/') {
                target
                    .strip_suffix('/')
                    .map(|s| self.clone().change_directory(s, false))
                    .unwrap_or(Ok(self))
            } else {
                self.clone().focus_path(target, false)
            }
        } else {
            Ok(self)
        }
    }

    fn next_visited_path(mut self) -> Result<Self> {
        self.history = self.history.visit_next();
        if let Some(target) = self.history.peek() {
            if target.ends_with('/') {
                target
                    .strip_suffix('/')
                    .map(|s| self.clone().change_directory(s, false))
                    .unwrap_or(Ok(self))
            } else {
                self.clone().focus_path(target, false)
            }
        } else {
            Ok(self)
        }
    }

    fn update_input_buffer(mut self, op: InputOperation) -> Result<Self> {
        if let Some(buf) = self.input.as_mut() {
            buf.handle(op.into());
            self.logs_hidden = true;
        } else {
            let mut buf = Input::default();
            if buf.handle(op.into()).is_some() {
                self.input = Some(buf);
                self.logs_hidden = true;
            }
        }
        Ok(self)
    }

    fn update_input_buffer_from_key(self, key: Option<Key>) -> Result<Self> {
        if let Some(op) = key.and_then(|k| k.to_input_operation()) {
            self.update_input_buffer(op)
        } else {
            Ok(self)
        }
    }

    fn buffer_input(mut self, input: &str) -> Result<Self> {
        if let Some(buf) = self.input.as_mut() {
            buf.handle(InputRequest::GoToEnd);
            for c in input.chars() {
                buf.handle(InputRequest::InsertChar(c));
            }
        } else {
            self.input = Some(Input::default().with_value(input.into()));
        };
        self.logs_hidden = true;
        Ok(self)
    }

    fn buffer_input_from_key(self, key: Option<Key>) -> Result<Self> {
        if let Some(c) = key.and_then(|k| k.to_char()) {
            self.buffer_input(&c.to_string())
        } else {
            Ok(self)
        }
    }

    fn set_input_buffer(mut self, string: String) -> Result<Self> {
        self.input = Some(Input::default().with_value(string));
        self.logs_hidden = true;
        Ok(self)
    }

    fn remove_input_buffer_last_character(mut self) -> Result<Self> {
        if let Some(buf) = self.input.as_mut() {
            buf.handle(InputRequest::GoToEnd);
            buf.handle(InputRequest::DeletePrevChar);
            self.logs_hidden = true;
        };
        Ok(self)
    }

    fn remove_input_buffer_last_word(mut self) -> Result<Self> {
        if let Some(buf) = self.input.as_mut() {
            buf.handle(InputRequest::GoToEnd);
            buf.handle(InputRequest::DeletePrevWord);
            self.logs_hidden = true;
        };
        Ok(self)
    }

    fn reset_input_buffer(mut self) -> Result<Self> {
        self.input = None;
        Ok(self)
    }

    fn focus_by_index(mut self, index: usize) -> Result<Self> {
        let history = self.history.clone();
        if let Some(dir) = self.directory_buffer_mut() {
            dir.focus = index.min(dir.total.max(1) - 1);
            if let Some(n) = self.focused_node() {
                self.history = history.push(n.absolute_path.clone());
            }
        };
        Ok(self)
    }

    fn focus_by_index_from_input(self) -> Result<Self> {
        if let Some(index) = self
            .input
            .as_ref()
            .and_then(|i| i.value().parse::<usize>().ok())
        {
            self.focus_by_index(index)
        } else {
            Ok(self)
        }
    }

    pub fn focus_by_file_name(
        mut self,
        name: &str,
        save_history: bool,
    ) -> Result<Self> {
        let mut history = self.history.clone();
        if let Some(dir_buf) = self.directory_buffer_mut() {
            if let Some(focus) = dir_buf
                .clone()
                .nodes
                .iter()
                .enumerate()
                .find(|(_, n)| n.relative_path == name)
                .map(|(i, _)| i)
            {
                if save_history {
                    if let Some(n) = dir_buf.focused_node() {
                        history = history.push(n.absolute_path.clone());
                    }
                }
                dir_buf.focus = focus;
                if save_history {
                    if let Some(n) = dir_buf.focused_node() {
                        self.history = history.push(n.absolute_path.clone());
                    }
                }
                Ok(self)
            } else {
                self.log_error(format!("{} not found in $PWD", name))
            }
        } else {
            Ok(self)
        }
    }

    pub fn focus_path(self, path: &str, save_history: bool) -> Result<Self> {
        let mut pathbuf = PathBuf::from(path);
        if pathbuf.is_relative() {
            pathbuf = PathBuf::from(self.pwd.clone()).join(pathbuf);
        }
        if let Some(parent) = pathbuf.parent() {
            if let Some(filename) = pathbuf.file_name() {
                self.change_directory(
                    &parent.to_string_lossy().to_string(),
                    false,
                )?
                .focus_by_file_name(
                    &filename.to_string_lossy().to_string(),
                    save_history,
                )
            } else {
                self.log_error(format!("{} not found", path))
            }
        } else {
            self.log_error(format!("Cannot focus on {}", path))
        }
    }

    fn focus_path_from_input(self) -> Result<Self> {
        if let Some(p) = self.input.clone() {
            self.focus_path(p.value(), true)
        } else {
            Ok(self)
        }
    }

    fn push_mode(mut self) -> Self {
        if self.mode != self.config.modes.builtin.recover
            && self
                .last_modes
                .last()
                .map(|m| m != &self.mode)
                .unwrap_or(true)
        {
            self.last_modes.push(self.mode.clone())
        }
        self
    }

    fn pop_mode(self) -> Result<Self> {
        self.pop_mode_keeping_input_buffer().map(|mut a| {
            a.input = None;
            a
        })
    }

    fn pop_mode_keeping_input_buffer(mut self) -> Result<Self> {
        if let Some(mode) = self.last_modes.pop() {
            self.mode = mode;
        };
        Ok(self)
    }

    fn switch_mode(self, mode: &str) -> Result<Self> {
        self.switch_mode_keeping_input_buffer(mode).map(|mut a| {
            a.input = None;
            a
        })
    }

    fn switch_mode_keeping_input_buffer(mut self, mode: &str) -> Result<Self> {
        if let Some(mode) = self.config.modes.get(mode).cloned() {
            self = self.push_mode();
            self.mode = mode.sanitized(self.config.general.read_only);
            Ok(self)
        } else {
            self.log_error(format!("Mode not found: {}", mode))
        }
    }

    fn switch_mode_builtin(self, mode: &str) -> Result<Self> {
        self.switch_mode_builtin_keeping_input_buffer(mode)
            .map(|mut a| {
                a.input = None;
                a
            })
    }

    fn switch_mode_builtin_keeping_input_buffer(
        mut self,
        mode: &str,
    ) -> Result<Self> {
        if let Some(mode) = self.config.modes.builtin.get(mode).cloned() {
            self = self.push_mode();
            self.mode = mode.sanitized(self.config.general.read_only);
            Ok(self)
        } else {
            self.log_error(format!("Builtin mode not found: {}", mode))
        }
    }

    fn switch_mode_custom(self, mode: &str) -> Result<Self> {
        self.switch_mode_custom_keeping_input_buffer(mode)
            .map(|mut a| {
                a.input = None;
                a
            })
    }

    fn switch_mode_custom_keeping_input_buffer(
        mut self,
        mode: &str,
    ) -> Result<Self> {
        if let Some(mode) = self.config.modes.custom.get(mode).cloned() {
            self = self.push_mode();
            self.mode = mode.sanitized(self.config.general.read_only);
            Ok(self)
        } else {
            self.log_error(format!("Custom mode not found: {}", mode))
        }
    }

    fn switch_layout(mut self, layout: &str) -> Result<Self> {
        if let Some(l) = self.config.layouts.get(layout) {
            self.layout = l.to_owned();
            Ok(self)
        } else {
            self.log_error(format!("Layout not found: {}", layout))
        }
    }

    fn switch_layout_builtin(mut self, layout: &str) -> Result<Self> {
        if let Some(l) = self.config.layouts.builtin.get(layout) {
            self.layout = l.to_owned();
            Ok(self)
        } else {
            self.log_error(format!("Builtin layout not found: {}", layout))
        }
    }

    fn switch_layout_custom(mut self, layout: &str) -> Result<Self> {
        if let Some(l) = self.config.layouts.get_custom(layout) {
            self.layout = l.to_owned();
            Ok(self)
        } else {
            self.log_error(format!("Custom layout not found: {}", layout))
        }
    }

    fn call(mut self, command: Command) -> Result<Self> {
        self.logs_hidden = true;
        self.msg_out.push_back(MsgOut::Call(command));
        Ok(self)
    }

    fn call_silently(mut self, command: Command) -> Result<Self> {
        self.logs_hidden = true;
        self.msg_out.push_back(MsgOut::CallSilently(command));
        Ok(self)
    }

    fn bash_exec(self, script: String) -> Result<Self> {
        self.call(Command {
            command: "bash".into(),
            args: vec!["-c".into(), script],
        })
    }

    fn bash_exec_silently(self, script: String) -> Result<Self> {
        self.call_silently(Command {
            command: "bash".into(),
            args: vec!["-c".into(), script],
        })
    }

    fn call_lua(mut self, func: String) -> Result<Self> {
        self.logs_hidden = true;
        self.msg_out.push_back(MsgOut::CallLua(func));
        Ok(self)
    }

    fn call_lua_silently(mut self, func: String) -> Result<Self> {
        self.logs_hidden = true;
        self.msg_out.push_back(MsgOut::CallLuaSilently(func));
        Ok(self)
    }

    fn lua_eval(mut self, code: String) -> Result<Self> {
        self.logs_hidden = true;
        self.msg_out.push_back(MsgOut::LuaEval(code));
        Ok(self)
    }

    fn lua_eval_silently(mut self, code: String) -> Result<Self> {
        self.logs_hidden = true;
        self.msg_out.push_back(MsgOut::LuaEvalSilently(code));
        Ok(self)
    }

    pub fn set_directory(mut self, dir: DirectoryBuffer) -> Result<Self> {
        self = self.add_last_focus(
            dir.parent.clone(),
            dir.focused_node().map(|n| n.relative_path.clone()),
        )?;
        if dir.parent == self.pwd {
            self.directory_buffer = Some(dir);
        }
        Ok(self)
    }

    pub fn add_last_focus(
        mut self,
        parent: String,
        focused_path: Option<String>,
    ) -> Result<Self> {
        self.last_focus.insert(parent, focused_path);
        Ok(self)
    }

    pub fn select(mut self) -> Result<Self> {
        if let Some(n) = self.focused_node().map(|n| n.to_owned()) {
            self.selection.insert(n);
        }
        Ok(self)
    }

    pub fn select_path(mut self, path: String) -> Result<Self> {
        let mut path = PathBuf::from(path);
        if path.is_relative() {
            path = PathBuf::from(self.pwd.clone()).join(path);
        }
        let parent = path.parent().map(|p| p.to_string_lossy().to_string());
        let filename =
            path.file_name().map(|p| p.to_string_lossy().to_string());
        if let (Some(p), Some(n)) = (parent, filename) {
            self.selection.insert(Node::new(p, n));
        }
        Ok(self)
    }

    pub fn select_all(mut self) -> Result<Self> {
        if let Some(d) = self.directory_buffer.as_ref() {
            d.nodes.clone().into_iter().for_each(|n| {
                self.selection.insert(n);
            });
        };

        Ok(self)
    }

    pub fn un_select(mut self) -> Result<Self> {
        if let Some(n) = self.focused_node().map(|n| n.to_owned()) {
            self.selection.retain(|s| s != &n);
        }
        Ok(self)
    }

    pub fn un_select_path(mut self, path: String) -> Result<Self> {
        let mut pathbuf = PathBuf::from(path);
        if pathbuf.is_relative() {
            pathbuf = PathBuf::from(self.pwd.clone()).join(pathbuf);
        }
        self.selection
            .retain(|n| PathBuf::from(&n.absolute_path) != pathbuf);
        Ok(self)
    }

    pub fn un_select_all(mut self) -> Result<Self> {
        if let Some(d) = self.directory_buffer.as_ref() {
            d.nodes.clone().into_iter().for_each(|n| {
                self.selection.retain(|s| s != &n);
            });
        };

        Ok(self)
    }

    fn toggle_selection(self) -> Result<Self> {
        if let Some(p) = self.focused_node().map(|n| n.absolute_path.clone()) {
            self.toggle_selection_by_path(p)
        } else {
            Ok(self)
        }
    }

    fn toggle_select_all(self) -> Result<Self> {
        if let Some(d) = self.directory_buffer.as_ref() {
            if d.nodes.iter().all(|n| self.selection.contains(n)) {
                self.un_select_all()
            } else {
                self.select_all()
            }
        } else {
            Ok(self)
        }
    }

    fn toggle_selection_by_path(self, path: String) -> Result<Self> {
        let mut pathbuf = PathBuf::from(&path);
        if pathbuf.is_relative() {
            pathbuf = PathBuf::from(self.pwd.clone()).join(pathbuf);
        }
        if self
            .selection
            .iter()
            .any(|n| PathBuf::from(&n.absolute_path) == pathbuf)
        {
            self.un_select_path(path)
        } else {
            self.select_path(path)
        }
    }

    fn clear_selection(mut self) -> Result<Self> {
        self.selection.clear();
        Ok(self)
    }

    fn add_node_filter(mut self, filter: NodeFilterApplicable) -> Result<Self> {
        self.explorer_config.filters.replace(filter);
        Ok(self)
    }

    fn add_node_filter_from_input(
        mut self,
        filter: NodeFilter,
    ) -> Result<Self> {
        if let Some(input) = self.input.as_ref() {
            self.explorer_config
                .filters
                .insert(NodeFilterApplicable::new(
                    filter,
                    input.value().into(),
                ));
        };
        Ok(self)
    }

    fn remove_node_filter(
        mut self,
        filter: NodeFilterApplicable,
    ) -> Result<Self> {
        self.explorer_config.filters.retain(|f| f != &filter);
        Ok(self)
    }

    fn remove_node_filter_from_input(
        mut self,
        filter: NodeFilter,
    ) -> Result<Self> {
        if let Some(input) = self.input.as_ref() {
            let nfa = NodeFilterApplicable::new(filter, input.value().into());
            self.explorer_config.filters.retain(|f| f != &nfa);
        };
        Ok(self)
    }

    fn toggle_node_filter(self, filter: NodeFilterApplicable) -> Result<Self> {
        if self.explorer_config.filters.contains(&filter) {
            self.remove_node_filter(filter)
        } else {
            self.add_node_filter(filter)
        }
    }

    fn remove_last_node_filter(mut self) -> Result<Self> {
        self.explorer_config.filters.pop();
        Ok(self)
    }

    fn reset_node_filters(mut self) -> Result<Self> {
        self.explorer_config.filters.clear();

        if !self.config.general.show_hidden {
            self.add_node_filter(NodeFilterApplicable::new(
                NodeFilter::RelativePathDoesNotStartWith,
                ".".into(),
            ))
        } else {
            Ok(self)
        }
    }
    fn clear_node_filters(mut self) -> Result<Self> {
        self.explorer_config.filters.clear();
        Ok(self)
    }

    fn add_node_sorter(mut self, sorter: NodeSorterApplicable) -> Result<Self> {
        self.explorer_config.sorters.replace(sorter);
        Ok(self)
    }

    fn remove_node_sorter(mut self, sorter: NodeSorter) -> Result<Self> {
        self.explorer_config.sorters.retain(|s| s.sorter != sorter);
        Ok(self)
    }

    fn reverse_node_sorter(mut self, sorter: NodeSorter) -> Result<Self> {
        self.explorer_config.sorters = self
            .explorer_config
            .sorters
            .into_iter()
            .map(|s| if s.sorter == sorter { s.reversed() } else { s })
            .collect();
        Ok(self)
    }

    fn toggle_node_sorter(self, sorter: NodeSorterApplicable) -> Result<Self> {
        if self.explorer_config.sorters.contains(&sorter) {
            self.remove_node_sorter(sorter.sorter)
        } else {
            self.add_node_sorter(sorter)
        }
    }

    fn remove_last_node_sorter(mut self) -> Result<Self> {
        self.explorer_config.sorters.pop();
        Ok(self)
    }

    fn reverse_node_sorters(mut self) -> Result<Self> {
        self.explorer_config.sorters = self
            .explorer_config
            .sorters
            .into_iter()
            .map(|s| s.reversed())
            .collect();
        Ok(self)
    }

    fn reset_node_sorters(mut self) -> Result<Self> {
        self.explorer_config.sorters = self
            .config
            .general
            .initial_sorting
            .to_owned()
            .unwrap_or_default();
        Ok(self)
    }

    fn clear_node_sorters(mut self) -> Result<Self> {
        self.explorer_config.sorters.clear();
        Ok(self)
    }

    fn enable_mouse(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::EnableMouse);
        Ok(self)
    }

    fn disable_mouse(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::DisableMouse);
        Ok(self)
    }

    fn toggle_mouse(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::ToggleMouse);
        Ok(self)
    }

    fn start_fifo(mut self, path: String) -> Result<Self> {
        self.msg_out.push_back(MsgOut::StartFifo(path));
        Ok(self)
    }

    fn stop_fifo(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::StopFifo);
        Ok(self)
    }

    fn toggle_fifo(mut self, path: String) -> Result<Self> {
        self.msg_out.push_back(MsgOut::ToggleFifo(path));
        Ok(self)
    }

    pub fn log_info(mut self, message: String) -> Result<Self> {
        self.logs_hidden = false;
        self.logs.push(Log::new(LogLevel::Info, message));
        Ok(self)
    }

    pub fn log_success(mut self, message: String) -> Result<Self> {
        self.logs_hidden = false;
        self.logs.push(Log::new(LogLevel::Success, message));
        Ok(self)
    }

    pub fn log_warning(mut self, message: String) -> Result<Self> {
        self.logs_hidden = false;
        self.logs.push(Log::new(LogLevel::Warning, message));
        Ok(self)
    }

    pub fn log_error(mut self, message: String) -> Result<Self> {
        self.logs_hidden = false;
        self.logs.push(Log::new(LogLevel::Error, message));
        Ok(self)
    }

    fn quit(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::Quit);
        Ok(self)
    }

    fn print_pwd_and_quit(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::PrintPwdAndQuit);
        Ok(self)
    }

    fn print_focus_path_and_quit(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::PrintFocusPathAndQuit);
        Ok(self)
    }

    fn print_selection_and_quit(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::PrintSelectionAndQuit);
        Ok(self)
    }

    fn print_result_and_quit(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::PrintResultAndQuit);
        Ok(self)
    }

    fn print_app_state_and_quit(mut self) -> Result<Self> {
        self.msg_out.push_back(MsgOut::PrintAppStateAndQuit);
        Ok(self)
    }

    fn debug(mut self, path: String) -> Result<Self> {
        self.msg_out.push_back(MsgOut::Debug(path));
        Ok(self)
    }

    fn directory_buffer_mut(&mut self) -> Option<&mut DirectoryBuffer> {
        self.directory_buffer.as_mut()
    }

    pub fn mode_str(&self) -> String {
        format!("{}\n", &self.mode.name)
    }

    fn refresh_selection(mut self) -> Result<Self> {
        // Should be able to select broken symlink
        self.selection.retain(|n| {
            PathBuf::from(&n.absolute_path).symlink_metadata().is_ok()
        });
        Ok(self)
    }

    pub fn result(&self) -> Vec<&Node> {
        if self.selection.is_empty() {
            self.focused_node().map(|n| vec![n]).unwrap_or_default()
        } else {
            self.selection.iter().collect()
        }
    }

    pub fn directory_nodes_str(&self) -> String {
        self.directory_buffer
            .as_ref()
            .map(|d| {
                d.nodes
                    .iter()
                    .map(|n| format!("{}\n", n.absolute_path))
                    .collect::<Vec<String>>()
                    .join("")
            })
            .unwrap_or_default()
    }

    pub fn pwd_str(&self) -> String {
        format!("{}\n", &self.pwd)
    }

    pub fn selection_str(&self) -> String {
        self.selection
            .iter()
            .map(|n| format!("{}\n", n.absolute_path))
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn result_str(&self) -> String {
        self.result()
            .into_iter()
            .map(|n| format!("{}\n", n.absolute_path))
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn logs_str(&self) -> String {
        self.logs
            .iter()
            .map(|l| format!("{}\n", l))
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn global_help_menu_str(&self) -> String {
        let builtin = &self.config.modes.builtin;
        let custom = &self.config.modes.custom;

        [
            &builtin.default,
            &builtin.debug_error,
            &builtin.recover,
            &builtin.filter,
            &builtin.number,
            &builtin.go_to,
            &builtin.search,
            &builtin.selection_ops,
            &builtin.action,
            &builtin.create,
            &builtin.create_file,
            &builtin.create_directory,
            &builtin.rename,
            &builtin.duplicate_as,
            &builtin.delete,
            &builtin.sort,
            &builtin.filter,
            &builtin.relative_path_does_contain,
            &builtin.relative_path_does_not_contain,
            &builtin.switch_layout,
        ]
        .iter().map(|m| (&m.name, m.to_owned()))
        .chain(custom.iter())
        .map(|(name, mode)| {
            let help = mode
                .help_menu()
                .iter()
                .map(|l| match l {
                    HelpMenuLine::Paragraph(p) => format!("\t{}\n", p),
                    HelpMenuLine::KeyMap(k, remaps, h) => {
                        let remaps = remaps.join(", ");
                        format!(" {:15} | {:25} | {}\n", k, remaps, h)
                    }
                })
                .collect::<Vec<String>>()
                .join("");

            format!(
                "### {}\n\n key             | remaps                    | action\n --------------- | ------------------------- | ------\n{}\n",
                name, help
            )
        })
        .collect::<Vec<String>>()
        .join("\n")
    }

    pub fn history_str(&self) -> String {
        self.history
            .paths
            .iter()
            .map(|p| format!("{}\n", &p))
            .collect::<Vec<String>>()
            .join("")
    }

    pub fn write_pipes(&self) -> Result<()> {
        fs::create_dir_all(self.pipe.path.clone())?;
        fs::write(&self.pipe.msg_in, "")?;

        let selection_str = self.selection_str();
        fs::write(&self.pipe.selection_out, selection_str)?;

        let history_str = self.history_str();
        fs::write(&self.pipe.history_out, history_str)?;

        let directory_nodes_str = self.directory_nodes_str();
        fs::write(&self.pipe.directory_nodes_out, directory_nodes_str)?;

        let logs_str = self.logs_str();
        fs::write(&self.pipe.logs_out, logs_str)?;

        let result_str = self.result_str();
        fs::write(&self.pipe.result_out, result_str)?;

        let global_help_menu_str = self.global_help_menu_str();
        fs::write(&self.pipe.global_help_menu_out, global_help_menu_str)?;

        Ok(())
    }

    pub fn cleanup_pipes(&self) -> Result<()> {
        while !fs::read_to_string(&self.pipe.msg_in)?.is_empty() {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }

        fs::remove_file(&self.pipe.msg_in)?;
        fs::remove_file(&self.pipe.selection_out)?;
        fs::remove_file(&self.pipe.result_out)?;
        fs::remove_file(&self.pipe.directory_nodes_out)?;
        fs::remove_file(&self.pipe.global_help_menu_out)?;
        fs::remove_file(&self.pipe.logs_out)?;
        fs::remove_file(&self.pipe.history_out)?;

        fs::remove_dir(&self.pipe.path)?;
        Ok(())
    }

    pub fn to_lua_ctx_heavy(&self) -> LuaContextHeavy {
        LuaContextHeavy {
            version: self.version.clone(),
            pwd: self.pwd.clone(),
            focused_node: self.focused_node().cloned(),
            directory_buffer: self.directory_buffer.clone(),
            selection: self.selection.clone(),
            mode: self.mode.clone(),
            layout: self.layout.clone(),
            input_buffer: self.input.as_ref().map(|i| i.value().into()),
            pid: self.pid,
            session_path: self.session_path.clone(),
            explorer_config: self.explorer_config.clone(),
            history: self.history.clone(),
            last_modes: self.last_modes.clone(),
        }
    }

    pub fn to_lua_ctx_light(&self) -> LuaContextLight {
        LuaContextLight {
            version: self.version.clone(),
            pwd: self.pwd.clone(),
            focused_node: self.focused_node().cloned(),
            selection: self.selection.clone(),
            mode: self.mode.clone(),
            layout: self.layout.clone(),
            input_buffer: self.input.as_ref().map(|i| i.value().into()),
            pid: self.pid,
            session_path: self.session_path.clone(),
            explorer_config: self.explorer_config.clone(),
        }
    }
}
